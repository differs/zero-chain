//! JSON-RPC Server Implementation

use axum::{
    extract::DefaultBodyLimit, extract::State, http::HeaderMap, routing::post, Json, Router,
};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use prometheus::{Encoder, IntCounterVec, IntGauge, Registry, TextEncoder};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::oneshot;
use tower_http::cors::{Any, CorsLayer};
use zerocore::account::{Account, AccountState, InMemoryAccountManager, U256};
use zerocore::block::{create_genesis_block, Block, BlockHeader};
use zerocore::compute::domain::DomainRegistry;
use zerocore::compute::{
    BasicTxExecutor, Command, ComputeTx, DefaultAuthorizationPolicy, DomainConfig, DomainId,
    InMemoryDomainRegistry, InMemoryObjectStore, NoopResourcePolicy, ObjectId, ObjectKind,
    ObjectOutput, ObjectStore, OutputId, OutputProposal, Ownership, ResourceMap, ResourceValue,
    Script, SignatureScheme, TxSignature, TxWitness, Version,
};
use zerocore::crypto::{Address, Hash};
use zerocore::crypto::{PrivateKey, Signature};
use zerocore::state::StateDb;
use zerocore::transaction::{pool::TxPoolConfig, SignedTransaction, TransactionPool};
use zeronet::{
    global_block_by_number, global_latest_block, global_peer_count, global_peers,
    global_store_block, global_synced_height, set_global_synced_height,
};
use zerostore::db::{KeyValueDB, MemDatabase, RedbDatabase, RocksDb};
use zerostore::ComputeStore;

static RPC_METRICS: Lazy<RpcMetrics> = Lazy::new(RpcMetrics::new);
const MAX_MINING_JOBS: usize = 2_048;
const MAX_MINING_JOB_AGE_SECS: u64 = 300;
const MAX_MINER_EXTRA_DATA_BYTES: usize = 64;
const MAX_SEEN_MINING_SUBMISSIONS: usize = 16_384;
const MAX_BLOCK_HISTORY: usize = 50_000;
const MAX_SUBMITTED_COMPUTE_RESULTS: usize = 50_000;
const MAX_TRANSFER_HISTORY: usize = 50_000;
const TARGET_BLOCK_INTERVAL_SECS: u64 = 10;
const MIN_MINING_DIFFICULTY: u128 = 250_000;
const BASE_MINING_DIFFICULTY: u128 = 1_000_000;
const MAX_MINING_DIFFICULTY: u128 = 16_000_000;

struct RpcMetrics {
    registry: Registry,
    method_calls: IntCounterVec,
    method_errors: IntCounterVec,
    mining_shares_accepted: IntCounterVec,
    mining_shares_rejected: IntCounterVec,
    latest_block_height: IntGauge,
}

impl RpcMetrics {
    fn new() -> Self {
        let registry = Registry::new();
        let method_calls = IntCounterVec::new(
            prometheus::Opts::new("zero_rpc_method_calls_total", "RPC method call count"),
            &["method"],
        )
        .expect("method_calls metric");
        let method_errors = IntCounterVec::new(
            prometheus::Opts::new("zero_rpc_method_errors_total", "RPC method error count"),
            &["method"],
        )
        .expect("method_errors metric");
        let mining_shares_accepted = IntCounterVec::new(
            prometheus::Opts::new(
                "zero_mining_shares_accepted_total",
                "Accepted mining shares",
            ),
            &["source"],
        )
        .expect("mining_shares_accepted metric");
        let mining_shares_rejected = IntCounterVec::new(
            prometheus::Opts::new(
                "zero_mining_shares_rejected_total",
                "Rejected mining shares",
            ),
            &["reason"],
        )
        .expect("mining_shares_rejected metric");
        let latest_block_height = IntGauge::new(
            "zero_latest_block_height",
            "Latest block height observed by RPC",
        )
        .expect("latest_block_height metric");

        registry
            .register(Box::new(method_calls.clone()))
            .expect("register method_calls");
        registry
            .register(Box::new(method_errors.clone()))
            .expect("register method_errors");
        registry
            .register(Box::new(mining_shares_accepted.clone()))
            .expect("register mining_shares_accepted");
        registry
            .register(Box::new(mining_shares_rejected.clone()))
            .expect("register mining_shares_rejected");
        registry
            .register(Box::new(latest_block_height.clone()))
            .expect("register latest_block_height");

        Self {
            registry,
            method_calls,
            method_errors,
            mining_shares_accepted,
            mining_shares_rejected,
            latest_block_height,
        }
    }

    fn render(&self) -> Result<String, RpcErrorObject> {
        let families = self.registry.gather();
        let mut out = Vec::new();
        TextEncoder::new()
            .encode(&families, &mut out)
            .map_err(|e| RpcErrorObject::internal_error(format!("encode metrics failed: {e}")))?;
        String::from_utf8(out)
            .map_err(|e| RpcErrorObject::internal_error(format!("metrics utf8 failed: {e}")))
    }
}

/// RPC configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RpcConfig {
    /// Listen address
    pub address: String,
    /// Port
    pub port: u16,
    /// Max connections
    pub max_connections: u32,
    /// Max request body size
    pub max_request_size: usize,
    /// Enabled modules
    pub modules: Vec<String>,
    /// Compute persistent backend kind.
    pub compute_backend: ComputeBackend,
    /// Database path for file-based backends (rocksdb/redb)
    pub compute_db_path: String,
    /// Chain identifier used by native transaction context.
    pub chain_id: u64,
    /// Network id returned by net_version.
    pub network_id: u64,
    /// Mining reward address.
    pub coinbase: String,
    /// Optional static auth token for all JSON-RPC requests.
    pub auth_token: Option<String>,
    /// Per-client request budget per rolling minute. `0` means disabled.
    pub rate_limit_per_minute: u32,
}

/// Persistent backend for compute storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputeBackend {
    /// In-memory backend.
    #[default]
    Mem,
    /// RocksDB backend.
    RocksDb,
    /// Redb backend.
    Redb,
}

impl ComputeBackend {
    /// Returns a stable string representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mem => "mem",
            Self::RocksDb => "rocksdb",
            Self::Redb => "redb",
        }
    }
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".to_string(),
            port: 8545,
            max_connections: 100,
            max_request_size: 15 * 1024 * 1024, // 15MB
            modules: vec!["zero".into(), "net".into(), "web3".into()],
            compute_backend: ComputeBackend::Mem,
            compute_db_path: "./data/compute-db".to_string(),
            chain_id: 10086,
            network_id: 10086,
            coinbase: "ZER0x0000000000000000000000000000000000000000".to_string(),
            auth_token: None,
            rate_limit_per_minute: 600,
        }
    }
}

impl RpcConfig {
    /// Validates RPC configuration consistency.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.chain_id == 0 {
            return Err("chain_id must be non-zero".to_string());
        }
        if self.network_id == 0 {
            return Err("network_id must be non-zero".to_string());
        }
        if parse_address(&self.coinbase).is_err() {
            return Err("coinbase must be a valid 20-byte address with ZER0x prefix".to_string());
        }
        if let Some(token) = &self.auth_token {
            if token.trim().is_empty() {
                return Err("auth_token cannot be empty".to_string());
            }
        }
        match self.compute_backend {
            ComputeBackend::Mem => Ok(()),
            ComputeBackend::RocksDb | ComputeBackend::Redb => {
                if self.compute_db_path.trim().is_empty() {
                    return Err(format!(
                        "compute_db_path cannot be empty when compute_backend={}",
                        self.compute_backend.as_str()
                    ));
                }
                Ok(())
            }
        }
    }
}

/// JSON-RPC request
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Vec<serde_json::Value>>,
    pub id: serde_json::Value,
}

/// JSON-RPC response
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcErrorObject>,
    pub id: serde_json::Value,
}

/// RPC error object
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl RpcErrorObject {
    pub fn parse_error() -> Self {
        Self {
            code: -32700,
            message: "Parse error".into(),
            data: None,
        }
    }

    pub fn invalid_request() -> Self {
        Self {
            code: -32600,
            message: "Invalid Request".into(),
            data: None,
        }
    }

    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("Method not found: {}", method),
            data: None,
        }
    }

    pub fn invalid_params(message: String) -> Self {
        Self {
            code: -32602,
            message: "Invalid params".into(),
            data: Some(serde_json::Value::String(message)),
        }
    }

    pub fn internal_error(message: String) -> Self {
        Self {
            code: -32603,
            message: "Internal error".into(),
            data: Some(serde_json::Value::String(message)),
        }
    }
}

impl std::fmt::Display for RpcErrorObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RpcErrorObject {}

/// RPC API handler
pub struct RpcApi {
    config: RpcConfig,
    state_db: Arc<StateDb>,
    tx_pool: Arc<TransactionPool>,
    latest_block: RwLock<Option<Block>>,
    block_history: RwLock<BTreeMap<u64, Block>>,
    compute_store: Arc<dyn ObjectStore>,
    domain_registry: Arc<InMemoryDomainRegistry>,
    submitted_compute_results: RwLock<HashMap<Hash, serde_json::Value>>,
    submitted_compute_order: RwLock<VecDeque<Hash>>,
    transfer_txs: RwLock<HashMap<Hash, TransferTxRecord>>,
    transfer_tx_order: RwLock<VecDeque<Hash>>,
    persistent_compute_store: Option<Arc<ComputeStore>>,
    mining_jobs: RwLock<HashMap<String, MiningWork>>,
    mining_job_order: RwLock<VecDeque<String>>,
    mining_seen_submissions: RwLock<HashSet<SeenShareKey>>,
    mining_seen_submission_order: RwLock<VecDeque<SeenShareKey>>,
    hashrate_counter: RwLock<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MiningWork {
    work_id: String,
    prev_hash: Hash,
    height: u64,
    target_leading_zero_bytes: usize,
    created_at_secs: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SeenShareKey {
    work_id: String,
    nonce: u64,
    hash: [u8; 32],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SubmitWorkRequest {
    work_id: String,
    nonce: u64,
    hash_hex: String,
    miner: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TransferTxRecord {
    tx_hash: Hash,
    from: Address,
    to: Address,
    value_hex: String,
    from_nonce: u64,
    timestamp: u64,
    block_number: u64,
}

impl RpcApi {
    pub fn new(config: RpcConfig, state_db: Arc<StateDb>, tx_pool: Arc<TransactionPool>) -> Self {
        let domain_registry = Arc::new(InMemoryDomainRegistry::new());
        domain_registry.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        Self {
            config,
            state_db,
            tx_pool,
            latest_block: RwLock::new(None),
            block_history: RwLock::new(BTreeMap::new()),
            compute_store: Arc::new(InMemoryObjectStore::new()),
            domain_registry,
            submitted_compute_results: RwLock::new(HashMap::new()),
            submitted_compute_order: RwLock::new(VecDeque::new()),
            transfer_txs: RwLock::new(HashMap::new()),
            transfer_tx_order: RwLock::new(VecDeque::new()),
            persistent_compute_store: None,
            mining_jobs: RwLock::new(HashMap::new()),
            mining_job_order: RwLock::new(VecDeque::new()),
            mining_seen_submissions: RwLock::new(HashSet::new()),
            mining_seen_submission_order: RwLock::new(VecDeque::new()),
            hashrate_counter: RwLock::new(0),
        }
    }

    /// Construct RPC API with injected compute backends.
    pub fn with_compute(
        config: RpcConfig,
        state_db: Arc<StateDb>,
        tx_pool: Arc<TransactionPool>,
        compute_store: Arc<dyn ObjectStore>,
        domain_registry: Arc<InMemoryDomainRegistry>,
    ) -> Self {
        Self {
            config,
            state_db,
            tx_pool,
            latest_block: RwLock::new(None),
            block_history: RwLock::new(BTreeMap::new()),
            compute_store,
            domain_registry,
            submitted_compute_results: RwLock::new(HashMap::new()),
            submitted_compute_order: RwLock::new(VecDeque::new()),
            transfer_txs: RwLock::new(HashMap::new()),
            transfer_tx_order: RwLock::new(VecDeque::new()),
            persistent_compute_store: None,
            mining_jobs: RwLock::new(HashMap::new()),
            mining_job_order: RwLock::new(VecDeque::new()),
            mining_seen_submissions: RwLock::new(HashSet::new()),
            mining_seen_submission_order: RwLock::new(VecDeque::new()),
            hashrate_counter: RwLock::new(0),
        }
    }

    /// Construct RPC API with durable compute store.
    pub fn with_persistent_compute(
        config: RpcConfig,
        state_db: Arc<StateDb>,
        tx_pool: Arc<TransactionPool>,
        compute_store: Arc<ComputeStore>,
        domain_registry: Arc<InMemoryDomainRegistry>,
    ) -> Self {
        Self {
            config,
            state_db,
            tx_pool,
            latest_block: RwLock::new(None),
            block_history: RwLock::new(BTreeMap::new()),
            compute_store: compute_store.clone(),
            domain_registry,
            submitted_compute_results: RwLock::new(HashMap::new()),
            submitted_compute_order: RwLock::new(VecDeque::new()),
            transfer_txs: RwLock::new(HashMap::new()),
            transfer_tx_order: RwLock::new(VecDeque::new()),
            persistent_compute_store: Some(compute_store),
            mining_jobs: RwLock::new(HashMap::new()),
            mining_job_order: RwLock::new(VecDeque::new()),
            mining_seen_submissions: RwLock::new(HashSet::new()),
            mining_seen_submission_order: RwLock::new(VecDeque::new()),
            hashrate_counter: RwLock::new(0),
        }
    }

    /// Handle RPC request
    pub async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        RPC_METRICS
            .method_calls
            .with_label_values(&[request.method.as_str()])
            .inc();
        let result = self.dispatch_method(&request.method, request.params).await;

        match result {
            Ok(value) => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                result: Some(value),
                error: None,
                id: request.id,
            },
            Err(error) => {
                RPC_METRICS
                    .method_errors
                    .with_label_values(&[request.method.as_str()])
                    .inc();
                JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    result: None,
                    error: Some(error),
                    id: request.id,
                }
            }
        }
    }

    /// Dispatch method call
    async fn dispatch_method(
        &self,
        method: &str,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        match method {
            // web3_* methods
            "web3_clientVersion" => self.web3_client_version(params),
            "web3_sha3" => self.web3_sha3(params),

            // net_* methods
            "net_version" => self.net_version(params),
            "net_peerCount" => self.net_peer_count(params),
            "net_listening" => self.net_listening(params),

            // ZeroChain extensions
            "zero_getAccount" => self.zero_get_account(params),
            "zero_getUtxos" => self.zero_get_utxos(params),
            "zero_getObject" => self.zero_get_object(params),
            "zero_getOutput" => self.zero_get_output(params),
            "zero_getDomain" => self.zero_get_domain(params),
            "zero_simulateComputeTx" => self.zero_simulate_compute_tx(params),
            "zero_submitComputeTx" => self.zero_submit_compute_tx(params),
            "zero_getComputeTxResult" => self.zero_get_compute_tx_result(params),
            "zero_listComputeTxResults" => self.zero_list_compute_tx_results(params),
            "zero_getTransactionByHash" => self.zero_get_transaction_by_hash(params),
            "zero_listTransactions" => self.zero_list_transactions(params),
            "zero_getTransactionsByAddress" => self.zero_get_transactions_by_address(params),
            "zero_getWork" => self.zero_get_work(params),
            "zero_submitWork" => self.zero_submit_work(params),
            "zero_getLatestBlock" => self.zero_get_latest_block(params),
            "zero_syncStatus" => self.zero_sync_status(params),
            "zero_getBlockByNumber" => self.zero_get_block_by_number(params),
            "zero_getBlocksRange" => self.zero_get_blocks_range(params),
            "zero_importBlock" => self.zero_import_block(params),
            "zero_getMetrics" => self.zero_get_metrics(params),
            "zero_peers" => self.zero_peers(params),
            "zero_transfer" => self.zero_transfer(params),

            _ => Err(RpcErrorObject::method_not_found(method)),
        }
    }

    // ============ web3_* methods ============

    fn web3_client_version(
        &self,
        _params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!("ZeroChain/v0.1.0"))
    }

