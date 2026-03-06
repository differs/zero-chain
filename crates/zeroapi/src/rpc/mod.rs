//! JSON-RPC Server Implementation

use zerocore::crypto::{Address, Hash};
use zerocore::account::{InMemoryAccountManager, U256};
use zerocore::transaction::{pool::TxPoolConfig, SignedTransaction, TransactionPool};
use zerocore::block::{Block, BlockHeader};
use zerocore::state::StateDb;
use zerocore::compute::{
    BasicTxExecutor, Command, ComputeTx, DefaultAuthorizationPolicy, DomainConfig, DomainId,
    InMemoryDomainRegistry, InMemoryObjectStore, NoopResourcePolicy, ObjectId, ObjectKind,
    ObjectOutput, ObjectStore, OutputId, OutputProposal, Ownership, SignatureScheme, TxSignature,
    TxWitness, Version,
};
use zerocore::compute::domain::DomainRegistry;
use zerocore::crypto::{PrivateKey, Signature};
use zerostore::ComputeStore;
use zerostore::db::{KeyValueDB, MemDatabase, RedbDatabase, RocksDb};
use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use axum::{Router, routing::post, Json, extract::State};
use tower_http::cors::{CorsLayer, Any};

/// RPC configuration
#[derive(Clone, Debug)]
#[derive(Serialize, Deserialize)]
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
}

/// Persistent backend for compute storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputeBackend {
    /// In-memory backend.
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

impl Default for ComputeBackend {
    fn default() -> Self {
        Self::Mem
    }
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".to_string(),
            port: 8545,
            max_connections: 100,
            max_request_size: 15 * 1024 * 1024,  // 15MB
            modules: vec!["eth".into(), "net".into(), "web3".into()],
            compute_backend: ComputeBackend::Mem,
            compute_db_path: "./data/compute-db".to_string(),
        }
    }
}

impl RpcConfig {
    /// Validates RPC configuration consistency.
    pub fn validate(&self) -> std::result::Result<(), String> {
        match self.compute_backend {
            ComputeBackend::Mem => Ok(()),
            ComputeBackend::RocksDb | ComputeBackend::Redb => {
                if self.compute_db_path.trim().is_empty() {
                    return Err(format!(
                        "compute_db_path cannot be empty when compute_backend={}"
                        ,self.compute_backend.as_str()
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
    compute_store: Arc<dyn ObjectStore>,
    domain_registry: Arc<InMemoryDomainRegistry>,
    submitted_compute_results: RwLock<HashMap<Hash, serde_json::Value>>,
    persistent_compute_store: Option<Arc<ComputeStore>>,
}

impl RpcApi {
    pub fn new(
        config: RpcConfig,
        state_db: Arc<StateDb>,
        tx_pool: Arc<TransactionPool>,
    ) -> Self {
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
            compute_store: Arc::new(InMemoryObjectStore::new()),
            domain_registry,
            submitted_compute_results: RwLock::new(HashMap::new()),
            persistent_compute_store: None,
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
            compute_store,
            domain_registry,
            submitted_compute_results: RwLock::new(HashMap::new()),
            persistent_compute_store: None,
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
            compute_store: compute_store.clone(),
            domain_registry,
            submitted_compute_results: RwLock::new(HashMap::new()),
            persistent_compute_store: Some(compute_store),
        }
    }
    
    /// Handle RPC request
    pub async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let result = self.dispatch_method(&request.method, request.params).await;
        
        match result {
            Ok(value) => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                result: Some(value),
                error: None,
                id: request.id,
            },
            Err(error) => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(error),
                id: request.id,
            },
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
            
            // eth_* methods
            "eth_protocolVersion" => self.eth_protocol_version(params),
            "eth_blockNumber" => self.eth_block_number(params),
            "eth_getBalance" => self.eth_get_balance(params),
            "eth_getStorageAt" => self.eth_get_storage_at(params),
            "eth_getTransactionCount" => self.eth_get_transaction_count(params),
            "eth_getBlockByNumber" => self.eth_get_block_by_number(params),
            "eth_getBlockByHash" => self.eth_get_block_by_hash(params),
            "eth_getTransactionByHash" => self.eth_get_transaction_by_hash(params),
            "eth_sendRawTransaction" => self.eth_send_raw_transaction(params),
            "eth_call" => self.eth_call(params),
            "eth_estimateGas" => self.eth_estimate_gas(params),
            "eth_gasPrice" => self.eth_gas_price(params),
            "eth_chainId" => self.eth_chain_id(params),
            "eth_syncing" => self.eth_syncing(params),
            "eth_coinbase" => self.eth_coinbase(params),
            "eth_mining" => self.eth_mining(params),
            "eth_hashrate" => self.eth_hashrate(params),
            "eth_accounts" => self.eth_accounts(params),
            
            // ZeroChain extensions
            "zero_getAccount" => self.zero_get_account(params),
            "zero_getUtxos" => self.zero_get_utxos(params),
            "zero_getObject" => self.zero_get_object(params),
            "zero_getOutput" => self.zero_get_output(params),
            "zero_getDomain" => self.zero_get_domain(params),
            "zero_simulateComputeTx" => self.zero_simulate_compute_tx(params),
            "zero_submitComputeTx" => self.zero_submit_compute_tx(params),
            "zero_getComputeTxResult" => self.zero_get_compute_tx_result(params),
            
            _ => Err(RpcErrorObject::method_not_found(method)),
        }
    }
    
    // ============ web3_* methods ============
    
    fn web3_client_version(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!("ZeroChain/v0.1.0"))
    }
    
    fn web3_sha3(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let data = params.get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing data".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Data must be string".to_string()))?;
        
