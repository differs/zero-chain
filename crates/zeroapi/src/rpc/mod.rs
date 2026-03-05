//! JSON-RPC Server Implementation

use zerocore::crypto::{Address, Hash};
use zerocore::account::U256;
use zerocore::transaction::{SignedTransaction, TransactionPool};
use zerocore::block::{Block, BlockHeader};
use zerocore::state::StateDb;
use std::sync::Arc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use axum::{Router, routing::post, Json, extract::State};
use tower_http::cors::{CorsLayer, Any};

/// RPC configuration
#[derive(Clone, Debug)]
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
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".to_string(),
            port: 8545,
            max_connections: 100,
            max_request_size: 15 * 1024 * 1024,  // 15MB
            modules: vec!["eth".into(), "net".into(), "web3".into()],
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

/// RPC API handler
pub struct RpcApi {
    config: RpcConfig,
    state_db: Arc<StateDb>,
    tx_pool: Arc<TransactionPool>,
    latest_block: RwLock<Option<Block>>,
}

impl RpcApi {
    pub fn new(
        config: RpcConfig,
        state_db: Arc<StateDb>,
        tx_pool: Arc<TransactionPool>,
    ) -> Self {
        Self {
            config,
            state_db,
            tx_pool,
            latest_block: RwLock::new(None),
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
            
            _ => Err(RpcErrorObject::method_not_found(method)),
        }
    }
    
    // ============ web3_* methods ============
    
    fn web3_client_version(&self, _params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!("ZeroChain/v0.1.0"))
    }
    
    fn web3_sha3(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or_else(|| RpcErrorObject::invalid_params("Missing params"))?;
        let data = params.get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing data"))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Data must be string"))?;
        
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
        Ok(serde_json::json!(format!("0x{:x}", number)))
    }
    
    fn eth_get_balance(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or_else(|| RpcErrorObject::invalid_params("Missing params"))?;
        
        let address_str = params.get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing address"))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Address must be string"))?;
        
        let address = parse_address(address_str)?;
        
        let balance = self.state_db.get_balance(&address);
        
        Ok(serde_json::json!(format!("0x{:x}", balance)))
    }
    
    fn eth_get_storage_at(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or_else(|| RpcErrorObject::invalid_params("Missing params"))?;
        
        let address_str = params.get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing address"))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Address must be string"))?;
        
        let position_str = params.get(1)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing position"))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Position must be string"))?;
        
        let address = parse_address(address_str)?;
        let position = parse_hash(position_str)?;
        
        let value = self.state_db.get_storage(&address, &position);
        
        Ok(serde_json::json!(format!("0x{}", hex::encode(value.as_bytes()))))
    }
    
    fn eth_get_transaction_count(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        let params = params.ok_or_else(|| RpcErrorObject::invalid_params("Missing params"))?;
        
        let address_str = params.get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing address"))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Address must be string"))?;
        
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
        let params = params.ok_or_else(|| RpcErrorObject::invalid_params("Missing params"))?;
        
        let tx_data = params.get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing transaction data"))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Transaction data must be string"))?;
        
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
        let params = params.ok_or_else(|| RpcErrorObject::invalid_params("Missing params"))?;
        
        let address_str = params.get(0)
            .ok_or_else(|| RpcErrorObject::invalid_params("Missing address"))?
            .as_str()
            .ok_or_else(|| RpcErrorObject::invalid_params("Address must be string"))?;
        
        let address = parse_address(address_str)?;
        
        // Would get full account info
        Ok(serde_json::json!({
            "address": address_str,
            "balance": format!("0x{:x}", self.state_db.get_balance(&address)),
            "nonce": format!("0x{:x}", self.state_db.get_nonce(&address)),
        }))
    }
    
    fn zero_get_utxos(&self, params: Option<Vec<serde_json::Value>>) -> Result<serde_json::Value, RpcErrorObject> {
        Ok(serde_json::json!([]))
    }
}

/// RPC Server
pub struct RpcServer {
    config: RpcConfig,
    api: Option<Arc<RpcApi>>,
}

impl RpcServer {
    pub fn new(config: RpcConfig) -> Self {
        Self {
            config,
            api: None,
        }
    }
    
    pub async fn start(&self) -> Result<(), crate::ApiError> {
        // Would start HTTP server
        Ok(())
    }
    
    pub async fn stop(&self) -> Result<(), crate::ApiError> {
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    
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
}