    fn web3_sha3(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let data = params
            .first()
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing data".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Data must be string".to_string()))?;

        let bytes = hex::decode(data.strip_prefix("0x").unwrap_or(data))
            .map_err(|e| RpcErrorObject::invalid_params(format!("Invalid hex: {}", e)))?;

        let hash = zerocore::crypto::keccak256(&bytes);

        Ok(serde_json::json!(format!("0x{}", hex::encode(hash))))
    }

    // ============ net_* methods ============

    fn net_version(
        &self,
        _params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!(self.config.network_id.to_string()))
    }

    fn net_peer_count(
        &self,
        _params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!(format!("0x{:x}", global_peer_count())))
    }

    fn net_listening(
        &self,
        _params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!(true))
    }

    fn zero_transfer(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let tx = params
            .first()
            .and_then(|v| v.as_object())
            .ok_or_else(|| RpcErrorObject::invalid_params("transfer object missing".to_string()))?;

        let from = tx
            .get("from")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RpcErrorObject::invalid_params("from missing".to_string()))
            .and_then(parse_address)?;
        let to = tx
            .get("to")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RpcErrorObject::invalid_params("to missing".to_string()))
            .and_then(parse_address)?;
        let value = tx
            .get("value")
            .and_then(|v| v.as_str())
            .map(parse_u256_hex)
            .transpose()?
            .unwrap_or(U256::zero());

        let now = current_unix_secs();
        let mut from_account = self
            .state_db
            .get_account(&from)
            .ok_or_else(|| RpcErrorObject::invalid_params("from account not found".to_string()))?;
        if from_account.balance < value {
            return Err(RpcErrorObject::invalid_params(
                "insufficient balance".to_string(),
            ));
        }

        from_account.balance = from_account.balance.saturating_sub(value);
        from_account.nonce = from_account.nonce.saturating_add(1);
        from_account.updated_at = now;
        self.state_db.insert_account(from, from_account.clone());

        let mut to_account = self.state_db.get_account(&to).unwrap_or_else(|| Account {
            address: to,
            state: AccountState::Active,
            created_at: now,
            updated_at: now,
            ..Account::default()
        });
        to_account.balance = to_account.balance.saturating_add(value);
        to_account.updated_at = now;
        self.state_db.insert_account(to, to_account);

        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(from.as_bytes());
        hash_input.extend_from_slice(to.as_bytes());
        hash_input.extend_from_slice(&value.to_big_endian());
        hash_input.extend_from_slice(&from_account.nonce.to_be_bytes());
        hash_input.extend_from_slice(&now.to_be_bytes());
        let tx_hash = Hash::from_bytes(zerocore::crypto::keccak256(&hash_input));
        let current_block_number = self.current_head_block().header.number.as_u64();
        let record = TransferTxRecord {
            tx_hash,
            from,
            to,
            value_hex: format_u256_hex(value),
            from_nonce: from_account.nonce,
            timestamp: now,
            block_number: current_block_number,
        };
        self.record_transfer_tx(record);

        Ok(serde_json::json!(format!(
            "0x{}",
            hex::encode(tx_hash.as_bytes())
        )))
    }

    // ============ ZeroChain extensions ============

    fn zero_get_account(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;

        let address_str = params
            .first()
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing address".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Address must be string".to_string()))?;

        let address = parse_address(address_str)?;

        // Would get full account info
        Ok(serde_json::json!({
            "address": format_zero_address(address),
            "balance": format_u256_hex(self.state_db.get_balance(&address)),
            "nonce": format!("0x{:x}", self.state_db.get_nonce(&address)),
        }))
    }

    fn zero_get_utxos(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!([]))
    }

    fn zero_get_object(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let object_id = params
            .first()
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing object_id".to_string()))?
            .as_str()
            .ok_or_else(|| {
                RpcErrorObject::invalid_params("object_id must be string".to_string())
            })?;

        let object_id = parse_object_id(object_id)?;
        let maybe_output = self.compute_store.get_latest_output_by_object(object_id);
        Ok(serde_json::json!(maybe_output.map(object_output_to_json)))
    }

    fn zero_get_output(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let output_id = params
            .first()
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing output_id".to_string()))?
            .as_str()
            .ok_or_else(|| {
                RpcErrorObject::invalid_params("output_id must be string".to_string())
            })?;

        let output_id = parse_output_id(output_id)?;
        let maybe_output = self.compute_store.get_output(output_id);
        Ok(serde_json::json!(maybe_output.map(object_output_to_json)))
    }

    fn zero_get_domain(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let id = params
            .first()
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing domain_id".to_string()))?
            .as_u64()
            .ok_or_else(|| RpcErrorObject::invalid_params("domain_id must be u64".to_string()))?;

        let id_u32 = u32::try_from(id)
            .map_err(|_| RpcErrorObject::invalid_params("domain_id overflow".to_string()))?;

        let maybe_domain = self.domain_registry.get_domain(DomainId(id_u32));
        Ok(serde_json::json!(maybe_domain.map(|d| {
            serde_json::json!({
                "domain_id": d.domain_id.0,
                "name": d.name,
                "vm": d.vm,
                "public": d.public,
            })
        })))
    }

    fn zero_simulate_compute_tx(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let tx_value = params
            .first()
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing tx".to_string()))?
            .clone();

        let tx = parse_compute_tx(tx_value)?;
        let executor = BasicTxExecutor::new(
            self.compute_store.clone(),
            DefaultAuthorizationPolicy,
            NoopResourcePolicy,
            self.domain_registry.clone(),
        );

        let validator = zerocore::compute::BasicTxValidator {
            store: &executor.store,
            authorization: &executor.authorization,
            resources: &executor.resources,
            domains: &executor.domains,
        };

        match validator.validate(&tx) {
            Ok(report) => Ok(serde_json::json!({
                "ok": true,
                "inputs": report.inputs.len(),
                "reads": report.reads.len(),
                "outputs": tx.output_proposals.len(),
                "tx_id": format!("0x{}", hex::encode(tx.tx_id.0.as_bytes())),
            })),
            Err(err) => Ok(serde_json::json!({
                "ok": false,
                "error": compute_error_to_json(&err),
            })),
        }
    }

    fn zero_submit_compute_tx(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let tx_value = params
            .first()
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing tx".to_string()))?
            .clone();

        let tx = parse_compute_tx(tx_value)?;

        if let Some(persistent) = &self.persistent_compute_store {
            if let Ok(Some(existing)) = persistent.get_tx_result(tx.tx_id) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&existing) {
                    return Ok(serde_json::json!({
                        "ok": true,
                        "duplicate": true,
                        "result": v,
                    }));
                }
            }
        }

        if let Some(existing) = self
            .submitted_compute_results
            .read()
            .get(&tx.tx_id.0)
            .cloned()
        {
            return Ok(serde_json::json!({
                "ok": true,
                "duplicate": true,
                "result": existing,
            }));
        }

        let executor = BasicTxExecutor::new(
            self.compute_store.clone(),
            DefaultAuthorizationPolicy,
            NoopResourcePolicy,
            self.domain_registry.clone(),
        );

        let report = executor
            .execute(&tx)
            .map_err(|e| RpcErrorObject::invalid_params(format!("compute execute failed: {e}")))?;
        let submitted_at_unix = current_unix_secs();

        let result = serde_json::json!({
            "ok": true,
            "tx_id": format!("0x{}", hex::encode(tx.tx_id.0.as_bytes())),
            "consumed_inputs": report.inputs.len(),
            "read_objects": report.reads.len(),
            "created_outputs": tx.output_proposals.len(),
            "submitted_at_unix": submitted_at_unix,
        });

        if let Some(persistent) = &self.persistent_compute_store {
            let serialized = serde_json::to_string(&result).map_err(|e| {
                RpcErrorObject::internal_error(format!("serialize result failed: {e}"))
            })?;
            persistent
                .put_tx_result(tx.tx_id, &serialized)
                .map_err(|e| {
                    RpcErrorObject::internal_error(format!("persist result failed: {e}"))
                })?;
        }

        {
            let mut results = self.submitted_compute_results.write();
            let mut order = self.submitted_compute_order.write();
            let tx_hash = tx.tx_id.0;
            results.insert(tx_hash, result.clone());
            order.retain(|existing| existing != &tx_hash);
            order.push_back(tx_hash);
            while order.len() > MAX_SUBMITTED_COMPUTE_RESULTS {
                if let Some(stale) = order.pop_front() {
                    results.remove(&stale);
                }
            }
        }

        Ok(result)
    }

    fn zero_get_compute_tx_result(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let tx_id_s = params
            .first()
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing tx_id".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("tx_id must be string".to_string()))?;
        let tx_id = zerocore::compute::TxId(parse_hash(tx_id_s)?);

        if let Some(value) = self.submitted_compute_results.read().get(&tx_id.0).cloned() {
            return Ok(value);
        }

        if let Some(persistent) = &self.persistent_compute_store {
            let maybe = persistent.get_tx_result(tx_id).map_err(|e| {
                RpcErrorObject::internal_error(format!("load tx result failed: {e}"))
            })?;
            if let Some(raw) = maybe {
                let value = serde_json::from_str::<serde_json::Value>(&raw).map_err(|e| {
                    RpcErrorObject::internal_error(format!("decode tx result failed: {e}"))
                })?;
                return Ok(value);
            }
        }

        Ok(serde_json::Value::Null)
    }

    fn zero_list_compute_tx_results(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let mut page: usize = 1;
        let mut limit: usize = 20;
        if let Some(values) = params {
            if let Some(first) = values.first() {
                let obj = first.as_object().ok_or_else(|| {
                    RpcErrorObject::invalid_params(
                        "query object required for zero_listComputeTxResults".to_string(),
                    )
                })?;
                if let Some(parsed_page) = parse_u64_opt(obj.get("page"))? {
                    page = usize::try_from(parsed_page)
                        .map_err(|_| RpcErrorObject::invalid_params("page overflow".to_string()))?;
                }
                if let Some(parsed_limit) = parse_u64_opt(obj.get("limit"))? {
                    limit = usize::try_from(parsed_limit).map_err(|_| {
                        RpcErrorObject::invalid_params("limit overflow".to_string())
                    })?;
                }
            }
        }
        page = page.max(1);
        limit = limit.clamp(1, 200);
        let skip = page.saturating_sub(1).saturating_mul(limit);

        let order = self.submitted_compute_order.read();
        let results = self.submitted_compute_results.read();
        let total = order.len();
        let mut items = Vec::new();

        for tx_hash in order.iter().rev().skip(skip).take(limit) {
            if let Some(result) = results.get(tx_hash) {
                items.push(serde_json::json!({
                    "tx_id": format!("0x{}", hex::encode(tx_hash.as_bytes())),
                    "result": result.clone(),
                }));
            }
        }

        let has_more = skip.saturating_add(items.len()) < total;
        Ok(serde_json::json!({
            "page": page,
            "limit": limit,
            "total": total,
            "has_more": has_more,
            "items": items,
        }))
    }

    fn zero_get_transaction_by_hash(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let tx_hash = params
            .first()
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing tx hash".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("tx hash must be string".to_string()))
            .and_then(parse_hash)?;

        if let Some(record) = self.transfer_txs.read().get(&tx_hash).cloned() {
            return Ok(transfer_record_to_json(&record));
        }
        if let Some(result) = self.submitted_compute_results.read().get(&tx_hash).cloned() {
            return Ok(compute_tx_to_json(tx_hash, &result));
        }
        if let Some(persistent) = &self.persistent_compute_store {
            let maybe = persistent
                .get_tx_result(zerocore::compute::TxId(tx_hash))
                .map_err(|e| {
                    RpcErrorObject::internal_error(format!("load tx result failed: {e}"))
                })?;
            if let Some(raw) = maybe {
                let value = serde_json::from_str::<serde_json::Value>(&raw).map_err(|e| {
                    RpcErrorObject::internal_error(format!("decode tx result failed: {e}"))
                })?;
                return Ok(compute_tx_to_json(tx_hash, &value));
            }
        }

        Ok(serde_json::Value::Null)
    }

    fn zero_list_transactions(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let mut page: usize = 1;
        let mut limit: usize = 20;
        let mut include_transfer = true;
        let mut include_compute = true;
        if let Some(values) = params {
            if let Some(first) = values.first() {
                let obj = first.as_object().ok_or_else(|| {
                    RpcErrorObject::invalid_params(
                        "query object required for zero_listTransactions".to_string(),
                    )
                })?;
                if let Some(parsed_page) = parse_u64_opt(obj.get("page"))? {
                    page = usize::try_from(parsed_page)
                        .map_err(|_| RpcErrorObject::invalid_params("page overflow".to_string()))?;
                }
                if let Some(parsed_limit) = parse_u64_opt(obj.get("limit"))? {
                    limit = usize::try_from(parsed_limit).map_err(|_| {
                        RpcErrorObject::invalid_params("limit overflow".to_string())
                    })?;
                }
                if let Some(kind) = obj.get("kind").and_then(|v| v.as_str()) {
                    match kind {
                        "all" => {
                            include_transfer = true;
                            include_compute = true;
                        }
                        "transfer" => {
                            include_transfer = true;
                            include_compute = false;
                        }
                        "compute" => {
                            include_transfer = false;
                            include_compute = true;
                        }
                        _ => {
                            return Err(RpcErrorObject::invalid_params(
                                "kind must be one of all|transfer|compute".to_string(),
                            ));
                        }
                    }
                }
            }
        }

        page = page.max(1);
        limit = limit.clamp(1, 200);
        let skip = page.saturating_sub(1).saturating_mul(limit);
        let mut items: Vec<(u64, serde_json::Value)> = Vec::new();

        if include_transfer {
            let order = self.transfer_tx_order.read();
            let txs = self.transfer_txs.read();
            for tx_hash in order.iter().rev() {
                if let Some(record) = txs.get(tx_hash) {
                    items.push((record.timestamp, transfer_record_to_json(record)));
                }
            }
        }

        if include_compute {
            let order = self.submitted_compute_order.read();
            let results = self.submitted_compute_results.read();
            for tx_hash in order.iter().rev() {
                if let Some(result) = results.get(tx_hash) {
                    let ts = result
                        .get("submitted_at_unix")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    items.push((ts, compute_tx_to_json(*tx_hash, result)));
                }
            }
        }

        items.sort_by(|a, b| b.0.cmp(&a.0));
        let total = items.len();
        let page_items = items
            .into_iter()
            .skip(skip)
            .take(limit)
            .map(|(_, item)| item)
            .collect::<Vec<_>>();
        let has_more = skip.saturating_add(page_items.len()) < total;

        Ok(serde_json::json!({
            "page": page,
            "limit": limit,
            "total": total,
            "has_more": has_more,
            "items": page_items,
        }))
    }

    fn zero_get_transactions_by_address(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let mut address: Option<Address> = None;
        let mut page: usize = 1;
        let mut limit: usize = 20;

        if let Some(first) = params.first() {
            match first {
                serde_json::Value::String(s) => {
                    address = Some(parse_address(s)?);
                    if let Some(second) = params.get(1).and_then(|v| v.as_object()) {
                        if let Some(parsed_page) = parse_u64_opt(second.get("page"))? {
                            page = usize::try_from(parsed_page).map_err(|_| {
                                RpcErrorObject::invalid_params("page overflow".to_string())
                            })?;
                        }
                        if let Some(parsed_limit) = parse_u64_opt(second.get("limit"))? {
                            limit = usize::try_from(parsed_limit).map_err(|_| {
                                RpcErrorObject::invalid_params("limit overflow".to_string())
                            })?;
                        }
                    }
                }
                serde_json::Value::Object(obj) => {
                    let addr = obj.get("address").and_then(|v| v.as_str()).ok_or_else(|| {
                        RpcErrorObject::invalid_params("address is required".to_string())
                    })?;
                    address = Some(parse_address(addr)?);
                    if let Some(parsed_page) = parse_u64_opt(obj.get("page"))? {
                        page = usize::try_from(parsed_page).map_err(|_| {
                            RpcErrorObject::invalid_params("page overflow".to_string())
                        })?;
                    }
                    if let Some(parsed_limit) = parse_u64_opt(obj.get("limit"))? {
                        limit = usize::try_from(parsed_limit).map_err(|_| {
                            RpcErrorObject::invalid_params("limit overflow".to_string())
                        })?;
                    }
                }
                _ => {
                    return Err(RpcErrorObject::invalid_params(
                        "address query must be string or object".to_string(),
                    ));
                }
            }
        }

        let address = address
            .ok_or_else(|| RpcErrorObject::invalid_params("address is required".to_string()))?;
        page = page.max(1);
        limit = limit.clamp(1, 200);
        let skip = page.saturating_sub(1).saturating_mul(limit);

        let order = self.transfer_tx_order.read();
        let txs = self.transfer_txs.read();
        let mut filtered = Vec::new();
        for tx_hash in order.iter().rev() {
            if let Some(record) = txs.get(tx_hash) {
                if record.from == address || record.to == address {
                    filtered.push(transfer_record_to_json(record));
                }
            }
        }

        let total = filtered.len();
        let items = filtered
            .into_iter()
            .skip(skip)
            .take(limit)
            .collect::<Vec<_>>();
        let has_more = skip.saturating_add(items.len()) < total;

        Ok(serde_json::json!({
            "address": format_zero_address(address),
            "page": page,
            "limit": limit,
            "total": total,
            "has_more": has_more,
            "items": items,
        }))
    }

    fn zero_get_work(
        &self,
        _params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let now = current_unix_secs();
        let latest = self.latest_block.read();
        let target_leading_zero_bytes = latest
            .as_ref()
            .map(|b| leading_zero_target_from_difficulty(b.header.difficulty))
            .unwrap_or_else(|| {
                leading_zero_target_from_difficulty(U256::from_u128(BASE_MINING_DIFFICULTY))
            });
        let (prev_hash, height) = match latest.as_ref() {
            Some(b) => (b.header.hash, b.header.number.as_u64().saturating_add(1)),
            None => (Hash::zero(), 1),
        };

        let work_id = format!("work-{}-{}", height, now);
        let work = MiningWork {
            work_id: work_id.clone(),
            prev_hash,
            height,
            target_leading_zero_bytes,
            created_at_secs: now,
        };
        {
            let mut jobs = self.mining_jobs.write();
            let mut order = self.mining_job_order.write();

            while let Some(front) = order.front().cloned() {
                let should_drop = match jobs.get(&front) {
                    Some(existing) => {
                        now.saturating_sub(existing.created_at_secs) > MAX_MINING_JOB_AGE_SECS
                    }
                    None => true,
                };
                if !should_drop {
                    break;
                }
                order.pop_front();
                jobs.remove(&front);
            }

            jobs.insert(work_id.clone(), work.clone());
            order.push_back(work_id.clone());

            while order.len() > MAX_MINING_JOBS {
                if let Some(stale_work_id) = order.pop_front() {
                    jobs.remove(&stale_work_id);
                }
            }
        }

        Ok(serde_json::json!({
            "work_id": work.work_id,
            "prev_hash": format!("0x{}", hex::encode(work.prev_hash.as_bytes())),
            "height": work.height,
            "target_leading_zero_bytes": work.target_leading_zero_bytes,
            "coinbase": format_zero_address(
                parse_address(&self.config.coinbase)
                    .map_err(|e| RpcErrorObject::internal_error(format!("invalid coinbase: {e}")))?,
            ),
        }))
    }

    fn zero_submit_work(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let req_value = params
            .first()
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing work payload".to_string()))?
            .clone();
        let req: SubmitWorkRequest = serde_json::from_value(req_value).map_err(|e| {
            RpcErrorObject::invalid_params(format!("invalid submit work payload: {e}"))
        })?;

        let work = self
            .mining_jobs
            .read()
            .get(&req.work_id)
            .cloned()
            .ok_or_else(|| {
                RpcErrorObject::invalid_params("unknown or stale work_id".to_string())
            })?;

        let hash_bytes = hex::decode(req.hash_hex.strip_prefix("0x").unwrap_or(&req.hash_hex))
            .map_err(|e| RpcErrorObject::invalid_params(format!("invalid hash hex: {e}")))?;
        if hash_bytes.len() != 32 {
            return Err(RpcErrorObject::invalid_params(
                "hash must be 32 bytes".to_string(),
            ));
        }
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&hash_bytes);
        let leading = hash_bytes.iter().take_while(|b| **b == 0).count();
        if leading < work.target_leading_zero_bytes {
            RPC_METRICS
                .mining_shares_rejected
                .with_label_values(&["low_difficulty_share"])
                .inc();
            return Ok(serde_json::json!({
                "accepted": false,
                "reason": "low_difficulty_share"
            }));
        }

        // Minimal consistency check with node-side work template.
        let mut data = Vec::new();
        data.extend_from_slice(work.prev_hash.as_bytes());
        data.extend_from_slice(&work.height.to_be_bytes());
        data.extend_from_slice(&req.nonce.to_be_bytes());
        let expected = zerocore::crypto::keccak256(&data);
        if expected != hash {
            RPC_METRICS
                .mining_shares_rejected
                .with_label_values(&["invalid_pow_hash"])
                .inc();
            return Ok(serde_json::json!({
                "accepted": false,
                "reason": "invalid_pow_hash"
            }));
        }

        let miner_label = req.miner.unwrap_or_else(|| "zero-miner".to_string());
        if miner_label.as_bytes().len() > MAX_MINER_EXTRA_DATA_BYTES {
            RPC_METRICS
                .mining_shares_rejected
                .with_label_values(&["invalid_miner_label"])
                .inc();
            return Ok(serde_json::json!({
                "accepted": false,
                "reason": "invalid_miner_label"
            }));
        }

        let seen_key = SeenShareKey {
            work_id: req.work_id.clone(),
            nonce: req.nonce,
            hash,
        };
        {
            let mut seen = self.mining_seen_submissions.write();
            let mut order = self.mining_seen_submission_order.write();
            if !seen.insert(seen_key.clone()) {
                RPC_METRICS
                    .mining_shares_rejected
                    .with_label_values(&["duplicate_share"])
                    .inc();
                return Ok(serde_json::json!({
                    "accepted": false,
                    "reason": "duplicate_share"
                }));
            }
            order.push_back(seen_key);
            while order.len() > MAX_SEEN_MINING_SUBMISSIONS {
                if let Some(stale) = order.pop_front() {
                    seen.remove(&stale);
                }
            }
        }

        let consumed = self.mining_jobs.write().remove(&req.work_id).is_some();
        self.mining_job_order
            .write()
            .retain(|work_id| work_id != &req.work_id);
        if !consumed {
            RPC_METRICS
                .mining_shares_rejected
                .with_label_values(&["stale_or_duplicate_work"])
                .inc();
            return Ok(serde_json::json!({
                "accepted": false,
                "reason": "stale_or_duplicate_work"
            }));
        }

        {
            let mut counter = self.hashrate_counter.write();
            *counter = counter.saturating_add(1);
        }
        RPC_METRICS
            .mining_shares_accepted
            .with_label_values(&["zero_submitWork"])
            .inc();

        // Build and publish a synthetic block header into latest_block for MVP chain progress.
        let parent = self.latest_block.read().as_ref().map(|b| b.header.clone());
        let parent_hash = parent.as_ref().map(|h| h.hash).unwrap_or(Hash::zero());
        let parent_number = parent.as_ref().map(|h| h.number).unwrap_or(U256::zero());
        let parent_difficulty = parent
            .as_ref()
            .map(|h| h.difficulty)
            .unwrap_or(U256::from_u128(BASE_MINING_DIFFICULTY));
        let timestamp = current_unix_secs();
        let parent_timestamp = parent
            .as_ref()
            .map(|h| h.timestamp)
            .unwrap_or_else(|| timestamp.saturating_sub(TARGET_BLOCK_INTERVAL_SECS));
        let difficulty = adjust_mining_difficulty(parent_difficulty, parent_timestamp, timestamp);

        let mut header = BlockHeader {
            version: 1,
            parent_hash,
            uncle_hashes: Vec::new(),
            coinbase: parse_address(&self.config.coinbase)
                .map_err(|e| RpcErrorObject::internal_error(format!("invalid coinbase: {e}")))?,
            state_root: Hash::zero(),
            transactions_root: Hash::zero(),
            receipts_root: Hash::zero(),
            number: parent_number + U256::one(),
            gas_limit: 30_000_000,
            gas_used: 0,
            timestamp,
            difficulty,
            nonce: req.nonce,
            extra_data: miner_label.into_bytes(),
            mix_hash: Hash::from_bytes(expected),
            base_fee_per_gas: U256::from(1_000_000_000u64),
            hash: Hash::zero(),
        };
        header.hash = header.compute_hash();

        let block = Block {
            header: header.clone(),
            transactions: Vec::new(),
            uncles: Vec::new(),
        };
        self.store_block(block.clone());
        *self.latest_block.write() = Some(block);
        set_global_synced_height(header.number.as_u64());
        self.credit_block_reward(header.coinbase, header.number);
        RPC_METRICS
            .latest_block_height
            .set(header.number.as_u64() as i64);

        Ok(serde_json::json!({
            "accepted": true,
            "block_hash": format!("0x{}", hex::encode(header.hash.as_bytes())),
            "height": header.number.as_u64(),
        }))
    }

    fn zero_get_metrics(
        &self,
        _params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let text = RPC_METRICS.render()?;
        Ok(serde_json::json!({ "text": text }))
    }

    fn zero_peers(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        if let Some(values) = params {
            if !values.is_empty() {
                return Err(RpcErrorObject::invalid_params(
                    "zero_peers does not accept params".to_string(),
                ));
            }
        }

        let now = current_unix_secs();
        let peers = global_peers()
            .into_iter()
            .map(|peer| {
                let idle_secs = now.saturating_sub(peer.last_activity);
                serde_json::json!({
                    "peer_id": peer.peer_id,
                    "network_id": peer.network_id,
                    "protocol_version": peer.protocol_version,
                    "client_version": peer.client_version,
                    "remote_addr": peer.remote_addr.to_string(),
                    "local_addr": peer.local_addr.to_string(),
                    "connected_at": peer.connected_at,
                    "last_activity": peer.last_activity,
                    "idle_secs": idle_secs,
                    "reputation": peer.reputation,
                    "capabilities": peer.capabilities,
                })
            })
            .collect::<Vec<_>>();

        Ok(serde_json::json!(peers))
    }

    fn zero_get_latest_block(
        &self,
        _params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(block_to_zero_block_json(&self.current_head_block()))
    }

    fn zero_sync_status(
        &self,
        _params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let local_head = self.current_head_block().header.number.as_u64();
        let network_head = global_synced_height();
        Ok(serde_json::json!({
            "local_head": local_head,
            "network_head": network_head,
            "syncing": network_head > local_head,
        }))
    }

    fn zero_get_block_by_number(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let number = parse_u64_opt(params.first())?
            .ok_or_else(|| RpcErrorObject::invalid_params("number is required".to_string()))?;
        let block = self.block_by_number(number);
        Ok(block
            .as_ref()
            .map(block_to_zero_block_json)
            .unwrap_or(serde_json::Value::Null))
    }

    fn zero_get_blocks_range(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let latest = self.current_head_block().header.number.as_u64();
        let mut from: Option<u64> = None;
        let mut to: Option<u64> = None;
        let mut limit: usize = 20;

        if let Some(values) = params {
            if let Some(first) = values.first() {
                let obj = first.as_object().ok_or_else(|| {
                    RpcErrorObject::invalid_params(
                        "query object required for zero_getBlocksRange".to_string(),
                    )
                })?;
                from = parse_u64_opt(obj.get("from"))?;
                to = parse_u64_opt(obj.get("to"))?;
                if let Some(parsed_limit) = parse_u64_opt(obj.get("limit"))? {
                    limit = usize::try_from(parsed_limit).map_err(|_| {
                        RpcErrorObject::invalid_params("limit overflow".to_string())
                    })?;
                }
            }
        }
        limit = limit.clamp(1, 500);
        let to = to.unwrap_or(latest).min(latest);
        let from = from
            .unwrap_or_else(|| to.saturating_sub(limit as u64).saturating_add(1))
            .min(to);

        let mut items = Vec::new();
        for number in (from..=to).rev() {
            if items.len() >= limit {
                break;
            }
            if let Some(block) = self.block_by_number(number) {
                items.push(block_to_zero_block_json(&block));
            }
        }

        Ok(serde_json::json!({
            "from": from,
            "to": to,
            "limit": limit,
            "items": items,
        }))
    }

    fn current_head_block(&self) -> Block {
        self.latest_block
            .read()
            .clone()
            .or_else(global_latest_block)
            .unwrap_or_else(create_genesis_block)
    }

    fn block_by_number(&self, number: u64) -> Option<Block> {
        if number == 0 {
            return Some(create_genesis_block());
        }

        if let Some(found) = self.block_history.read().get(&number).cloned() {
            return Some(found);
        }

        if let Some(found) = global_block_by_number(number) {
            return Some(found);
        }

        self.latest_block
            .read()
            .as_ref()
            .filter(|block| block.header.number.as_u64() == number)
            .cloned()
            .or_else(|| {
                let head = self.current_head_block();
                (head.header.number.as_u64() == number).then_some(head)
            })
    }

    fn store_block(&self, block: Block) {
        let height = block.header.number.as_u64();
        let mut history = self.block_history.write();
        history.insert(height, block.clone());
        while history.len() > MAX_BLOCK_HISTORY {
            let Some(oldest) = history.keys().next().copied() else {
                break;
            };
            history.remove(&oldest);
        }
        global_store_block(block);
    }

    fn record_transfer_tx(&self, record: TransferTxRecord) {
        let tx_hash = record.tx_hash;
        let mut txs = self.transfer_txs.write();
        let mut order = self.transfer_tx_order.write();
        txs.insert(tx_hash, record);
        order.retain(|h| h != &tx_hash);
        order.push_back(tx_hash);
        while order.len() > MAX_TRANSFER_HISTORY {
            if let Some(stale) = order.pop_front() {
                txs.remove(&stale);
            }
        }
    }

    fn credit_block_reward(&self, coinbase: Address, block_number: U256) {
        let reward = block_reward_for_height(block_number.as_u64());
        if reward.is_zero() {
            return;
        }

        let now = current_unix_secs();
        let mut account = self
            .state_db
            .get_account(&coinbase)
            .unwrap_or_else(|| Account {
                address: coinbase,
                state: AccountState::Active,
                created_at: now,
                updated_at: now,
                ..Account::default()
            });

        account.balance = account.balance.saturating_add(reward);
        account.updated_at = now;
        self.state_db.insert_account(coinbase, account);
    }

    fn zero_import_block(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let block = params
            .first()
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing block".to_string()))?
            .as_object()
            .ok_or_else(|| RpcErrorObject::invalid_params("block must be object".to_string()))?;

        let hash = parse_hash_field(block, "hash")?;
        let parent_hash = parse_hash_field(block, "parent_hash")?;
        let number = parse_u64_hex_field(block, "number")?;
        let timestamp = block
            .get("timestamp")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                RpcErrorObject::invalid_params("timestamp missing or invalid".to_string())
            })?;
        let difficulty_u64 = parse_u64_hex_field(block, "difficulty")?;
        let nonce = block.get("nonce").and_then(|v| v.as_u64()).ok_or_else(|| {
            RpcErrorObject::invalid_params("nonce missing or invalid".to_string())
        })?;
        let coinbase = block
            .get("coinbase")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                RpcErrorObject::invalid_params("coinbase missing or invalid".to_string())
            })?;
        let coinbase = parse_address(coinbase)?;
        let mix_hash = parse_hash_field(block, "mix_hash")?;
        let extra_data = parse_bytes_hex_opt(block.get("extra_data"))?.unwrap_or_default();

        let mut latest = self.latest_block.write();
        if let Some(current) = latest.as_ref() {
            let current_num = current.header.number.as_u64();
            if number <= current_num {
                return Ok(serde_json::json!({
                    "imported": false,
                    "reason": "stale_or_duplicate"
                }));
            }
            if number != current_num.saturating_add(1) || parent_hash != current.header.hash {
                return Ok(serde_json::json!({
                    "imported": false,
                    "reason": "parent_mismatch"
                }));
            }
        }

        let header = BlockHeader {
            version: 1,
            parent_hash,
            uncle_hashes: Vec::new(),
            coinbase,
            state_root: Hash::zero(),
            transactions_root: Hash::zero(),
            receipts_root: Hash::zero(),
            number: U256::from(number),
            gas_limit: 30_000_000,
            gas_used: 0,
            timestamp,
            difficulty: U256::from(difficulty_u64),
            nonce,
            extra_data,
            mix_hash,
            base_fee_per_gas: U256::from(1_000_000_000u64),
            hash,
        };
        let block = Block {
            header: header.clone(),
            transactions: Vec::new(),
            uncles: Vec::new(),
        };
        self.store_block(block.clone());
        *latest = Some(block);
        set_global_synced_height(number);

        Ok(serde_json::json!({
            "imported": true,
            "height": number,
            "hash": format!("0x{}", hex::encode(header.hash.as_bytes())),
        }))
    }
}