        let bytes = hex::decode(data.strip_prefix("0x").unwrap_or(data))
            .map_err(|e| RpcErrorObject::invalid_params(format!("Invalid hex: {}", e)))?;
        
        let hash = zerocore::crypto::keccak256(&bytes);
        
        Ok(serde_json::json!(format!("0x{}", hex::encode(hash))))
    }
    
    // ============ net_* methods ============
    
    fn net_version(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!("10086"))
    }
    
    fn net_peer_count(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!("0x0"))
    }
    
    fn net_listening(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!(true))
    }
    
    // ============ eth_* methods ============
    
    fn eth_protocol_version(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!("1"))
    }
    
    fn eth_block_number(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let block = self.latest_block.read();
        let number = block.as_ref().map(|b| b.header.number).unwrap_or(U256::zero());
        Ok(serde_json::json!(format!("0x{:x}", number.as_u64())))
    }
    
    fn eth_get_balance(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        
        let address_str = params.get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing address".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Address must be string".to_string()))?;
        
        let address = parse_address(address_str)?;
        
        let balance = self.state_db.get_balance(&address);
        
        Ok(serde_json::json!(format!("0x{:x}", balance.as_u64())))
    }
    
    fn eth_get_storage_at(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        
        let address_str = params.get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing address".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Address must be string".to_string()))?;
        
        let position_str = params.get(1)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing position".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Position must be string".to_string()))?;
        
        let address = parse_address(address_str)?;
        let position = parse_hash(position_str)?;
        
        let value = self.state_db.get_storage(&address, &position);
        
        Ok(serde_json::json!(format!("0x{}", hex::encode(value.as_bytes()))))
    }
    
    fn eth_get_transaction_count(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        
        let address_str = params.get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing address".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Address must be string".to_string()))?;
        
        let address = parse_address(address_str)?;
        let nonce = self.state_db.get_nonce(&address);
        
        Ok(serde_json::json!(format!("0x{:x}", nonce)))
    }
    
    fn eth_gas_price(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!("0x3b9aca00"))  // 1 Gwei
    }
    
    fn eth_chain_id(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!("0x276e"))  // 10086
    }
    
    fn eth_syncing(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!(false))
    }
    
    fn eth_coinbase(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!("0x0000000000000000000000000000000000000000"))
    }
    
    fn eth_mining(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!(false))
    }
    
    fn eth_hashrate(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!("0x0"))
    }
    
    fn eth_accounts(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!([]))
    }
    
    fn eth_get_block_by_number(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        // Simplified - would fetch from blockchain
        Ok(serde_json::json!(null))
    }
    
    fn eth_get_block_by_hash(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!(null))
    }
    
    fn eth_get_transaction_by_hash(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!(null))
    }
    
    fn eth_send_raw_transaction(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        
        let tx_data = params.get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing transaction data".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Transaction data must be string".to_string()))?;
        
        let tx_bytes = hex::decode(tx_data.strip_prefix("0x").unwrap_or(tx_data))
            .map_err(|e| RpcErrorObject::invalid_params(format!("Invalid hex: {}", e)))?;
        
        // Decode and add to pool
        let tx_hash = Hash::from_bytes(zerocore::crypto::keccak256(&tx_bytes));
        
        Ok(serde_json::json!(format!("0x{}", hex::encode(tx_hash.as_bytes()))))
    }
    
    fn eth_call(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!("0x"))
    }
    
    fn eth_estimate_gas(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!("0x5208"))  // 21000
    }
    
    // ============ ZeroChain extensions ============
    
    fn zero_get_account(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        
        let address_str = params.get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing address".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Address must be string".to_string()))?;
        
        let address = parse_address(address_str)?;
        
        // Would get full account info
        Ok(serde_json::json!({
            "address": address_str,
            "balance": format!("0x{:x}", self.state_db.get_balance(&address).as_u64()),
            "nonce": format!("0x{:x}", self.state_db.get_nonce(&address)),
        }))
    }
    
    fn zero_get_utxos(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!([]))
    }

    fn zero_get_object(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let object_id = params
            .get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing object_id".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("object_id must be string".to_string()))?;

        let object_id = parse_object_id(object_id)?;
        let maybe_output = self.compute_store.get_latest_output_by_object(object_id);
        Ok(serde_json::json!(maybe_output.map(object_output_to_json)))
    }

    fn zero_get_output(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let output_id = params
            .get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing output_id".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("output_id must be string".to_string()))?;

        let output_id = parse_output_id(output_id)?;
        let maybe_output = self.compute_store.get_output(output_id);
        Ok(serde_json::json!(maybe_output.map(object_output_to_json)))
    }

    fn zero_get_domain(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let id = params
            .get(0)
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
            .get(0)
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
            .get(0)
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

        if let Some(existing) = self.submitted_compute_results.read().get(&tx.tx_id.0).cloned() {
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

        let result = serde_json::json!({
            "ok": true,
            "tx_id": format!("0x{}", hex::encode(tx.tx_id.0.as_bytes())),
            "consumed_inputs": report.inputs.len(),
            "read_objects": report.reads.len(),
            "created_outputs": tx.output_proposals.len(),
        });

        if let Some(persistent) = &self.persistent_compute_store {
            let serialized = serde_json::to_string(&result)
                .map_err(|e| RpcErrorObject::internal_error(format!("serialize result failed: {e}")))?;
            persistent
                .put_tx_result(tx.tx_id, &serialized)
                .map_err(|e| RpcErrorObject::internal_error(format!("persist result failed: {e}")))?;
        }

        self.submitted_compute_results
            .write()
            .insert(tx.tx_id.0, result.clone());

        Ok(result)
    }

    fn zero_get_compute_tx_result(
        &self,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or(RpcErrorObject::invalid_params("Missing params".to_string()))?;
        let tx_id_s = params
            .get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing tx_id".to_string()))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("tx_id must be string".to_string()))?;
        let tx_id = zerocore::compute::TxId(parse_hash(tx_id_s)?);

        if let Some(value) = self.submitted_compute_results.read().get(&tx_id.0).cloned() {
            return Ok(value);
        }

        if let Some(persistent) = &self.persistent_compute_store {
            let maybe = persistent
                .get_tx_result(tx_id)
                .map_err(|e| RpcErrorObject::internal_error(format!("load tx result failed: {e}")))?;
            if let Some(raw) = maybe {
                let value = serde_json::from_str::<serde_json::Value>(&raw)
                    .map_err(|e| RpcErrorObject::internal_error(format!("decode tx result failed: {e}")))?;
                return Ok(value);
            }
        }

        Ok(serde_json::Value::Null)
    }
}