fn leading_zero_target_from_difficulty(difficulty: U256) -> usize {
    let raw = difficulty.as_u64() as u128;
    if raw >= 8_000_000 {
        4
    } else if raw >= 2_000_000 {
        3
    } else {
        2
    }
}

fn adjust_mining_difficulty(parent_difficulty: U256, parent_timestamp: u64, now: u64) -> U256 {
    let elapsed = now.saturating_sub(parent_timestamp);
    let mut next = parent_difficulty.as_u64() as u128;
    if next == 0 {
        next = BASE_MINING_DIFFICULTY;
    }

    if elapsed <= TARGET_BLOCK_INTERVAL_SECS / 2 {
        next = next.saturating_mul(110) / 100;
    } else if elapsed >= TARGET_BLOCK_INTERVAL_SECS.saturating_mul(2) {
        next = next.saturating_mul(90) / 100;
    }

    let bounded = next.clamp(MIN_MINING_DIFFICULTY, MAX_MINING_DIFFICULTY);
    U256::from_u128(bounded)
}

fn current_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn compute_error_to_json(err: &zerocore::compute::ComputeError) -> serde_json::Value {
    let (numeric_code, code, category) = match err {
        zerocore::compute::ComputeError::DomainNotRegistered(_)
        | zerocore::compute::ComputeError::DomainNotPublic(_)
        | zerocore::compute::ComputeError::DomainMismatch { .. } => {
            (1001, "domain_error", "domain")
        }
        zerocore::compute::ComputeError::ReadVersionMismatch { .. }
        | zerocore::compute::ComputeError::ReadSetValidationFailed => {
            (2001, "readset_error", "readset")
        }
        zerocore::compute::ComputeError::AuthorizationDenied => {
            (3001, "authorization_error", "authorization")
        }
        zerocore::compute::ComputeError::OwnershipCheckFailed => {
            (3002, "ownership_check_failed", "authorization")
        }
        zerocore::compute::ComputeError::InvalidSignature => {
            (3003, "invalid_signature", "authorization")
        }
        zerocore::compute::ComputeError::SignatureOwnerMismatch => {
            (3004, "signature_owner_mismatch", "authorization")
        }
        zerocore::compute::ComputeError::TxIdMismatch => (3005, "tx_id_mismatch", "authorization"),
        zerocore::compute::ComputeError::UnsupportedSignatureScheme => {
            (3006, "unsupported_signature_scheme", "authorization")
        }
        zerocore::compute::ComputeError::InvalidPredecessor
        | zerocore::compute::ComputeError::InvalidVersionProgression
        | zerocore::compute::ComputeError::DuplicateOutputId
        | zerocore::compute::ComputeError::ObjectNotFound(_) => (4001, "state_error", "state"),
        zerocore::compute::ComputeError::ResourcePolicyViolation => {
            (5001, "resource_error", "resource")
        }
        zerocore::compute::ComputeError::InvalidObjectKind
        | zerocore::compute::ComputeError::InvalidTransaction(_) => {
            (6001, "tx_error", "transaction")
        }
    };

    serde_json::json!({
        "numeric_code": numeric_code,
        "code": code,
        "category": category,
        "message": err.to_string(),
    })
}

/// RPC Server
pub struct RpcServer {
    config: RpcConfig,
    api: Option<Arc<RpcApi>>,
    server_task: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
    shutdown_tx: parking_lot::Mutex<Option<oneshot::Sender<()>>>,
}

#[derive(Clone)]
struct RpcServerState {
    api: Arc<RpcApi>,
    security: Arc<RpcSecurityContext>,
}

struct RpcSecurityContext {
    auth_token: Option<String>,
    rate_limit_per_minute: u32,
    buckets: parking_lot::Mutex<HashMap<String, VecDeque<u64>>>,
}

impl RpcSecurityContext {
    fn new(config: &RpcConfig) -> Self {
        Self {
            auth_token: config.auth_token.clone(),
            rate_limit_per_minute: config.rate_limit_per_minute,
            buckets: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    fn allow_request(&self, client: &str) -> bool {
        if self.rate_limit_per_minute == 0 {
            return true;
        }

        let now = current_unix_secs();
        let mut buckets = self.buckets.lock();
        let window = buckets.entry(client.to_string()).or_default();
        while let Some(ts) = window.front() {
            if now.saturating_sub(*ts) > 60 {
                window.pop_front();
            } else {
                break;
            }
        }

        if window.len() >= self.rate_limit_per_minute as usize {
            return false;
        }
        window.push_back(now);
        true
    }
}

impl RpcServer {
    /// Creates server with validation and returns detailed error on invalid config.
    pub fn try_new(config: RpcConfig) -> Result<Self, crate::ApiError> {
        config.validate().map_err(crate::ApiError::InvalidRequest)?;

        let api = Some(Arc::new(build_default_rpc_api(config.clone())));
        Ok(Self {
            config,
            api,
            server_task: parking_lot::Mutex::new(None),
            shutdown_tx: parking_lot::Mutex::new(None),
        })
    }

    pub fn new(config: RpcConfig) -> Self {
        match Self::try_new(config.clone()) {
            Ok(server) => server,
            Err(err) => {
                tracing::warn!("invalid rpc config, fallback to default: {}", err);
                // Keep backward compatibility for callers expecting infallible constructor.
                Self::try_new(RpcConfig::default()).expect("default RpcConfig must be valid")
            }
        }
    }

    /// Create server with externally provided RPC API instance.
    pub fn with_api(config: RpcConfig, api: Arc<RpcApi>) -> Self {
        Self {
            config,
            api: Some(api),
            server_task: parking_lot::Mutex::new(None),
            shutdown_tx: parking_lot::Mutex::new(None),
        }
    }

    /// Returns the RPC API instance if initialized.
    pub fn api(&self) -> Option<Arc<RpcApi>> {
        self.api.clone()
    }

    pub async fn start(&self) -> Result<(), crate::ApiError> {
        if self.server_task.lock().is_some() {
            return Ok(());
        }

        let api = self
            .api
            .as_ref()
            .cloned()
            .ok_or_else(|| crate::ApiError::Internal("RPC API not initialized".to_string()))?;

        let state = RpcServerState {
            api,
            security: Arc::new(RpcSecurityContext::new(&self.config)),
        };

        let app = Router::new()
            .route("/", post(handle_rpc_request))
            .layer(DefaultBodyLimit::max(self.config.max_request_size))
            .layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_headers(Any)
                    .allow_methods(Any),
            )
            .with_state(state);

        let bind_addr = format!("{}:{}", self.config.address, self.config.port);
        let listener = tokio::net::TcpListener::bind(&bind_addr)
            .await
            .map_err(|e| {
                crate::ApiError::IO(std::io::Error::new(std::io::ErrorKind::AddrInUse, e))
            })?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        *self.shutdown_tx.lock() = Some(shutdown_tx);

        let task = tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            });

            if let Err(err) = server.await {
                tracing::error!("RPC server exited with error: {}", err);
            }
        });

        *self.server_task.lock() = Some(task);
        Ok(())
    }

    pub async fn stop(&self) -> Result<(), crate::ApiError> {
        if let Some(tx) = self.shutdown_tx.lock().take() {
            let _ = tx.send(());
        }
        let task = self.server_task.lock().take();
        if let Some(task) = task {
            let _ = task.await;
        }
        Ok(())
    }
}

async fn handle_rpc_request(
    State(state): State<RpcServerState>,
    headers: HeaderMap,
    Json(request): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    if !is_authorized(&headers, state.security.auth_token.as_deref()) {
        return Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(RpcErrorObject {
                code: -32001,
                message: "Unauthorized".into(),
                data: None,
            }),
            id: request.id,
        });
    }

    let client = extract_client_key(&headers);
    if !state.security.allow_request(&client) {
        return Json(JsonRpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(RpcErrorObject {
                code: -32005,
                message: "Rate limit exceeded".into(),
                data: Some(serde_json::json!({
                    "client": client,
                    "limit_per_minute": state.security.rate_limit_per_minute
                })),
            }),
            id: request.id,
        });
    }

    Json(state.api.handle_request(request).await)
}

fn extract_client_key(headers: &HeaderMap) -> String {
    if let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let first = v.split(',').next().unwrap_or_default().trim();
        if !first.is_empty() {
            return first.to_string();
        }
    }
    if let Some(v) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        let ip = v.trim();
        if !ip.is_empty() {
            return ip.to_string();
        }
    }
    "local".to_string()
}

fn is_authorized(headers: &HeaderMap, expected_token: Option<&str>) -> bool {
    let Some(expected) = expected_token else {
        return true;
    };

    let bearer_ok = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|v| v.trim() == expected)
        .unwrap_or(false);
    if bearer_ok {
        return true;
    }

    headers
        .get("x-zero-token")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim() == expected)
        .unwrap_or(false)
}

fn build_default_rpc_api(config: RpcConfig) -> RpcApi {
    let account_manager = Arc::new(InMemoryAccountManager::new());
    let tx_pool = Arc::new(TransactionPool::new(
        TxPoolConfig::default(),
        account_manager,
    ));
    let state_db = Arc::new(StateDb::new(Hash::zero()));

    let persistent_db = build_compute_kv_backend(&config);
    let compute_store = Arc::new(ComputeStore::new(persistent_db));

    let domains = Arc::new(InMemoryDomainRegistry::new());
    domains.upsert_domain(DomainConfig {
        domain_id: DomainId(0),
        name: "main".to_string(),
        vm: "wasm".to_string(),
        public: true,
    });

    RpcApi::with_persistent_compute(config, state_db, tx_pool, compute_store, domains)
}

fn build_compute_kv_backend(config: &RpcConfig) -> Arc<dyn KeyValueDB> {
    match config.compute_backend {
        ComputeBackend::Mem => Arc::new(MemDatabase::new()),
        ComputeBackend::RocksDb => match RocksDb::open(&config.compute_db_path) {
            Ok(db) => Arc::new(db),
            Err(err) => {
                tracing::warn!(
                    "failed to open rocksdb at {}: {}, fallback to mem",
                    config.compute_db_path,
                    err
                );
                Arc::new(MemDatabase::new())
            }
        },
        ComputeBackend::Redb => match RedbDatabase::open(&config.compute_db_path) {
            Ok(db) => Arc::new(db),
            Err(err) => {
                tracing::warn!(
                    "failed to open redb at {}: {}, fallback to mem",
                    config.compute_db_path,
                    err
                );
                Arc::new(MemDatabase::new())
            }
        },
    }
}

fn parse_address(s: &str) -> Result<Address, RpcErrorObject> {
    let raw = s.trim();
    if raw.len() != 45 {
        return Err(RpcErrorObject::invalid_params(
            "Address must be 20 bytes".to_string(),
        ));
    }
    let prefix = raw.get(..5).ok_or_else(|| {
        RpcErrorObject::invalid_params("Address must use ZER0x prefix".to_string())
    })?;
    if !prefix.eq_ignore_ascii_case("ZER0x") {
        return Err(RpcErrorObject::invalid_params(
            "Address must use ZER0x prefix".to_string(),
        ));
    }
    let body = raw
        .get(5..)
        .ok_or_else(|| RpcErrorObject::invalid_params("Address must be 20 bytes".to_string()))?;
    if body.len() != 40 || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(RpcErrorObject::invalid_params(
            "Address must be 20 bytes".to_string(),
        ));
    }
    let bytes = hex::decode(body)
        .map_err(|e| RpcErrorObject::invalid_params(format!("Invalid address: {}", e)))?;

    Ok(Address::from_slice(&bytes).unwrap())
}

fn parse_hash(s: &str) -> Result<Hash, RpcErrorObject> {
    let bytes = hex::decode(s.strip_prefix("0x").unwrap_or(s))
        .map_err(|e| RpcErrorObject::invalid_params(format!("Invalid hash: {}", e)))?;

    if bytes.len() != 32 {
        return Err(RpcErrorObject::invalid_params(
            "Hash must be 32 bytes".into(),
        ));
    }

    Ok(Hash::from_slice(&bytes).unwrap())
}

fn parse_u256_hex(s: &str) -> Result<U256, RpcErrorObject> {
    let raw = s.strip_prefix("0x").unwrap_or(s);
    let normalized = if raw.len().is_multiple_of(2) {
        raw.to_string()
    } else {
        format!("0{}", raw)
    };
    let bytes = hex::decode(normalized)
        .map_err(|e| RpcErrorObject::invalid_params(format!("Invalid u256 hex: {}", e)))?;
    if bytes.len() > 32 {
        return Err(RpcErrorObject::invalid_params(
            "u256 must be <= 32 bytes".to_string(),
        ));
    }
    Ok(U256::from_big_endian(&bytes))
}

fn parse_object_id(s: &str) -> Result<ObjectId, RpcErrorObject> {
    Ok(ObjectId(parse_hash(s)?))
}

fn parse_output_id(s: &str) -> Result<OutputId, RpcErrorObject> {
    Ok(OutputId(parse_hash(s)?))
}

fn block_reward_for_height(block_number: u64) -> U256 {
    let mut reward = zerocore::INITIAL_BLOCK_REWARD;
    let halving_count = block_number / zerocore::HALVING_PERIOD;
    for _ in 0..halving_count {
        reward /= 2;
    }
    U256::from_u128(reward)
}

fn format_u256_hex(value: U256) -> String {
    let bytes = value.to_big_endian();
    let first_non_zero = bytes.iter().position(|b| *b != 0);
    match first_non_zero {
        Some(idx) => {
            let encoded = hex::encode(&bytes[idx..]);
            let trimmed = encoded.trim_start_matches('0');
            if trimmed.is_empty() {
                "0x0".to_string()
            } else {
                format!("0x{}", trimmed)
            }
        }
        None => "0x0".to_string(),
    }
}

fn format_u128_hex(value: u128) -> String {
    format!("0x{:x}", value)
}

fn format_zero_address(address: Address) -> String {
    let lower_hex = hex::encode(address.as_bytes());
    let hash = zerocore::crypto::keccak256(lower_hex.as_bytes());
    let mut checksummed = String::with_capacity(40);

    for (idx, ch) in lower_hex.chars().enumerate() {
        let nibble = if idx % 2 == 0 {
            (hash[idx / 2] >> 4) & 0x0f
        } else {
            hash[idx / 2] & 0x0f
        };

        if ch.is_ascii_hexdigit() && ch.is_ascii_lowercase() && nibble >= 8 {
            checksummed.push(ch.to_ascii_uppercase());
        } else {
            checksummed.push(ch);
        }
    }

    format!("ZER0x{}", checksummed)
}

fn transfer_record_to_json(record: &TransferTxRecord) -> serde_json::Value {
    serde_json::json!({
        "kind": "transfer",
        "tx_hash": format!("0x{}", hex::encode(record.tx_hash.as_bytes())),
        "hash": format!("0x{}", hex::encode(record.tx_hash.as_bytes())),
        "from": format_zero_address(record.from),
        "to": format_zero_address(record.to),
        "value": record.value_hex,
        "from_nonce": format!("0x{:x}", record.from_nonce),
        "timestamp": record.timestamp,
        "block_number": record.block_number,
        "status": "executed",
    })
}

fn compute_tx_to_json(tx_hash: Hash, result: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "kind": "compute",
        "tx_hash": format!("0x{}", hex::encode(tx_hash.as_bytes())),
        "hash": format!("0x{}", hex::encode(tx_hash.as_bytes())),
        "timestamp": result.get("submitted_at_unix").and_then(|v| v.as_u64()).unwrap_or(0),
        "result": result,
    })
}

fn block_to_zero_block_json(block: &Block) -> serde_json::Value {
    serde_json::json!({
        "hash": format!("0x{}", hex::encode(block.header.hash.as_bytes())),
        "parent_hash": format!("0x{}", hex::encode(block.header.parent_hash.as_bytes())),
        "number": format!("0x{:x}", block.header.number.as_u64()),
        "timestamp": block.header.timestamp,
        "difficulty": format!("0x{:x}", block.header.difficulty.as_u64()),
        "nonce": block.header.nonce,
        "coinbase": format_zero_address(block.header.coinbase),
        "mix_hash": format!("0x{}", hex::encode(block.header.mix_hash.as_bytes())),
        "extra_data": format!("0x{}", hex::encode(&block.header.extra_data)),
    })
}

fn object_output_to_json(output: ObjectOutput) -> serde_json::Value {
    serde_json::json!({
        "output_id": format!("0x{}", hex::encode(output.output_id.0.as_bytes())),
        "object_id": format!("0x{}", hex::encode(output.object_id.0.as_bytes())),
        "version": output.version.0,
        "domain_id": output.domain_id.0,
        "kind": format!("{:?}", output.kind),
        "owner": ownership_to_json(&output.owner),
        "spent": output.spent,
        "predecessor": output.predecessor.map(|p| format!("0x{}", hex::encode(p.0.as_bytes()))),
        "state": format!("0x{}", hex::encode(output.state)),
        "state_root": output
            .state_root
            .map(|root| format!("0x{}", hex::encode(root.as_bytes()))),
        "resources": resource_map_to_json(&output.resources),
        "lock": script_to_json(&output.lock),
        "logic": output.logic.as_ref().map(script_to_json),
        "created_at": output.created_at,
        "ttl": output.ttl,
        "rent_reserve": output.rent_reserve.map(format_u128_hex),
        "flags": output.flags,
        "extensions": metadata_to_json(&output.extensions),
    })
}

fn ownership_to_json(owner: &Ownership) -> serde_json::Value {
    match owner {
        Ownership::Shared => serde_json::json!({ "type": "Shared" }),
        Ownership::Address(address) => serde_json::json!({
            "type": "Address",
            "address": format_zero_address(*address),
        }),
        Ownership::Program(address) => serde_json::json!({
            "type": "Program",
            "address": format_zero_address(*address),
        }),
        Ownership::NativeEd25519(public_key) => serde_json::json!({
            "type": "NativeEd25519",
            "public_key": format!("0x{}", hex::encode(public_key)),
        }),
    }
}

fn script_to_json(script: &Script) -> serde_json::Value {
    serde_json::json!({
        "vm": script.vm,
        "code": format!("0x{}", hex::encode(&script.code)),
    })
}

fn resource_map_to_json(resources: &ResourceMap) -> serde_json::Value {
    let values = resources
        .iter()
        .map(|(asset_id, value)| {
            let value = match value {
                ResourceValue::Amount(amount) => serde_json::json!({
                    "type": "Amount",
                    "amount": format_u128_hex(*amount),
                }),
                ResourceValue::Data(data) => serde_json::json!({
                    "type": "Data",
                    "data": format!("0x{}", hex::encode(data)),
                }),
                ResourceValue::Ref(object_id) => serde_json::json!({
                    "type": "Ref",
                    "object_id": format!("0x{}", hex::encode(object_id.0.as_bytes())),
                }),
                ResourceValue::RefBatch(object_ids) => serde_json::json!({
                    "type": "RefBatch",
                    "object_ids": object_ids
                        .iter()
                        .map(|id| format!("0x{}", hex::encode(id.0.as_bytes())))
                        .collect::<Vec<_>>(),
                }),
            };
            serde_json::json!({
                "asset_id": format!("0x{}", hex::encode(asset_id.as_bytes())),
                "value": value,
            })
        })
        .collect::<Vec<_>>();
    serde_json::Value::Array(values)
}