fn compute_error_to_json(err: &zerocore::compute::ComputeError) -> serde_json::Value {
    let (numeric_code, code, category) = match err {
        zerocore::compute::ComputeError::DomainNotRegistered(_)
        | zerocore::compute::ComputeError::DomainNotPublic(_)
        | zerocore::compute::ComputeError::DomainMismatch { .. } => (1001, "domain_error", "domain"),
        zerocore::compute::ComputeError::ReadVersionMismatch { .. }
        | zerocore::compute::ComputeError::ReadSetValidationFailed => (2001, "readset_error", "readset"),
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
        zerocore::compute::ComputeError::TxIdMismatch => {
            (3005, "tx_id_mismatch", "authorization")
        }
        zerocore::compute::ComputeError::UnsupportedSignatureScheme => {
            (3006, "unsupported_signature_scheme", "authorization")
        }
        zerocore::compute::ComputeError::InvalidPredecessor
        | zerocore::compute::ComputeError::InvalidVersionProgression
        | zerocore::compute::ComputeError::DuplicateOutputId
        | zerocore::compute::ComputeError::ObjectNotFound(_) => (4001, "state_error", "state"),
        zerocore::compute::ComputeError::ResourcePolicyViolation => (5001, "resource_error", "resource"),
        zerocore::compute::ComputeError::InvalidObjectKind
        | zerocore::compute::ComputeError::InvalidTransaction(_) => (6001, "tx_error", "transaction"),
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
}

impl RpcServer {
    /// Creates server with validation and returns detailed error on invalid config.
    pub fn try_new(config: RpcConfig) -> Result<Self, crate::ApiError> {
        config
            .validate()
            .map_err(crate::ApiError::InvalidRequest)?;

        let api = Some(Arc::new(build_default_rpc_api(config.clone())));
        Ok(Self {
            config,
            api,
        })
    }

    pub fn new(config: RpcConfig) -> Self {
        match Self::try_new(config.clone()) {
            Ok(server) => server,
            Err(err) => {
                tracing::warn!("invalid rpc config, fallback to default: {}", err);
                // Keep backward compatibility for callers expecting infallible constructor.
                Self::try_new(RpcConfig::default())
                    .expect("default RpcConfig must be valid")
            }
        }
    }

    /// Create server with externally provided RPC API instance.
    pub fn with_api(config: RpcConfig, api: Arc<RpcApi>) -> Self {
        Self {
            config,
            api: Some(api),
        }
    }

    /// Returns the RPC API instance if initialized.
    pub fn api(&self) -> Option<Arc<RpcApi>> {
        self.api.clone()
    }
    
    pub async fn start(&self) -> Result<(), crate::ApiError> {
        // Would start HTTP server
        Ok(())
    }
    
    pub async fn stop(&self) -> Result<(), crate::ApiError> {
        Ok(())
    }
}

fn build_default_rpc_api(config: RpcConfig) -> RpcApi {
    let account_manager = Arc::new(InMemoryAccountManager::new());
    let tx_pool = Arc::new(TransactionPool::new(TxPoolConfig::default(), account_manager));
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
    let bytes = hex::decode(s.strip_prefix("0x").unwrap_or(s))
        .map_err(|e| RpcErrorObject::invalid_params(format!("Invalid address: {}", e)))?;
    
    if bytes.len() != 20 {
        return Err(RpcErrorObject::invalid_params("Address must be 20 bytes".into()));
    }
    
    Ok(Address::from_slice(&bytes).unwrap())
}

fn parse_hash(s: &str) -> Result<Hash, RpcErrorObject> {
    let bytes = hex::decode(s.strip_prefix("0x").unwrap_or(s))
        .map_err(|e| RpcErrorObject::invalid_params(format!("Invalid hash: {}", e)))?;
    
    if bytes.len() != 32 {
        return Err(RpcErrorObject::invalid_params("Hash must be 32 bytes".into()));
    }
    
    Ok(Hash::from_slice(&bytes).unwrap())
}

fn parse_object_id(s: &str) -> Result<ObjectId, RpcErrorObject> {
    Ok(ObjectId(parse_hash(s)?))
}

fn parse_output_id(s: &str) -> Result<OutputId, RpcErrorObject> {
    Ok(OutputId(parse_hash(s)?))
}

fn object_output_to_json(output: ObjectOutput) -> serde_json::Value {
    serde_json::json!({
        "output_id": format!("0x{}", hex::encode(output.output_id.0.as_bytes())),
        "object_id": format!("0x{}", hex::encode(output.object_id.0.as_bytes())),
        "version": output.version.0,
        "domain_id": output.domain_id.0,
        "kind": format!("{:?}", output.kind),
        "spent": output.spent,
        "predecessor": output.predecessor.map(|p| format!("0x{}", hex::encode(p.0.as_bytes()))),
        "state": format!("0x{}", hex::encode(output.state)),
        "logic": output.logic.map(|b| format!("0x{}", hex::encode(b))),
    })
}

fn parse_compute_tx(value: serde_json::Value) -> Result<ComputeTx, RpcErrorObject> {
    let obj = value
        .as_object()
        .ok_or_else(|| RpcErrorObject::invalid_params("tx must be object".to_string()))?;

    let tx_id = parse_hash_field(obj, "tx_id").map(zerocore::compute::TxId)?;
    let domain_id = DomainId(parse_u32_field(obj, "domain_id")?);
    let command = parse_command(
        obj.get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| RpcErrorObject::invalid_params("command must be string".to_string()))?,
    )?;

    let input_set = parse_hash_array_field(obj, "input_set")?
        .into_iter()
        .map(OutputId)
        .collect::<Vec<_>>();

    let read_set = parse_read_set(obj.get("read_set"))?;
    let output_proposals = parse_output_proposals(obj.get("output_proposals"))?;

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
        .ok_or_else(|| RpcErrorObject::invalid_params("witness.signatures must be array".to_string()))?;

    let mut signatures = Vec::with_capacity(sig_arr.len());
    for raw in sig_arr {
        if let Some(s) = raw.as_str() {
            let bytes = hex::decode(s.strip_prefix("0x").unwrap_or(s))
                .map_err(|e| RpcErrorObject::invalid_params(format!("invalid signature hex: {e}")))?;
            let sig = Signature::from_bytes(&bytes)
                .map_err(|e| RpcErrorObject::invalid_params(format!("invalid signature bytes: {e}")))?;
            signatures.push(TxSignature::secp256k1(sig));
            continue;
        }

        let obj = raw
            .as_object()
            .ok_or_else(|| RpcErrorObject::invalid_params("signature must be string or object".to_string()))?;
        let scheme = obj
            .get("scheme")
            .and_then(|x| x.as_str())
            .ok_or_else(|| RpcErrorObject::invalid_params("signature.scheme must be string".to_string()))?;
        let sig_hex = obj
            .get("signature")
            .and_then(|x| x.as_str())
            .ok_or_else(|| RpcErrorObject::invalid_params("signature.signature must be string".to_string()))?;
        let sig_bytes = hex::decode(sig_hex.strip_prefix("0x").unwrap_or(sig_hex))
            .map_err(|e| RpcErrorObject::invalid_params(format!("invalid signature hex: {e}")))?;

        match scheme {
            "secp256k1" => {
                let sig = Signature::from_bytes(&sig_bytes)
                    .map_err(|e| RpcErrorObject::invalid_params(format!("invalid signature bytes: {e}")))?;
                signatures.push(TxSignature::secp256k1(sig));
            }
            "ed25519" => {
                let pubkey_hex = obj
                    .get("public_key")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| RpcErrorObject::invalid_params("ed25519 signature requires public_key".to_string()))?;
                let pubkey = hex::decode(pubkey_hex.strip_prefix("0x").unwrap_or(pubkey_hex))
                    .map_err(|e| RpcErrorObject::invalid_params(format!("invalid public_key hex: {e}")))?;
                if sig_bytes.len() != 64 {
                    return Err(RpcErrorObject::invalid_params("ed25519 signature must be 64 bytes".to_string()));
                }
                if pubkey.len() != 32 {
                    return Err(RpcErrorObject::invalid_params("ed25519 public_key must be 32 bytes".to_string()));
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
            let v = raw
                .as_u64()
                .ok_or_else(|| RpcErrorObject::invalid_params("witness.threshold must be u64".to_string()))?;
            Some(u16::try_from(v)
                .map_err(|_| RpcErrorObject::invalid_params("witness.threshold overflow".to_string()))?)
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
        _ => Err(RpcErrorObject::invalid_params(format!("unsupported command: {s}"))),
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
        _ => Err(RpcErrorObject::invalid_params(format!("unsupported object kind: {s}"))),
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
            let addr = obj
                .get("address")
                .and_then(|x| x.as_str())
                .ok_or_else(|| RpcErrorObject::invalid_params("owner.address missing".to_string()))?;
            Ok(Ownership::Address(parse_address(addr)?))
        }
        "Program" => {
            let addr = obj
                .get("address")
                .and_then(|x| x.as_str())
                .ok_or_else(|| RpcErrorObject::invalid_params("owner.address missing".to_string()))?;
            Ok(Ownership::Program(parse_address(addr)?))
        }
        "NativeEd25519" => {
            let pubkey = obj
                .get("public_key")
                .and_then(|x| x.as_str())
                .ok_or_else(|| RpcErrorObject::invalid_params("owner.public_key missing".to_string()))?;
            let bytes = hex::decode(pubkey.strip_prefix("0x").unwrap_or(pubkey))
                .map_err(|e| RpcErrorObject::invalid_params(format!("invalid owner.public_key hex: {e}")))?;
            if bytes.len() != 32 {
                return Err(RpcErrorObject::invalid_params(
                    "owner.public_key must be 32 bytes".to_string(),
                ));
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok(Ownership::NativeEd25519(arr))
        }
        _ => Err(RpcErrorObject::invalid_params(format!("unsupported owner type: {typ}"))),
    }
}

fn parse_read_set(v: Option<&serde_json::Value>) -> Result<Vec<zerocore::compute::ObjectReadRef>, RpcErrorObject> {
    let Some(v) = v else {
        return Ok(vec![]);
    };
    let arr = v
        .as_array()
        .ok_or_else(|| RpcErrorObject::invalid_params("read_set must be array".to_string()))?;

    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let obj = item
            .as_object()
            .ok_or_else(|| RpcErrorObject::invalid_params("read_set item must be object".to_string()))?;
        let output_id = parse_hash_field(obj, "output_id").map(OutputId)?;
        let domain_id = DomainId(parse_u32_field(obj, "domain_id")?);
        let expected_version = Version(
            obj.get("expected_version")
                .and_then(|x| x.as_u64())
                .ok_or_else(|| RpcErrorObject::invalid_params("expected_version missing".to_string()))?,
        );
        out.push(zerocore::compute::ObjectReadRef {
            output_id,
            domain_id,
            expected_version,
        });
    }
    Ok(out)
}

fn parse_output_proposals(v: Option<&serde_json::Value>) -> Result<Vec<OutputProposal>, RpcErrorObject> {
    let Some(v) = v else {
        return Ok(vec![]);
    };
    let arr = v
        .as_array()
        .ok_or_else(|| RpcErrorObject::invalid_params("output_proposals must be array".to_string()))?;

    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let obj = item
            .as_object()
            .ok_or_else(|| RpcErrorObject::invalid_params("output proposal must be object".to_string()))?;
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
        let logic = parse_bytes_hex_opt(obj.get("logic"))?;
        out.push(OutputProposal {
            output_id,
            object_id,
            domain_id,
            kind,
            owner,
            predecessor,
            version,
            state,
            logic,
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
        let s = item
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params(format!("{key} items must be hex string")))?;
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

fn parse_u32_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<u32, RpcErrorObject> {
    let v = obj
        .get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| RpcErrorObject::invalid_params(format!("{key} missing")))?;
    u32::try_from(v)
        .map_err(|_| RpcErrorObject::invalid_params(format!("{key} overflow")))
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
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use zerostore::db::MemDatabase;

    fn build_test_api_with_compute() -> RpcApi {
        let account_manager = Arc::new(InMemoryAccountManager::new());
        let tx_pool = Arc::new(TransactionPool::new(TxPoolConfig::default(), account_manager));
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
        let account_manager = Arc::new(InMemoryAccountManager::new());
        let tx_pool = Arc::new(TransactionPool::new(TxPoolConfig::default(), account_manager));
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
        assert_eq!(parsed.witness.signatures[0].scheme, SignatureScheme::Ed25519);
    }
    
    #[test]
    fn test_parse_address() {
        let addr = parse_address("0x0000000000000000000000000000000000000001").unwrap();
        assert!(!addr.is_zero());
    }
    
    #[test]
    fn test_parse_hash() {
        let hash = parse_hash("0x0000000000000000000000000000000000000000000000000000000000000001").unwrap();
        assert!(!hash.is_zero());
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
            logic: None,
            resources: BTreeMap::new(),
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
        assert_eq!(domain_value.get("domain_id").and_then(|v| v.as_u64()), Some(0));
    }

    #[test]
    fn test_zero_simulate_and_submit_compute_tx() {
        let api = build_test_api_with_compute();

        let witness_sig = format!("0x{}", hex::encode(Signature::new([1; 32], [2; 32], 27).as_bytes()));

        let tx = canonicalize_compute_tx_id(serde_json::json!({
            "tx_id": format!("0x{}", hex::encode([0x55u8; 32])),
            "domain_id": 0,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Mint",
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
            "input_set": [format!("0x{}", hex::encode([0xD2u8; 32]))],
            "read_set": [],
            "output_proposals": [{
                "output_id": format!("0x{}", hex::encode([0xD3u8; 32])),
                "object_id": format!("0x{}", hex::encode([0xD4u8; 32])),
                "domain_id": 0,
                "kind": "Asset",
                "owner": { "type": "Address", "address": "0x1111111111111111111111111111111111111111" },
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
            owner: Ownership::Address(Address::from_hex("0x1111111111111111111111111111111111111111").unwrap()),
            predecessor: None,
            state: vec![1],
            logic: None,
            resources: BTreeMap::new(),
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
            owner: Ownership::Address(Address::from_hex("0x2222222222222222222222222222222222222222").unwrap()),
            predecessor: None,
            state: vec![1],
            logic: None,
            resources: BTreeMap::new(),
            spent: false,
        };
        api.compute_store.insert_output(input).unwrap();

        let mut tx = serde_json::json!({
            "tx_id": format!("0x{}", hex::encode([0xF1u8; 32])),
            "domain_id": 0,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Transfer",
            "input_set": [format!("0x{}", hex::encode([0xF2u8; 32]))],
            "read_set": [],
            "output_proposals": [{
                "output_id": format!("0x{}", hex::encode([0xF4u8; 32])),
                "object_id": format!("0x{}", hex::encode([0xF3u8; 32])),
                "domain_id": 0,
                "kind": "Asset",
                "owner": { "type": "Address", "address": "0x2222222222222222222222222222222222222222" },
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
        tx["witness"]["signatures"] = serde_json::json!([format!(
            "0x{}",
            hex::encode(sig.as_bytes())
        )]);
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
            logic: None,
            resources: BTreeMap::new(),
            spent: false,
        };
        api.compute_store.insert_output(input).unwrap();

        let mut tx = serde_json::json!({
            "tx_id": format!("0x{}", hex::encode([0xADu8; 32])),
            "domain_id": 0,
            "chain_id": 10086,
            "network_id": 1,
            "command": "Transfer",
            "input_set": [format!("0x{}", hex::encode([0xABu8; 32]))],
            "read_set": [],
            "output_proposals": [{
                "output_id": format!("0x{}", hex::encode([0xAEu8; 32])),
                "object_id": format!("0x{}", hex::encode([0xACu8; 32])),
                "domain_id": 0,
                "kind": "Asset",
                "owner": { "type": "Address", "address": owner_addr.to_checksum_hex() },
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
        tx["witness"]["signatures"] = serde_json::json!([format!(
            "0x{}",
            hex::encode(sig.as_bytes())
        )]);

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
        let mut cfg = RpcConfig::default();
        cfg.compute_backend = ComputeBackend::Mem;
        let db = build_compute_kv_backend(&cfg);
        db.put(b"k", b"v").unwrap();
        assert_eq!(db.get(b"k").unwrap(), Some(b"v".to_vec()));
    }

    #[test]
    fn test_rpc_config_validate_rejects_empty_path_for_file_backend() {
        let mut cfg = RpcConfig::default();
        cfg.compute_backend = ComputeBackend::RocksDb;
        cfg.compute_db_path = "   ".to_string();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_try_new_returns_error_for_invalid_config() {
        let mut cfg = RpcConfig::default();
        cfg.compute_backend = ComputeBackend::Redb;
        cfg.compute_db_path = "".to_string();
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