fn metadata_to_json(metadata: &[(String, Vec<u8>)]) -> serde_json::Value {
    serde_json::Value::Array(
        metadata
            .iter()
            .map(|(key, value)| {
                serde_json::json!({
                    "key": key,
                    "value": format!("0x{}", hex::encode(value)),
                })
            })
            .collect::<Vec<_>>(),
    )
}

fn parse_compute_tx(value: serde_json::Value) -> Result<ComputeTx, RpcErrorObject> {
    let obj = value
        .as_object()
        .ok_or_else(|| RpcErrorObject::invalid_params("tx must be object".to_string()))?;

    let tx_id = parse_hash_field(obj, "tx_id").map(zerocore::compute::TxId)?;
    let domain_id = DomainId(parse_u32_field(obj, "domain_id")?);
    let command =
        parse_command(obj.get("command").and_then(|v| v.as_str()).ok_or_else(|| {
            RpcErrorObject::invalid_params("command must be string".to_string())
        })?)?;

    let input_set = parse_hash_array_field(obj, "input_set")?
        .into_iter()
        .map(OutputId)
        .collect::<Vec<_>>();

    let read_set = parse_read_set(obj.get("read_set"))?;
    let output_proposals = parse_output_proposals(obj.get("output_proposals"))?;
    let fee = parse_u64_opt(obj.get("fee"))?.unwrap_or(0);
    let nonce = parse_u64_opt(obj.get("nonce"))?;
    let metadata = parse_metadata(obj.get("metadata"))?;

    let payload = parse_bytes_hex_opt(obj.get("payload"))?.unwrap_or_default();
    let deadline_unix_secs = obj.get("deadline_unix_secs").and_then(|v| v.as_u64());
    let chain_id = obj.get("chain_id").and_then(|v| v.as_u64());
    let network_id = match obj.get("network_id").and_then(|v| v.as_u64()) {
        None => None,
        Some(v) => Some(
            u32::try_from(v)
                .map_err(|_| RpcErrorObject::invalid_params("network_id overflow".to_string()))?,
        ),
    };
    let witness = parse_witness(obj.get("witness"))?;

    Ok(ComputeTx {
        tx_id,
        domain_id,
        command,
        input_set,
        read_set,
        output_proposals,
        fee,
        nonce,
        metadata,
        payload,
        deadline_unix_secs,
        chain_id,
        network_id,
        witness,
    })
}

fn parse_witness(v: Option<&serde_json::Value>) -> Result<TxWitness, RpcErrorObject> {
    let obj = v
        .and_then(|x| x.as_object())
        .ok_or_else(|| RpcErrorObject::invalid_params("witness must be object".to_string()))?;
    let sig_arr = obj
        .get("signatures")
        .and_then(|x| x.as_array())
        .ok_or_else(|| {
            RpcErrorObject::invalid_params("witness.signatures must be array".to_string())
        })?;

    let mut signatures = Vec::with_capacity(sig_arr.len());
    for raw in sig_arr {
        if let Some(s) = raw.as_str() {
            let bytes = hex::decode(s.strip_prefix("0x").unwrap_or(s)).map_err(|e| {
                RpcErrorObject::invalid_params(format!("invalid signature hex: {e}"))
            })?;
            let sig = Signature::from_bytes(&bytes).map_err(|e| {
                RpcErrorObject::invalid_params(format!("invalid signature bytes: {e}"))
            })?;
            signatures.push(TxSignature::secp256k1(sig));
            continue;
        }

        let obj = raw.as_object().ok_or_else(|| {
            RpcErrorObject::invalid_params("signature must be string or object".to_string())
        })?;
        let scheme = obj.get("scheme").and_then(|x| x.as_str()).ok_or_else(|| {
            RpcErrorObject::invalid_params("signature.scheme must be string".to_string())
        })?;
        let sig_hex = obj
            .get("signature")
            .and_then(|x| x.as_str())
            .ok_or_else(|| {
                RpcErrorObject::invalid_params("signature.signature must be string".to_string())
            })?;
        let sig_bytes = hex::decode(sig_hex.strip_prefix("0x").unwrap_or(sig_hex))
            .map_err(|e| RpcErrorObject::invalid_params(format!("invalid signature hex: {e}")))?;

        match scheme {
            "secp256k1" => {
                let sig = Signature::from_bytes(&sig_bytes).map_err(|e| {
                    RpcErrorObject::invalid_params(format!("invalid signature bytes: {e}"))
                })?;
                signatures.push(TxSignature::secp256k1(sig));
            }
            "ed25519" => {
                let pubkey_hex =
                    obj.get("public_key")
                        .and_then(|x| x.as_str())
                        .ok_or_else(|| {
                            RpcErrorObject::invalid_params(
                                "ed25519 signature requires public_key".to_string(),
                            )
                        })?;
                let pubkey = hex::decode(pubkey_hex.strip_prefix("0x").unwrap_or(pubkey_hex))
                    .map_err(|e| {
                        RpcErrorObject::invalid_params(format!("invalid public_key hex: {e}"))
                    })?;
                if sig_bytes.len() != 64 {
                    return Err(RpcErrorObject::invalid_params(
                        "ed25519 signature must be 64 bytes".to_string(),
                    ));
                }
                if pubkey.len() != 32 {
                    return Err(RpcErrorObject::invalid_params(
                        "ed25519 public_key must be 32 bytes".to_string(),
                    ));
                }
                signatures.push(TxSignature {
                    scheme: SignatureScheme::Ed25519,
                    bytes: sig_bytes,
                    public_key: Some(pubkey),
                });
            }
            other => {
                return Err(RpcErrorObject::invalid_params(format!(
                    "unsupported signature scheme: {other}"
                )));
            }
        }
    }

    let threshold = match obj.get("threshold") {
        None | Some(serde_json::Value::Null) => None,
        Some(raw) => {
            let v = raw.as_u64().ok_or_else(|| {
                RpcErrorObject::invalid_params("witness.threshold must be u64".to_string())
            })?;
            Some(u16::try_from(v).map_err(|_| {
                RpcErrorObject::invalid_params("witness.threshold overflow".to_string())
            })?)
        }
    };

    Ok(TxWitness {
        signatures,
        threshold,
    })
}

fn parse_command(s: &str) -> Result<Command, RpcErrorObject> {
    match s {
        "Transfer" => Ok(Command::Transfer),
        "Invoke" => Ok(Command::Invoke),
        "Mint" => Ok(Command::Mint),
        "Burn" => Ok(Command::Burn),
        "Anchor" => Ok(Command::Anchor),
        "Reveal" => Ok(Command::Reveal),
        "AgentTick" => Ok(Command::AgentTick),
        _ => Err(RpcErrorObject::invalid_params(format!(
            "unsupported command: {s}"
        ))),
    }
}

fn parse_object_kind(s: &str) -> Result<ObjectKind, RpcErrorObject> {
    match s {
        "Asset" => Ok(ObjectKind::Asset),
        "Code" => Ok(ObjectKind::Code),
        "State" => Ok(ObjectKind::State),
        "Capability" => Ok(ObjectKind::Capability),
        "Agent" => Ok(ObjectKind::Agent),
        "Anchor" => Ok(ObjectKind::Anchor),
        "Ticket" => Ok(ObjectKind::Ticket),
        _ => Err(RpcErrorObject::invalid_params(format!(
            "unsupported object kind: {s}"
        ))),
    }
}

fn parse_ownership(v: Option<&serde_json::Value>) -> Result<Ownership, RpcErrorObject> {
    let Some(v) = v else {
        return Ok(Ownership::Shared);
    };
    let obj = v
        .as_object()
        .ok_or_else(|| RpcErrorObject::invalid_params("owner must be object".to_string()))?;
    let typ = obj
        .get("type")
        .and_then(|x| x.as_str())
        .ok_or_else(|| RpcErrorObject::invalid_params("owner.type missing".to_string()))?;
    match typ {
        "Shared" => Ok(Ownership::Shared),
        "Address" => {
            let addr = obj.get("address").and_then(|x| x.as_str()).ok_or_else(|| {
                RpcErrorObject::invalid_params("owner.address missing".to_string())
            })?;
            Ok(Ownership::Address(parse_address(addr)?))
        }
        "Program" => {
            let addr = obj.get("address").and_then(|x| x.as_str()).ok_or_else(|| {
                RpcErrorObject::invalid_params("owner.address missing".to_string())
            })?;
            Ok(Ownership::Program(parse_address(addr)?))
        }
        "NativeEd25519" => {
            let pubkey = obj
                .get("public_key")
                .and_then(|x| x.as_str())
                .ok_or_else(|| {
                    RpcErrorObject::invalid_params("owner.public_key missing".to_string())
                })?;
            let bytes = hex::decode(pubkey.strip_prefix("0x").unwrap_or(pubkey)).map_err(|e| {
                RpcErrorObject::invalid_params(format!("invalid owner.public_key hex: {e}"))
            })?;
            if bytes.len() != 32 {
                return Err(RpcErrorObject::invalid_params(
                    "owner.public_key must be 32 bytes".to_string(),
                ));
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok(Ownership::NativeEd25519(arr))
        }
        _ => Err(RpcErrorObject::invalid_params(format!(
            "unsupported owner type: {typ}"
        ))),
    }
}

fn parse_read_set(
    v: Option<&serde_json::Value>,
) -> Result<Vec<zerocore::compute::ObjectReadRef>, RpcErrorObject> {
    let Some(v) = v else {
        return Ok(vec![]);
    };
    let arr = v
        .as_array()
        .ok_or_else(|| RpcErrorObject::invalid_params("read_set must be array".to_string()))?;

    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let obj = item.as_object().ok_or_else(|| {
            RpcErrorObject::invalid_params("read_set item must be object".to_string())
        })?;
        let output_id = parse_hash_field(obj, "output_id").map(OutputId)?;
        let domain_id = DomainId(parse_u32_field(obj, "domain_id")?);
        let expected_version = Version(
            obj.get("expected_version")
                .and_then(|x| x.as_u64())
                .ok_or_else(|| {
                    RpcErrorObject::invalid_params("expected_version missing".to_string())
                })?,
        );
        out.push(zerocore::compute::ObjectReadRef {
            output_id,
            domain_id,
            expected_version,
        });
    }
    Ok(out)
}

fn parse_output_proposals(
    v: Option<&serde_json::Value>,
) -> Result<Vec<OutputProposal>, RpcErrorObject> {
    let Some(v) = v else {
        return Ok(vec![]);
    };
    let arr = v.as_array().ok_or_else(|| {
        RpcErrorObject::invalid_params("output_proposals must be array".to_string())
    })?;

    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let obj = item.as_object().ok_or_else(|| {
            RpcErrorObject::invalid_params("output proposal must be object".to_string())
        })?;
        let output_id = parse_hash_field(obj, "output_id").map(OutputId)?;
        let object_id = parse_hash_field(obj, "object_id").map(ObjectId)?;
        let domain_id = DomainId(parse_u32_field(obj, "domain_id")?);
        let kind = parse_object_kind(
            obj.get("kind")
                .and_then(|x| x.as_str())
                .ok_or_else(|| RpcErrorObject::invalid_params("kind missing".to_string()))?,
        )?;
        let owner = parse_ownership(obj.get("owner"))?;
        let predecessor = match obj.get("predecessor") {
            Some(serde_json::Value::String(s)) => Some(OutputId(parse_hash(s)?)),
            Some(serde_json::Value::Null) | None => None,
            _ => {
                return Err(RpcErrorObject::invalid_params(
                    "predecessor must be hex string or null".to_string(),
                ));
            }
        };
        let version = Version(
            obj.get("version")
                .and_then(|x| x.as_u64())
                .ok_or_else(|| RpcErrorObject::invalid_params("version missing".to_string()))?,
        );
        let state = parse_bytes_hex_opt(obj.get("state"))?.unwrap_or_default();
        let state_root = parse_hash_opt(obj.get("state_root"))?;
        let resources = parse_resource_map(obj.get("resources"))?;
        let lock = parse_script(obj.get("lock"))?.unwrap_or_default();
        let logic = parse_script(obj.get("logic"))?;
        let created_at = parse_u64_opt(obj.get("created_at"))?.unwrap_or(0);
        let ttl = parse_u64_opt(obj.get("ttl"))?;
        let rent_reserve = parse_u128_opt(obj.get("rent_reserve"))?;
        let flags = parse_u32_opt(obj.get("flags"))?.unwrap_or(0);
        let extensions = parse_metadata(obj.get("extensions"))?;
        out.push(OutputProposal {
            output_id,
            object_id,
            domain_id,
            kind,
            owner,
            predecessor,
            version,
            state,
            state_root,
            resources,
            lock,
            logic,
            created_at,
            ttl,
            rent_reserve,
            flags,
            extensions,
        });
    }

    Ok(out)
}

fn parse_hash_array_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Vec<Hash>, RpcErrorObject> {
    let Some(v) = obj.get(key) else {
        return Ok(vec![]);
    };
    let arr = v
        .as_array()
        .ok_or_else(|| RpcErrorObject::invalid_params(format!("{key} must be array")))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let s = item.as_str().ok_or_else(|| {
            RpcErrorObject::invalid_params(format!("{key} items must be hex string"))
        })?;
        out.push(parse_hash(s)?);
    }
    Ok(out)
}

fn parse_hash_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Hash, RpcErrorObject> {
    let s = obj
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| RpcErrorObject::invalid_params(format!("{key} missing")))?;
    parse_hash(s)
}

fn parse_u64_hex_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<u64, RpcErrorObject> {
    let raw = obj
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| RpcErrorObject::invalid_params(format!("{key} must be hex string")))?;
    let s = raw.strip_prefix("0x").unwrap_or(raw);
    u64::from_str_radix(s, 16)
        .map_err(|e| RpcErrorObject::invalid_params(format!("invalid {key} hex: {e}")))
}

fn parse_u32_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<u32, RpcErrorObject> {
    let v = obj
        .get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| RpcErrorObject::invalid_params(format!("{key} missing")))?;
    u32::try_from(v).map_err(|_| RpcErrorObject::invalid_params(format!("{key} overflow")))
}

fn parse_u64_opt(v: Option<&serde_json::Value>) -> Result<Option<u64>, RpcErrorObject> {
    match v {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(num)) => num
            .as_u64()
            .map(Some)
            .ok_or_else(|| RpcErrorObject::invalid_params("expected u64".to_string())),
        Some(serde_json::Value::String(s)) => {
            let raw = s.trim();
            let parsed = if let Some(hex) = raw.strip_prefix("0x") {
                u64::from_str_radix(hex, 16)
            } else {
                raw.parse::<u64>()
            }
            .map_err(|e| RpcErrorObject::invalid_params(format!("invalid u64 value: {e}")))?;
            Ok(Some(parsed))
        }
        _ => Err(RpcErrorObject::invalid_params(
            "expected u64/hex string/null".to_string(),
        )),
    }
}

fn parse_u32_opt(v: Option<&serde_json::Value>) -> Result<Option<u32>, RpcErrorObject> {
    let Some(raw) = parse_u64_opt(v)? else {
        return Ok(None);
    };
    Ok(Some(u32::try_from(raw).map_err(|_| {
        RpcErrorObject::invalid_params("u32 overflow".to_string())
    })?))
}

fn parse_u128_opt(v: Option<&serde_json::Value>) -> Result<Option<u128>, RpcErrorObject> {
    match v {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(num)) => {
            if let Some(v) = num.as_u64() {
                return Ok(Some(v as u128));
            }
            Err(RpcErrorObject::invalid_params("expected u128".to_string()))
        }
        Some(serde_json::Value::String(s)) => {
            let raw = s.trim();
            let parsed = if let Some(hex) = raw.strip_prefix("0x") {
                u128::from_str_radix(hex, 16)
            } else {
                raw.parse::<u128>()
            }
            .map_err(|e| RpcErrorObject::invalid_params(format!("invalid u128 value: {e}")))?;
            Ok(Some(parsed))
        }
        _ => Err(RpcErrorObject::invalid_params(
            "expected u128/hex string/null".to_string(),
        )),
    }
}

fn parse_hash_opt(v: Option<&serde_json::Value>) -> Result<Option<Hash>, RpcErrorObject> {
    match v {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(parse_hash(s)?)),
        _ => Err(RpcErrorObject::invalid_params(
            "expected hash hex string or null".to_string(),
        )),
    }
}

fn parse_script(v: Option<&serde_json::Value>) -> Result<Option<Script>, RpcErrorObject> {
    let Some(v) = v else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    let obj = v
        .as_object()
        .ok_or_else(|| RpcErrorObject::invalid_params("script must be object".to_string()))?;
    let vm = parse_u64_opt(obj.get("vm"))?.unwrap_or(1);
    let vm = u8::try_from(vm)
        .map_err(|_| RpcErrorObject::invalid_params("script.vm overflow".to_string()))?;
    let code = parse_bytes_hex_opt(obj.get("code"))?.unwrap_or_default();
    Ok(Some(Script { vm, code }))
}

fn parse_resource_map(v: Option<&serde_json::Value>) -> Result<ResourceMap, RpcErrorObject> {
    let Some(v) = v else {
        return Ok(vec![]);
    };
    let arr = v
        .as_array()
        .ok_or_else(|| RpcErrorObject::invalid_params("resources must be array".to_string()))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let obj = item.as_object().ok_or_else(|| {
            RpcErrorObject::invalid_params("resource item must be object".to_string())
        })?;
        let asset_id = parse_hash(
            obj.get("asset_id")
                .and_then(|x| x.as_str())
                .ok_or_else(|| RpcErrorObject::invalid_params("asset_id missing".to_string()))?,
        )?;
        let value_obj = obj
            .get("value")
            .and_then(|x| x.as_object())
            .ok_or_else(|| RpcErrorObject::invalid_params("resource.value missing".to_string()))?;
        let value_type = value_obj
            .get("type")
            .and_then(|x| x.as_str())
            .ok_or_else(|| {
                RpcErrorObject::invalid_params("resource.value.type missing".to_string())
            })?;
        let value = match value_type {
            "Amount" => {
                ResourceValue::Amount(parse_u128_opt(value_obj.get("amount"))?.ok_or_else(
                    || RpcErrorObject::invalid_params("resource amount missing".to_string()),
                )?)
            }
            "Data" => ResourceValue::Data(parse_bytes_hex_opt(value_obj.get("data"))?.ok_or_else(
                || RpcErrorObject::invalid_params("resource data missing".to_string()),
            )?),
            "Ref" => {
                let object_id = parse_hash(
                    value_obj
                        .get("object_id")
                        .and_then(|x| x.as_str())
                        .ok_or_else(|| {
                            RpcErrorObject::invalid_params("resource object_id missing".to_string())
                        })?,
                )?;
                ResourceValue::Ref(ObjectId(object_id))
            }
            "RefBatch" => {
                let refs = value_obj
                    .get("object_ids")
                    .and_then(|x| x.as_array())
                    .ok_or_else(|| {
                        RpcErrorObject::invalid_params(
                            "resource object_ids must be array".to_string(),
                        )
                    })?
                    .iter()
                    .map(|v| {
                        let s = v.as_str().ok_or_else(|| {
                            RpcErrorObject::invalid_params(
                                "resource object_ids item must be string".to_string(),
                            )
                        })?;
                        Ok(ObjectId(parse_hash(s)?))
                    })
                    .collect::<Result<Vec<_>, RpcErrorObject>>()?;
                ResourceValue::RefBatch(refs)
            }
            other => {
                return Err(RpcErrorObject::invalid_params(format!(
                    "unsupported resource value type: {other}"
                )));
            }
        };
        out.push((asset_id, value));
    }
    out.sort_by_key(|(asset_id, _)| *asset_id);
    for pair in out.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(RpcErrorObject::invalid_params(
                "duplicate asset_id in resources".to_string(),
            ));
        }
    }
    Ok(out)
}

fn parse_metadata(v: Option<&serde_json::Value>) -> Result<Vec<(String, Vec<u8>)>, RpcErrorObject> {
    let Some(v) = v else {
        return Ok(vec![]);
    };
    let arr = v
        .as_array()
        .ok_or_else(|| RpcErrorObject::invalid_params("metadata must be array".to_string()))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let obj = item.as_object().ok_or_else(|| {
            RpcErrorObject::invalid_params("metadata item must be object".to_string())
        })?;
        let key = obj
            .get("key")
            .and_then(|x| x.as_str())
            .ok_or_else(|| RpcErrorObject::invalid_params("metadata key missing".to_string()))?
            .to_string();
        let value = parse_bytes_hex_opt(obj.get("value"))?
            .ok_or_else(|| RpcErrorObject::invalid_params("metadata value missing".to_string()))?;
        out.push((key, value));
    }
    Ok(out)
}

fn parse_bytes_hex_opt(v: Option<&serde_json::Value>) -> Result<Option<Vec<u8>>, RpcErrorObject> {
    match v {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => {
            let bytes = hex::decode(s.strip_prefix("0x").unwrap_or(s))
                .map_err(|e| RpcErrorObject::invalid_params(format!("invalid hex bytes: {e}")))?;
            Ok(Some(bytes))
        }
        _ => Err(RpcErrorObject::invalid_params(
            "expected hex string or null".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer as _;
    use std::sync::Arc;
    use zerostore::db::MemDatabase;

    fn build_test_api_with_compute() -> RpcApi {
        zeronet::global_reset_sync_cache();
        let account_manager = Arc::new(InMemoryAccountManager::new());
        let tx_pool = Arc::new(TransactionPool::new(
            TxPoolConfig::default(),
            account_manager,
        ));
        let state_db = Arc::new(StateDb::new(Hash::zero()));

        let store = Arc::new(InMemoryObjectStore::new());
        let domains = Arc::new(InMemoryDomainRegistry::new());
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        RpcApi::with_compute(RpcConfig::default(), state_db, tx_pool, store, domains)
    }

    fn build_test_api_with_persistent_compute() -> RpcApi {
        zeronet::global_reset_sync_cache();
        let account_manager = Arc::new(InMemoryAccountManager::new());
        let tx_pool = Arc::new(TransactionPool::new(
            TxPoolConfig::default(),
            account_manager,
        ));
        let state_db = Arc::new(StateDb::new(Hash::zero()));
        let db = Arc::new(MemDatabase::new());
        let persistent_store = Arc::new(ComputeStore::new(db));

        let domains = Arc::new(InMemoryDomainRegistry::new());
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        RpcApi::with_persistent_compute(
            RpcConfig::default(),
            state_db,
            tx_pool,
            persistent_store,
            domains,
        )
    }

    fn canonicalize_compute_tx_id(mut tx_json: serde_json::Value) -> serde_json::Value {
        let mut tx = parse_compute_tx(tx_json.clone()).expect("tx json should parse");
        tx.assign_expected_tx_id();
        tx_json["tx_id"] = serde_json::Value::String(format!("0x{}", tx.tx_id.0.to_hex()));
        tx_json
    }

    fn mine_one_block(api: &RpcApi, nonce: u64, miner: &str) -> serde_json::Value {
        let work = api.zero_get_work(None).expect("work should be available");
        let work_id = work["work_id"].as_str().unwrap().to_string();
        let prev_hash = work["prev_hash"].as_str().unwrap().to_string();
        let height = work["height"].as_u64().unwrap();
        {
            let mut jobs = api.mining_jobs.write();
            if let Some(job) = jobs.get_mut(&work_id) {
                job.target_leading_zero_bytes = 0;
            }
        }
        let prev_hash_bytes =
            hex::decode(prev_hash.strip_prefix("0x").unwrap_or(&prev_hash)).unwrap();
        let mut data = Vec::new();
        data.extend_from_slice(&prev_hash_bytes);
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&nonce.to_be_bytes());
        let digest = zerocore::crypto::keccak256(&data);
        api.zero_submit_work(Some(vec![serde_json::json!({
            "work_id": work_id,
            "nonce": nonce,
            "hash_hex": format!("0x{}", hex::encode(digest)),
            "miner": miner
        })]))
        .expect("submit work")
    }

    #[test]
    fn test_parse_compute_tx_accepts_ed25519_witness_and_owner() {
        let signer = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let verify = signer.verifying_key();
        let owner_pub_hex = format!("0x{}", hex::encode(verify.to_bytes()));

        let mut tx = serde_json::json!({
            "tx_id": format!("0x{}", hex::encode([0x91u8; 32])),
            "domain_id": 0,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Transfer",
            "nonce": 1,
            "input_set": [format!("0x{}", hex::encode([0x92u8; 32]))],
            "read_set": [],
            "output_proposals": [{
                "output_id": format!("0x{}", hex::encode([0x93u8; 32])),
                "object_id": format!("0x{}", hex::encode([0x94u8; 32])),
                "domain_id": 0,
                "kind": "Asset",
                "owner": { "type": "NativeEd25519", "public_key": owner_pub_hex },
                "predecessor": format!("0x{}", hex::encode([0x92u8; 32])),
                "version": 2,
                "state": "0x01",
                "logic": null
            }],
            "payload": "0x1234",
            "deadline_unix_secs": 1900000000u64,
            "witness": {"signatures": [], "threshold": 1}
        });

        let parsed = parse_compute_tx(tx.clone()).expect("tx should parse");
        let sig = signer.sign(&parsed.signing_preimage());
        tx["witness"]["signatures"] = serde_json::json!([{
            "scheme": "ed25519",
            "signature": format!("0x{}", hex::encode(sig.to_bytes())),
            "public_key": format!("0x{}", hex::encode(verify.to_bytes()))
        }]);

        let parsed = parse_compute_tx(tx).expect("ed25519 tx should parse");
        assert_eq!(parsed.witness.signatures.len(), 1);
        assert_eq!(
            parsed.witness.signatures[0].scheme,
            SignatureScheme::Ed25519
        );
    }

    #[test]
    fn test_zero_get_work_returns_work_payload() {
        let api = build_test_api_with_persistent_compute();
        let work = api
            .zero_get_work(None)
            .expect("zero_getWork should succeed");
        assert!(work.get("work_id").and_then(|v| v.as_str()).is_some());
        assert!(work.get("height").and_then(|v| v.as_u64()).is_some());
        assert_eq!(
            work.get("target_leading_zero_bytes")
                .and_then(|v| v.as_u64()),
            Some(2)
        );
    }

    #[test]
    fn test_zero_submit_work_accepts_valid_share() {
        let api = build_test_api_with_persistent_compute();
        let work = api.zero_get_work(None).expect("work should be available");
        let work_id = work
            .get("work_id")
            .and_then(|v| v.as_str())
            .expect("work_id missing")
            .to_string();
        let prev_hash = work
            .get("prev_hash")
            .and_then(|v| v.as_str())
            .expect("prev_hash missing")
            .to_string();
        let height = work
            .get("height")
            .and_then(|v| v.as_u64())
            .expect("height missing");

        {
            let mut jobs = api.mining_jobs.write();
            if let Some(job) = jobs.get_mut(&work_id) {
                job.target_leading_zero_bytes = 0;
            }
        }

        let prev_hash_bytes =
            hex::decode(prev_hash.strip_prefix("0x").unwrap_or(&prev_hash)).expect("prev hash hex");
        let nonce = 42u64;
        let mut data = Vec::new();
        data.extend_from_slice(&prev_hash_bytes);
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&nonce.to_be_bytes());
        let digest = zerocore::crypto::keccak256(&data);
        let hash_hex = format!("0x{}", hex::encode(digest));
        let submit = api
            .zero_submit_work(Some(vec![serde_json::json!({
                "work_id": work_id,
                "nonce": nonce,
                "hash_hex": hash_hex,
                "miner": "test-miner"
            })]))
            .expect("submit should succeed");

        assert_eq!(submit.get("accepted").and_then(|v| v.as_bool()), Some(true));
        assert!(submit.get("block_hash").and_then(|v| v.as_str()).is_some());
    }

    #[test]
    fn test_zero_submit_work_credits_coinbase_balance() {
        let api = build_test_api_with_persistent_compute();
        let work = api.zero_get_work(None).expect("work should be available");
        let work_id = work
            .get("work_id")
            .and_then(|v| v.as_str())
            .expect("work_id missing")
            .to_string();
        let prev_hash = work
            .get("prev_hash")
            .and_then(|v| v.as_str())
            .expect("prev_hash missing")
            .to_string();
        let height = work
            .get("height")
            .and_then(|v| v.as_u64())
            .expect("height missing");

        {
            let mut jobs = api.mining_jobs.write();
            if let Some(job) = jobs.get_mut(&work_id) {
                job.target_leading_zero_bytes = 0;
            }
        }

        let prev_hash_bytes =
            hex::decode(prev_hash.strip_prefix("0x").unwrap_or(&prev_hash)).expect("prev hash hex");
        let nonce = 123u64;
        let mut data = Vec::new();
        data.extend_from_slice(&prev_hash_bytes);
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&nonce.to_be_bytes());
        let digest = zerocore::crypto::keccak256(&data);
        let hash_hex = format!("0x{}", hex::encode(digest));

        let submit = api
            .zero_submit_work(Some(vec![serde_json::json!({
                "work_id": work_id,
                "nonce": nonce,
                "hash_hex": hash_hex,
                "miner": "reward-test-miner"
            })]))
            .expect("submit should succeed");
        assert_eq!(submit.get("accepted").and_then(|v| v.as_bool()), Some(true));

        let coinbase = api.config.coinbase.clone();
        let account = api
            .zero_get_account(Some(vec![serde_json::json!(coinbase)]))
            .expect("coinbase account");
        let balance = account
            .get("balance")
            .and_then(|v| v.as_str())
            .expect("balance string");
        let expected = format!("0x{:x}", zerocore::INITIAL_BLOCK_REWARD);

        assert_eq!(balance, expected);
    }

    #[test]
    fn test_zero_submit_work_rejects_low_difficulty_share() {
        let api = build_test_api_with_persistent_compute();
        let work = api.zero_get_work(None).expect("work should be available");
        let work_id = work
            .get("work_id")
            .and_then(|v| v.as_str())
            .expect("work_id missing");

        let submit = api
            .zero_submit_work(Some(vec![serde_json::json!({
                "work_id": work_id,
                "nonce": 1,
                "hash_hex": "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                "miner": "test-miner"
            })]))
            .expect("submit should return result");

        assert_eq!(
            submit.get("accepted").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            submit.get("reason").and_then(|v| v.as_str()),
            Some("low_difficulty_share")
        );
    }

    #[test]
    fn test_zero_import_block_updates_latest_block() {
        let api = build_test_api_with_persistent_compute();

        let first = api.zero_get_work(None).expect("get work");
        let work_id = first["work_id"].as_str().unwrap().to_string();
        let prev_hash = first["prev_hash"].as_str().unwrap().to_string();
        let height = first["height"].as_u64().unwrap();

        {
            let mut jobs = api.mining_jobs.write();
            if let Some(job) = jobs.get_mut(&work_id) {
                job.target_leading_zero_bytes = 0;
            }
        }

        let prev_hash_bytes =
            hex::decode(prev_hash.strip_prefix("0x").unwrap_or(&prev_hash)).unwrap();
        let nonce = 7u64;
        let mut data = Vec::new();
        data.extend_from_slice(&prev_hash_bytes);
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&nonce.to_be_bytes());
        let digest = zerocore::crypto::keccak256(&data);

        let mined = api
            .zero_submit_work(Some(vec![serde_json::json!({
                "work_id": work_id,
                "nonce": nonce,
                "hash_hex": format!("0x{}", hex::encode(digest)),
                "miner": "test-miner"
            })]))
            .expect("submit work");
        assert_eq!(mined["accepted"].as_bool(), Some(true));

        let latest = api.zero_get_latest_block(None).expect("latest block");
        assert!(latest.get("hash").is_some());
        assert_eq!(latest.get("number").and_then(|v| v.as_str()), Some("0x1"));
    }

    #[test]
    fn test_zero_get_latest_block_defaults_to_genesis() {
        let api = build_test_api_with_persistent_compute();

        let latest = api.zero_get_latest_block(None).expect("latest block");
        assert_eq!(latest.get("number").and_then(|v| v.as_str()), Some("0x0"));
        assert!(latest.get("hash").and_then(|v| v.as_str()).is_some());
    }

    #[test]
    fn test_zero_sync_status_reports_gap_without_synthesizing_block() {
        let api = build_test_api_with_persistent_compute();
        set_global_synced_height(12);

        let status = api
            .zero_sync_status(None)
            .expect("sync status should succeed");
        let local_head = status
            .get("local_head")
            .and_then(|v| v.as_u64())
            .expect("local_head");
        let network_head = status
            .get("network_head")
            .and_then(|v| v.as_u64())
            .expect("network_head");
        let syncing = status
            .get("syncing")
            .and_then(|v| v.as_bool())
            .expect("syncing");
        assert_eq!(local_head, 0);
        assert_eq!(syncing, network_head > local_head);

        let latest = api.zero_get_latest_block(None).expect("latest block");
        assert_eq!(latest.get("number").and_then(|v| v.as_str()), Some("0x0"));
        set_global_synced_height(0);
    }

    #[test]
    fn test_zero_get_block_by_number_and_range() {
        let api = build_test_api_with_persistent_compute();

        assert_eq!(
            mine_one_block(&api, 101, "range-miner")["accepted"].as_bool(),
            Some(true)
        );
        assert_eq!(
            mine_one_block(&api, 102, "range-miner")["accepted"].as_bool(),
            Some(true)
        );
        assert_eq!(
            mine_one_block(&api, 103, "range-miner")["accepted"].as_bool(),
            Some(true)
        );

        let block_2 = api
            .zero_get_block_by_number(Some(vec![serde_json::json!("0x2")]))
            .expect("block by number should succeed");
        assert_eq!(block_2.get("number").and_then(|v| v.as_str()), Some("0x2"));

        let range = api
            .zero_get_blocks_range(Some(vec![serde_json::json!({
                "from": "0x1",
                "to": "0x3",
                "limit": 10
            })]))
            .expect("range should succeed");
        let items = range
            .get("items")
            .and_then(|v| v.as_array())
            .expect("items missing");
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].get("number").and_then(|v| v.as_str()), Some("0x3"));
        assert_eq!(items[1].get("number").and_then(|v| v.as_str()), Some("0x2"));
        assert_eq!(items[2].get("number").and_then(|v| v.as_str()), Some("0x1"));
    }

    #[test]
    fn test_zero_list_compute_tx_results_returns_paginated_items() {
        let api = build_test_api_with_persistent_compute();
        let witness_sig = format!(
            "0x{}",
            hex::encode(Signature::new([9; 32], [10; 32], 27).as_bytes())
        );

        let tx_a = canonicalize_compute_tx_id(serde_json::json!({
            "tx_id": format!("0x{}", hex::encode([0x21u8; 32])),
            "domain_id": 0,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Mint",
            "nonce": 1,
            "input_set": [],
            "read_set": [],
            "output_proposals": [
                {
                    "output_id": format!("0x{}", hex::encode([0x31u8; 32])),
                    "object_id": format!("0x{}", hex::encode([0x41u8; 32])),
                    "domain_id": 0,
                    "kind": "State",
                    "owner": { "type": "Shared" },
                    "predecessor": null,
                    "version": 1,
                    "state": "0x01",
                    "logic": null
                }
            ],
            "payload": "0x",
            "deadline_unix_secs": null,
            "witness": {"signatures": [witness_sig.clone()], "threshold": 1}
        }));
        let tx_b = canonicalize_compute_tx_id(serde_json::json!({
            "tx_id": format!("0x{}", hex::encode([0x22u8; 32])),
            "domain_id": 0,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Mint",
            "nonce": 2,
            "input_set": [],
            "read_set": [],
            "output_proposals": [
                {
                    "output_id": format!("0x{}", hex::encode([0x32u8; 32])),
                    "object_id": format!("0x{}", hex::encode([0x42u8; 32])),
                    "domain_id": 0,
                    "kind": "State",
                    "owner": { "type": "Shared" },
                    "predecessor": null,
                    "version": 1,
                    "state": "0x02",
                    "logic": null
                }
            ],
            "payload": "0x",
            "deadline_unix_secs": null,
            "witness": {"signatures": [witness_sig], "threshold": 1}
        }));

        let _ = api
            .zero_submit_compute_tx(Some(vec![tx_a]))
            .expect("submit compute tx a");
        let _ = api
            .zero_submit_compute_tx(Some(vec![tx_b]))
            .expect("submit compute tx b");

        let listed = api
            .zero_list_compute_tx_results(Some(vec![serde_json::json!({
                "page": 1,
                "limit": 1
            })]))
            .expect("list tx results");
        assert_eq!(listed.get("page").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(listed.get("limit").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(listed.get("total").and_then(|v| v.as_u64()), Some(2));
        assert_eq!(listed.get("has_more").and_then(|v| v.as_bool()), Some(true));
        let items = listed
            .get("items")
            .and_then(|v| v.as_array())
            .expect("items missing");
        assert_eq!(items.len(), 1);
        let tx_id = items[0]
            .get("tx_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert!(tx_id.starts_with("0x"));
    }

    #[test]
    fn test_zero_transfer_moves_balance_between_addresses() {
        let account_manager = Arc::new(InMemoryAccountManager::new());
        let tx_pool = Arc::new(TransactionPool::new(
            TxPoolConfig::default(),
            account_manager,
        ));
        let state_db = Arc::new(StateDb::new(Hash::zero()));
        let db = Arc::new(MemDatabase::new());
        let persistent_store = Arc::new(ComputeStore::new(db));

        let domains = Arc::new(InMemoryDomainRegistry::new());
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });
        let api = RpcApi::with_persistent_compute(
            RpcConfig::default(),
            state_db,
            tx_pool,
            persistent_store,
            domains,
        );
        let from = parse_address("ZER0x1111111111111111111111111111111111111111").unwrap();
        let to = parse_address("ZER0x2222222222222222222222222222222222222222").unwrap();

        let mut from_account = Account {
            address: from,
            state: AccountState::Active,
            created_at: 1,
            updated_at: 1,
            ..Account::default()
        };
        from_account.balance = U256::from(9_000);
        api.state_db.insert_account(from, from_account);

        let tx_hash = api
            .zero_transfer(Some(vec![serde_json::json!({
                "from": "ZER0x1111111111111111111111111111111111111111",
                "to": "ZER0x2222222222222222222222222222222222222222",
                "value": "0x3e8"
            })]))
            .expect("send tx should succeed");
        assert!(tx_hash.as_str().unwrap_or_default().starts_with("0x"));

        assert_eq!(api.state_db.get_balance(&from).as_u64(), 8_000);
        assert_eq!(api.state_db.get_nonce(&from), 1);
        assert_eq!(api.state_db.get_balance(&to).as_u64(), 1_000);
    }

    #[test]
    fn test_zero_transfer_rejects_missing_from_account() {
        let api = build_test_api_with_persistent_compute();
        let err = api
            .zero_transfer(Some(vec![serde_json::json!({
                "from": "ZER0x1111111111111111111111111111111111111111",
                "to": "ZER0x2222222222222222222222222222222222222222",
                "value": "0x1"
            })]))
            .expect_err("from account should be required");
        assert_eq!(err.code, -32602);
    }

    #[test]
    fn test_zero_transfer_query_and_address_history() {
        let account_manager = Arc::new(InMemoryAccountManager::new());
        let tx_pool = Arc::new(TransactionPool::new(
            TxPoolConfig::default(),
            account_manager,
        ));
        let state_db = Arc::new(StateDb::new(Hash::zero()));
        let db = Arc::new(MemDatabase::new());
        let persistent_store = Arc::new(ComputeStore::new(db));

        let domains = Arc::new(InMemoryDomainRegistry::new());
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });
        let api = RpcApi::with_persistent_compute(
            RpcConfig::default(),
            state_db,
            tx_pool,
            persistent_store,
            domains,
        );
        let from = parse_address("ZER0x1111111111111111111111111111111111111111").unwrap();
        let to = parse_address("ZER0x2222222222222222222222222222222222222222").unwrap();

        let mut from_account = Account {
            address: from,
            state: AccountState::Active,
            created_at: 1,
            updated_at: 1,
            ..Account::default()
        };
        from_account.balance = U256::from(9_000);
        api.state_db.insert_account(from, from_account);

        let tx_hash = api
            .zero_transfer(Some(vec![serde_json::json!({
                "from": "ZER0x1111111111111111111111111111111111111111",
                "to": "ZER0x2222222222222222222222222222222222222222",
                "value": "0x3e8"
            })]))
            .expect("transfer should succeed")
            .as_str()
            .unwrap()
            .to_string();

        let tx_detail = api
            .zero_get_transaction_by_hash(Some(vec![serde_json::json!(tx_hash.clone())]))
            .expect("tx detail should succeed");
        assert_eq!(
            tx_detail.get("kind").and_then(|v| v.as_str()),
            Some("transfer")
        );
        assert_eq!(
            tx_detail.get("from").and_then(|v| v.as_str()),
            Some("ZER0x1111111111111111111111111111111111111111")
        );

        let list = api
            .zero_list_transactions(Some(vec![serde_json::json!({
                "page": 1,
                "limit": 10,
                "kind": "transfer"
            })]))
            .expect("list should succeed");
        assert_eq!(list.get("total").and_then(|v| v.as_u64()), Some(1));

        let by_addr = api
            .zero_get_transactions_by_address(Some(vec![serde_json::json!({
                "address": "ZER0x1111111111111111111111111111111111111111",
                "page": 1,
                "limit": 10
            })]))
            .expect("address txs should succeed");
        assert_eq!(
            by_addr.get("address").and_then(|v| v.as_str()),
            Some("ZER0x1111111111111111111111111111111111111111")
        );
        assert_eq!(by_addr.get("total").and_then(|v| v.as_u64()), Some(1));

        // Ensure recipient side can also query same transfer.
        let by_to = api
            .zero_get_transactions_by_address(Some(vec![serde_json::json!(
                "ZER0x2222222222222222222222222222222222222222"
            )]))
            .expect("recipient txs should succeed");
        assert_eq!(by_to.get("total").and_then(|v| v.as_u64()), Some(1));
    }

    #[test]
    fn test_zero_submit_work_rejects_stale_work_id() {
        let api = build_test_api_with_persistent_compute();
        let submit = api.zero_submit_work(Some(vec![serde_json::json!({
            "work_id": "work-stale-1",
            "nonce": 1,
            "hash_hex": "0x00",
            "miner": "test-miner"
        })]));
        let err = submit.expect_err("stale work id should error");
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "Invalid params");
    }

    #[test]
    fn test_zero_submit_work_replay_is_rejected_after_accept() {
        let api = build_test_api_with_persistent_compute();
        let work = api.zero_get_work(None).expect("work should be available");
        let work_id = work["work_id"].as_str().unwrap().to_string();
        let prev_hash = work["prev_hash"].as_str().unwrap().to_string();
        let height = work["height"].as_u64().unwrap();

        {
            let mut jobs = api.mining_jobs.write();
            if let Some(job) = jobs.get_mut(&work_id) {
                job.target_leading_zero_bytes = 0;
            }
        }

        let prev_hash_bytes =
            hex::decode(prev_hash.strip_prefix("0x").unwrap_or(&prev_hash)).unwrap();
        let nonce = 77u64;
        let mut data = Vec::new();
        data.extend_from_slice(&prev_hash_bytes);
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&nonce.to_be_bytes());
        let digest = zerocore::crypto::keccak256(&data);
        let payload = serde_json::json!({
            "work_id": work_id,
            "nonce": nonce,
            "hash_hex": format!("0x{}", hex::encode(digest)),
            "miner": "test-miner"
        });

        let first = api
            .zero_submit_work(Some(vec![payload.clone()]))
            .expect("first submit should succeed");
        assert_eq!(first["accepted"].as_bool(), Some(true));

        let second = api.zero_submit_work(Some(vec![payload]));
        let err = second.expect_err("replay should be rejected as stale work_id");
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "Invalid params");
    }

    #[test]
    fn test_zero_submit_work_rejects_oversized_miner_label() {
        let api = build_test_api_with_persistent_compute();
        let work = api.zero_get_work(None).expect("work should be available");
        let work_id = work["work_id"].as_str().unwrap().to_string();
        let prev_hash = work["prev_hash"].as_str().unwrap().to_string();
        let height = work["height"].as_u64().unwrap();

        {
            let mut jobs = api.mining_jobs.write();
            if let Some(job) = jobs.get_mut(&work_id) {
                job.target_leading_zero_bytes = 0;
            }
        }

        let prev_hash_bytes =
            hex::decode(prev_hash.strip_prefix("0x").unwrap_or(&prev_hash)).unwrap();
        let nonce = 88u64;
        let mut data = Vec::new();
        data.extend_from_slice(&prev_hash_bytes);
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&nonce.to_be_bytes());
        let digest = zerocore::crypto::keccak256(&data);
        let too_long_miner = "m".repeat(MAX_MINER_EXTRA_DATA_BYTES + 1);

        let submit = api
            .zero_submit_work(Some(vec![serde_json::json!({
                "work_id": work_id,
                "nonce": nonce,
                "hash_hex": format!("0x{}", hex::encode(digest)),
                "miner": too_long_miner
            })]))
            .expect("submit should return rejection");

        assert_eq!(submit["accepted"].as_bool(), Some(false));
        assert_eq!(submit["reason"].as_str(), Some("invalid_miner_label"));
    }

    #[test]
    fn test_zero_import_block_rejects_parent_mismatch() {
        let api = build_test_api_with_persistent_compute();

        // Mine one block first.
        let first = api.zero_get_work(None).expect("get work");
        let work_id = first["work_id"].as_str().unwrap().to_string();
        let prev_hash = first["prev_hash"].as_str().unwrap().to_string();
        let height = first["height"].as_u64().unwrap();
        {
            let mut jobs = api.mining_jobs.write();
            if let Some(job) = jobs.get_mut(&work_id) {
                job.target_leading_zero_bytes = 0;
            }
        }
        let prev_hash_bytes =
            hex::decode(prev_hash.strip_prefix("0x").unwrap_or(&prev_hash)).unwrap();
        let nonce = 9u64;
        let mut data = Vec::new();
        data.extend_from_slice(&prev_hash_bytes);
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&nonce.to_be_bytes());
        let digest = zerocore::crypto::keccak256(&data);
        let mined = api
            .zero_submit_work(Some(vec![serde_json::json!({
                "work_id": work_id,
                "nonce": nonce,
                "hash_hex": format!("0x{}", hex::encode(digest)),
                "miner": "test-miner"
            })]))
            .expect("submit work");
        assert_eq!(mined["accepted"].as_bool(), Some(true));

        // Import block with wrong parent hash should be rejected.
        let latest = api.zero_get_latest_block(None).expect("latest block");
        let bad_import = api
            .zero_import_block(Some(vec![serde_json::json!({
                "hash": "0x1111111111111111111111111111111111111111111111111111111111111111",
                "parent_hash": "0x2222222222222222222222222222222222222222222222222222222222222222",
                "number": "0x2",
                "timestamp": latest["timestamp"].as_u64().unwrap_or(1) + 1,
                "difficulty": latest["difficulty"].as_str().unwrap_or("0x1"),
                "nonce": 1,
                "coinbase": latest["coinbase"].as_str().unwrap_or("ZER0x0000000000000000000000000000000000000000"),
                "mix_hash": latest["mix_hash"].as_str().unwrap_or("0x0000000000000000000000000000000000000000000000000000000000000000"),
                "extra_data": "0x"
            })]))
            .expect("import call should return result");
        assert_eq!(bad_import["imported"].as_bool(), Some(false));
        assert_eq!(bad_import["reason"].as_str(), Some("parent_mismatch"));
    }

    #[test]
    fn test_zero_get_metrics_contains_rpc_and_mining_counters() {
        let api = build_test_api_with_persistent_compute();

        let _ = futures::executor::block_on(api.handle_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "zero_getWork".to_string(),
            params: Some(vec![]),
            id: serde_json::json!(1),
        }));
        let _ = futures::executor::block_on(api.handle_request(JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method: "zero_submitWork".to_string(),
            params: Some(vec![serde_json::json!({
                "work_id": "work-stale-2",
                "nonce": 1,
                "hash_hex": "0x00",
                "miner": "metric-miner"
            })]),
            id: serde_json::json!(2),
        }));

        let metrics = api.zero_get_metrics(None).expect("metrics should render");
        let text = metrics
            .get("text")
            .and_then(|v| v.as_str())
            .expect("metrics text missing");

        assert!(text.contains("zero_rpc_method_calls_total"));
        assert!(text.contains("zero_rpc_method_errors_total"));
    }

    #[test]
    fn test_zero_peers_returns_array() {
        let api = build_test_api_with_compute();
        let peers = api.zero_peers(None).expect("zero_peers should succeed");
        assert!(peers.is_array());
    }

    #[test]
    fn test_zero_peers_rejects_params() {
        let api = build_test_api_with_compute();
        let err = api
            .zero_peers(Some(vec![serde_json::json!(1)]))
            .expect_err("zero_peers should reject params");
        assert_eq!(err.code, -32602);
    }

    #[test]
    fn test_is_authorized_supports_bearer_and_header_token() {
        let mut headers = HeaderMap::new();
        assert!(!is_authorized(&headers, Some("abc")));

        headers.insert(
            "authorization",
            axum::http::HeaderValue::from_static("Bearer abc"),
        );
        assert!(is_authorized(&headers, Some("abc")));

        headers.remove("authorization");
        headers.insert("x-zero-token", axum::http::HeaderValue::from_static("abc"));
        assert!(is_authorized(&headers, Some("abc")));
        assert!(!is_authorized(&headers, Some("def")));
    }

    #[test]
    fn test_rate_limiter_enforces_budget() {
        let cfg = RpcConfig {
            rate_limit_per_minute: 2,
            ..RpcConfig::default()
        };
        let limiter = RpcSecurityContext::new(&cfg);
        assert!(limiter.allow_request("127.0.0.1"));
        assert!(limiter.allow_request("127.0.0.1"));
        assert!(!limiter.allow_request("127.0.0.1"));
        assert!(limiter.allow_request("10.0.0.1"));
    }

    #[test]
    fn test_parse_address() {
        let addr = parse_address("ZER0x0000000000000000000000000000000000000001").unwrap();
        assert!(!addr.is_zero());
    }

    #[test]
    fn test_parse_address_rejects_0x_prefix() {
        let err = parse_address("0x0000000000000000000000000000000000000001");
        assert!(err.is_err());
    }

    #[test]
    fn test_parse_address_rejects_native1() {
        let err = parse_address("native10000000000000000000000000000000000000001");
        assert!(err.is_err());
    }

    #[test]
    fn test_parse_address_accepts_case_insensitive_prefix() {
        let addr = parse_address("zer0x1111111111111111111111111111111111111111");
        assert!(addr.is_ok());
    }

    #[test]
    fn test_format_zero_address_prefix() {
        let addr = parse_address("ZER0x1111111111111111111111111111111111111111").unwrap();
        let formatted = format_zero_address(addr);
        assert!(formatted.starts_with("ZER0x"));
        assert_eq!(formatted.len(), 45);
    }

    #[test]
    fn test_parse_hash() {
        let hash = parse_hash("0x0000000000000000000000000000000000000000000000000000000000000001")
            .unwrap();
        assert!(!hash.is_zero());
    }

    #[test]
    fn test_format_u256_hex_preserves_values_above_u64() {
        let value = U256::from_big_endian(&[0x01, 0, 0, 0, 0, 0, 0, 0, 0x01]); // 2^64 + 1
        assert_eq!(format_u256_hex(value), "0x10000000000000001");
    }

    #[test]
    fn test_zero_get_output_object_domain() {
        let api = build_test_api_with_compute();

        let output = ObjectOutput {
            output_id: OutputId(Hash::from_bytes([11; 32])),
            object_id: ObjectId(Hash::from_bytes([22; 32])),
            version: Version(1),
            domain_id: DomainId(0),
            kind: ObjectKind::State,
            owner: Ownership::Shared,
            predecessor: None,
            state: vec![0xAA, 0xBB],
            state_root: None,
            resources: vec![],
            lock: Script::default(),
            logic: None,
            created_at: 0,
            ttl: None,
            rent_reserve: None,
            flags: 0,
            extensions: vec![],
            spent: false,
        };

        api.compute_store.insert_output(output).unwrap();

        let output_id_hex = format!("0x{}", hex::encode([11u8; 32]));
        let out_value = api
            .zero_get_output(Some(vec![serde_json::Value::String(output_id_hex)]))
            .unwrap();
        assert!(out_value.is_object());

        let object_id_hex = format!("0x{}", hex::encode([22u8; 32]));
        let obj_value = api
            .zero_get_object(Some(vec![serde_json::Value::String(object_id_hex)]))
            .unwrap();
        assert!(obj_value.is_object());

        let domain_value = api
            .zero_get_domain(Some(vec![serde_json::Value::from(0u64)]))
            .unwrap();
        assert_eq!(
            domain_value.get("domain_id").and_then(|v| v.as_u64()),
            Some(0)
        );
    }

    #[test]
    fn test_zero_simulate_and_submit_compute_tx() {
        let api = build_test_api_with_compute();

        let witness_sig = format!(
            "0x{}",
            hex::encode(Signature::new([1; 32], [2; 32], 27).as_bytes())
        );

        let tx = canonicalize_compute_tx_id(serde_json::json!({
            "tx_id": format!("0x{}", hex::encode([0x55u8; 32])),
            "domain_id": 0,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Mint",
            "nonce": 1,
            "input_set": [],
            "read_set": [],
            "output_proposals": [
                {
                    "output_id": format!("0x{}", hex::encode([0x66u8; 32])),
                    "object_id": format!("0x{}", hex::encode([0x77u8; 32])),
                    "domain_id": 0,
                    "kind": "State",
                    "owner": { "type": "Shared" },
                    "predecessor": null,
                    "version": 1,
                    "state": "0x010203",
                    "logic": null
                }
            ],
            "payload": "0x",
            "deadline_unix_secs": null,
            "witness": {
                "signatures": [witness_sig],
                "threshold": 1
            }
        }));

        let sim = api
            .zero_simulate_compute_tx(Some(vec![tx.clone()]))
            .expect("simulation should succeed");
        assert_eq!(sim.get("ok").and_then(|v| v.as_bool()), Some(true));

        let submit = api
            .zero_submit_compute_tx(Some(vec![tx.clone()]))
            .expect("submit should succeed");
        assert_eq!(submit.get("ok").and_then(|v| v.as_bool()), Some(true));

        let out = api
            .zero_get_output(Some(vec![serde_json::Value::String(format!(
                "0x{}",
                hex::encode([0x66u8; 32])
            ))]))
            .expect("output query should succeed");
        assert!(out.is_object());

        let dup = api
            .zero_submit_compute_tx(Some(vec![tx]))
            .expect("duplicate submit should return cached result");
        assert_eq!(dup.get("duplicate").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_zero_simulate_returns_structured_domain_error() {
        let api = build_test_api_with_compute();
        let tx = serde_json::json!({
            "tx_id": format!("0x{}", hex::encode([0x99u8; 32])),
            "domain_id": 9,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Mint",
            "nonce": 2,
            "input_set": [],
            "read_set": [],
            "output_proposals": [{
                "output_id": format!("0x{}", hex::encode([0x98u8; 32])),
                "object_id": format!("0x{}", hex::encode([0x97u8; 32])),
                "domain_id": 9,
                "kind": "State",
                "owner": { "type": "Shared" },
                "predecessor": null,
                "version": 1,
                "state": "0x01",
                "logic": null
            }],
            "payload": "0x",
            "deadline_unix_secs": null,
            "witness": {"signatures": [format!("0x{}", hex::encode(Signature::new([1; 32], [2; 32], 27).as_bytes()))], "threshold": 1}
        });

        let sim = api
            .zero_simulate_compute_tx(Some(vec![tx]))
            .expect("simulate should return result object");
        assert_eq!(sim.get("ok").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            sim.get("error")
                .and_then(|v| v.get("category"))
                .and_then(|v| v.as_str()),
            Some("domain")
        );
        assert_eq!(
            sim.get("error")
                .and_then(|v| v.get("numeric_code"))
                .and_then(|v| v.as_i64()),
            Some(1001)
        );
    }

    #[test]
    fn test_zero_simulate_returns_invalid_signature_error() {
        let api = build_test_api_with_compute();

        let tx = canonicalize_compute_tx_id(serde_json::json!({
            "tx_id": format!("0x{}", hex::encode([0xD1u8; 32])),
            "domain_id": 0,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Transfer",
            "nonce": 2,
            "input_set": [format!("0x{}", hex::encode([0xD2u8; 32]))],
            "read_set": [],
            "output_proposals": [{
                "output_id": format!("0x{}", hex::encode([0xD3u8; 32])),
                "object_id": format!("0x{}", hex::encode([0xD4u8; 32])),
                "domain_id": 0,
                "kind": "Asset",
                "owner": { "type": "Address", "address": "ZER0x1111111111111111111111111111111111111111" },
                "predecessor": null,
                "version": 1,
                "state": "0x01",
                "logic": null
            }],
            "payload": "0x",
            "deadline_unix_secs": null,
            "witness": {"signatures": ["0x0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"], "threshold": 1}
        }));

        // Prepare input object for transfer validation.
        let input = ObjectOutput {
            output_id: OutputId(Hash::from_bytes([0xD2; 32])),
            object_id: ObjectId(Hash::from_bytes([0xE1; 32])),
            version: Version(1),
            domain_id: DomainId(0),
            kind: ObjectKind::Asset,
            owner: Ownership::Address(
                Address::from_hex("0x1111111111111111111111111111111111111111").unwrap(),
            ),
            predecessor: None,
            state: vec![1],
            state_root: None,
            resources: vec![],
            lock: Script::default(),
            logic: None,
            created_at: 0,
            ttl: None,
            rent_reserve: None,
            flags: 0,
            extensions: vec![],
            spent: false,
        };
        api.compute_store.insert_output(input).unwrap();

        let sim = api
            .zero_simulate_compute_tx(Some(vec![tx]))
            .expect("simulate should return result object");
        assert_eq!(sim.get("ok").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            sim.get("error")
                .and_then(|v| v.get("code"))
                .and_then(|v| v.as_str()),
            Some("invalid_signature")
        );
        assert_eq!(
            sim.get("error")
                .and_then(|v| v.get("numeric_code"))
                .and_then(|v| v.as_i64()),
            Some(3003)
        );
    }

    #[test]
    fn test_zero_simulate_returns_owner_mismatch_error() {
        let api = build_test_api_with_compute();
        let signer = PrivateKey::from_bytes([3u8; 32]).unwrap();

        let input = ObjectOutput {
            output_id: OutputId(Hash::from_bytes([0xF2; 32])),
            object_id: ObjectId(Hash::from_bytes([0xF3; 32])),
            version: Version(1),
            domain_id: DomainId(0),
            kind: ObjectKind::Asset,
            owner: Ownership::Address(
                Address::from_hex("0x2222222222222222222222222222222222222222").unwrap(),
            ),
            predecessor: None,
            state: vec![1],
            state_root: None,
            resources: vec![],
            lock: Script::default(),
            logic: None,
            created_at: 0,
            ttl: None,
            rent_reserve: None,
            flags: 0,
            extensions: vec![],
            spent: false,
        };
        api.compute_store.insert_output(input).unwrap();

        let mut tx = serde_json::json!({
            "tx_id": format!("0x{}", hex::encode([0xF1u8; 32])),
            "domain_id": 0,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Transfer",
            "nonce": 3,
            "input_set": [format!("0x{}", hex::encode([0xF2u8; 32]))],
            "read_set": [],
            "output_proposals": [{
                "output_id": format!("0x{}", hex::encode([0xF4u8; 32])),
                "object_id": format!("0x{}", hex::encode([0xF3u8; 32])),
                "domain_id": 0,
                "kind": "Asset",
                "owner": { "type": "Address", "address": "ZER0x2222222222222222222222222222222222222222" },
                "predecessor": format!("0x{}", hex::encode([0xF2u8; 32])),
                "version": 2,
                "state": "0x01",
                "logic": null
            }],
            "payload": "0x",
            "deadline_unix_secs": null,
            "witness": {"signatures": [], "threshold": 1}
        });

        let parsed = parse_compute_tx(tx.clone()).expect("tx should parse");
        let sig = signer.sign(&parsed.signing_preimage());
        tx["witness"]["signatures"] =
            serde_json::json!([format!("0x{}", hex::encode(sig.as_bytes()))]);
        let tx = canonicalize_compute_tx_id(tx);

        let sim = api
            .zero_simulate_compute_tx(Some(vec![tx]))
            .expect("simulate should return result object");
        assert_eq!(sim.get("ok").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            sim.get("error")
                .and_then(|v| v.get("code"))
                .and_then(|v| v.as_str()),
            Some("signature_owner_mismatch")
        );
        assert_eq!(
            sim.get("error")
                .and_then(|v| v.get("numeric_code"))
                .and_then(|v| v.as_i64()),
            Some(3004)
        );
    }

    #[test]
    fn test_zero_simulate_returns_tx_id_mismatch_error() {
        let api = build_test_api_with_compute();
        let owner_key = PrivateKey::from_bytes([9u8; 32]).unwrap();
        let owner_addr = Address::from_public_key(&owner_key.public_key());

        let input = ObjectOutput {
            output_id: OutputId(Hash::from_bytes([0xAB; 32])),
            object_id: ObjectId(Hash::from_bytes([0xAC; 32])),
            version: Version(1),
            domain_id: DomainId(0),
            kind: ObjectKind::Asset,
            owner: Ownership::Address(owner_addr),
            predecessor: None,
            state: vec![1],
            state_root: None,
            resources: vec![],
            lock: Script::default(),
            logic: None,
            created_at: 0,
            ttl: None,
            rent_reserve: None,
            flags: 0,
            extensions: vec![],
            spent: false,
        };
        api.compute_store.insert_output(input).unwrap();

        let mut tx = serde_json::json!({
            "tx_id": format!("0x{}", hex::encode([0xADu8; 32])),
            "domain_id": 0,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Transfer",
            "nonce": 4,
            "input_set": [format!("0x{}", hex::encode([0xABu8; 32]))],
            "read_set": [],
            "output_proposals": [{
                "output_id": format!("0x{}", hex::encode([0xAEu8; 32])),
                "object_id": format!("0x{}", hex::encode([0xACu8; 32])),
                "domain_id": 0,
                "kind": "Asset",
                "owner": { "type": "Address", "address": format_zero_address(owner_addr) },
                "predecessor": format!("0x{}", hex::encode([0xABu8; 32])),
                "version": 2,
                "state": "0x02",
                "logic": null
            }],
            "payload": "0x1234",
            "deadline_unix_secs": 1900000000u64,
            "witness": {"signatures": [], "threshold": 1}
        });

        let parsed = parse_compute_tx(tx.clone()).expect("tx should parse");
        let sig = owner_key.sign(&parsed.signing_preimage());
        tx["witness"]["signatures"] =
            serde_json::json!([format!("0x{}", hex::encode(sig.as_bytes()))]);

        let sim = api
            .zero_simulate_compute_tx(Some(vec![tx]))
            .expect("simulate should return result object");
        assert_eq!(sim.get("ok").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(
            sim.get("error")
                .and_then(|v| v.get("code"))
                .and_then(|v| v.as_str()),
            Some("tx_id_mismatch")
        );
        assert_eq!(
            sim.get("error")
                .and_then(|v| v.get("numeric_code"))
                .and_then(|v| v.as_i64()),
            Some(3005)
        );
    }

    #[test]
    fn test_parse_compute_tx_requires_witness() {
        let tx = serde_json::json!({
            "tx_id": format!("0x{}", hex::encode([0x11u8; 32])),
            "domain_id": 0,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Mint",
            "nonce": 3,
            "input_set": [],
            "read_set": [],
            "output_proposals": [],
            "payload": "0x",
            "deadline_unix_secs": null
        });

        let err = parse_compute_tx(tx).expect_err("witness should be required");
        assert_eq!(err.code, -32602);
    }

    #[test]
    fn test_zero_get_compute_tx_result_with_persistent_store() {
        let api = build_test_api_with_persistent_compute();
        let sig_hex = format!(
            "0x{}",
            hex::encode(Signature::new([1; 32], [2; 32], 27).as_bytes())
        );

        let tx = serde_json::json!({
            "tx_id": format!("0x{}", hex::encode([0xA1u8; 32])),
            "domain_id": 0,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Mint",
            "nonce": 4,
            "input_set": [],
            "read_set": [],
            "output_proposals": [{
                "output_id": format!("0x{}", hex::encode([0xA2u8; 32])),
                "object_id": format!("0x{}", hex::encode([0xA3u8; 32])),
                "domain_id": 0,
                "kind": "State",
                "owner": { "type": "Shared" },
                "predecessor": null,
                "version": 1,
                "state": "0x01",
                "logic": null
            }],
            "payload": "0x",
            "deadline_unix_secs": null,
            "witness": {"signatures": [sig_hex], "threshold": 1}
        });

        let tx = canonicalize_compute_tx_id(tx);
        let tx_id_hex = tx
            .get("tx_id")
            .and_then(|v| v.as_str())
            .expect("tx_id must exist after canonicalization")
            .to_string();

        let submit = api
            .zero_submit_compute_tx(Some(vec![tx]))
            .expect("submit should succeed");
        assert_eq!(submit.get("ok").and_then(|v| v.as_bool()), Some(true));

        let got = api
            .zero_get_compute_tx_result(Some(vec![serde_json::Value::String(tx_id_hex)]))
            .expect("get tx result should succeed");
        assert_eq!(got.get("ok").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_get_compute_tx_result_returns_null_when_missing() {
        let api = build_test_api_with_persistent_compute();
        let missing = api
            .zero_get_compute_tx_result(Some(vec![serde_json::Value::String(format!(
                "0x{}",
                hex::encode([0xFEu8; 32])
            ))]))
            .expect("query should not fail");
        assert!(missing.is_null());
    }

    #[test]
    fn test_build_compute_backend_mem() {
        let cfg = RpcConfig {
            compute_backend: ComputeBackend::Mem,
            ..RpcConfig::default()
        };
        let db = build_compute_kv_backend(&cfg);
        db.put(b"k", b"v").unwrap();
        assert_eq!(db.get(b"k").unwrap(), Some(b"v".to_vec()));
    }

    #[test]
    fn test_rpc_config_validate_rejects_empty_path_for_file_backend() {
        let cfg = RpcConfig {
            compute_backend: ComputeBackend::RocksDb,
            compute_db_path: "   ".to_string(),
            ..RpcConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_try_new_returns_error_for_invalid_config() {
        let cfg = RpcConfig {
            compute_backend: ComputeBackend::Redb,
            compute_db_path: "".to_string(),
            ..RpcConfig::default()
        };
        let err = match RpcServer::try_new(cfg) {
            Ok(_) => panic!("invalid config should fail"),
            Err(err) => err,
        };
        match err {
            crate::ApiError::InvalidRequest(_) => {}
            other => panic!("unexpected error: {other}"),
        }
    }
}
