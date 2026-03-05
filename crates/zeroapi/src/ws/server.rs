//! WebSocket Server Core Implementation

use super::{WsConfig, SubscriptionManager, SubscriptionType, LogsFilter, WsNotification, Result, WsError};
use axum::{
    extract::ws::{WebSocket, WebSocketUpgrade, Message, CloseFrame},
    extract::State,
    response::IntoResponse,
    Json,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn, error};

/// WebSocket server
pub struct WsServer {
    config: WsConfig,
    manager: Arc<SubscriptionManager>,
    shutdown_signal: Option<tokio::sync::broadcast::Sender<()>>,
}

impl WsServer {
    /// Create new WebSocket server
    pub fn new(config: WsConfig) -> Self {
        Self {
            config,
            manager: Arc::new(SubscriptionManager::new(config.max_connections)),
            shutdown_signal: None,
        }
    }
    
    /// Get subscription manager
    pub fn manager(&self) -> Arc<SubscriptionManager> {
        self.manager.clone()
    }
    
    /// Start server
    pub async fn start(&mut self) -> Result<()> {
        let addr = format!("{}:{}", self.config.address, self.config.port);
        
        info!("Starting WebSocket server on {}", addr);
        
        // Create shutdown channel
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel(1);
        self.shutdown_signal = Some(shutdown_tx);
        
        // Would start actual WebSocket server here
        // For now, just simulate
        
        Ok(())
    }
    
    /// Stop server
    pub async fn stop(&self) -> Result<()> {
        if let Some(tx) = &self.shutdown_signal {
            let _ = tx.send(());
            info!("WebSocket server stopping");
        }
        
        Ok(())
    }
    
    /// Handle WebSocket upgrade
    pub async fn handle_ws(&self, ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(|socket| self.handle_socket(socket))
    }
    
    /// Handle WebSocket connection
    async fn handle_socket(&self, socket: WebSocket) {
        let (mut sender, mut receiver) = socket.split();
        
        // Check connection limit
        if !self.manager.add_connection() {
            warn!("Connection limit reached");
            let _ = sender
                .send(Message::Close(Some(CloseFrame {
                    code: 1013,
                    reason: "Too many connections".into(),
                })))
                .await;
            return;
        }
        
        info!("New WebSocket connection");
        
        // Subscribe to broadcast channels
        let mut new_heads_rx = self.manager.get_channels().new_heads.resubscribe();
        let mut new_pending_txs_rx = self.manager.get_channels().new_pending_txs.resubscribe();
        let mut logs_rx = self.manager.get_channels().logs.resubscribe();
        
        // Spawn task to handle incoming messages
        let manager = self.manager.clone();
        let handle_incoming = tokio::spawn(async move {
            while let Some(msg) = receiver.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Err(e) = handle_incoming_message(&text, &manager, &mut sender).await {
                            error!("Error handling message: {}", e);
                        }
                    }
                    Ok(Message::Close(_)) => {
                        break;
                    }
                    Err(e) => {
                        error!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        });
        
        // Spawn task to broadcast messages
        let handle_broadcast = tokio::spawn(async move {
            loop {
                tokio::select! {
                    Ok(header) = new_heads_rx.recv() => {
                        // Broadcast to all subscribers
                        let _ = sender.send(Message::Text(
                            serde_json::to_string(&WsNotification::new(
                                "new_heads".to_string(),
                                header,
                            )).unwrap()
                        )).await;
                    }
                    Ok(tx_hash) = new_pending_txs_rx.recv() => {
                        let _ = sender.send(Message::Text(
                            serde_json::to_string(&WsNotification::new(
                                "new_pending_txs".to_string(),
                                tx_hash,
                            )).unwrap()
                        )).await;
                    }
                    Ok((filter, log)) = logs_rx.recv() => {
                        let _ = sender.send(Message::Text(
                            serde_json::to_string(&WsNotification::new(
                                "logs".to_string(),
                                log,
                            )).unwrap()
                        )).await;
                    }
                    else => break,
                }
            }
        });
        
        // Wait for either task to complete
        tokio::select! {
            _ = handle_incoming => {},
            _ = handle_broadcast => {},
        }
        
        // Cleanup
        self.manager.remove_connection();
        info!("WebSocket connection closed");
    }
}

/// WebSocket request
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WsRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Vec<serde_json::Value>>,
    pub id: serde_json::Value,
}

/// WebSocket response
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WsResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WsErrorObject>,
    pub id: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WsErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Handle incoming WebSocket message
async fn handle_incoming_message(
    text: &str,
    manager: &Arc<SubscriptionManager>,
    sender: &mut futures_util::sink::SplitSink<WebSocket, Message>,
) -> Result<()> {
    // Parse request
    let request: WsRequest = serde_json::from_str(text)
        .map_err(|e| WsError::Serialization(e.to_string()))?;
    
    // Handle method
    let response = match request.method.as_str() {
        "eth_subscribe" => handle_subscribe(&request, manager),
        "eth_unsubscribe" => handle_unsubscribe(&request, manager),
        _ => {
            Err(WsError::Subscription("Method not supported".into()))
        }
    };
    
    // Send response
    let ws_response = match response {
        Ok(result) => WsResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id: request.id,
        },
        Err(e) => WsResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(WsErrorObject {
                code: -32000,
                message: e.to_string(),
                data: None,
            }),
            id: request.id,
        },
    };
    
    let response_text = serde_json::to_string(&ws_response).unwrap();
    sender.send(Message::Text(response_text)).await
        .map_err(|_| WsError::Channel)?;
    
    Ok(())
}

/// Handle eth_subscribe
fn handle_subscribe(request: &WsRequest, manager: &Arc<SubscriptionManager>) -> Result<serde_json::Value> {
    let params = request.params.as_ref()
        .ok_or_else(|| WsError::Subscription("Missing params".into()))?;
    
    let sub_type_str = params.get(0)
        .and_then(|v| v.as_str())
        .ok_or_else(|| WsError::Subscription("Missing subscription type".into()))?;
    
    let sub_type = match sub_type_str {
        "newHeads" => SubscriptionType::NewHeads,
        "newPendingTransactions" => SubscriptionType::NewPendingTransactions,
        "logs" => {
            let filter = params.get(1)
                .and_then(|v| serde_json::from_value::<LogsFilter>(v.clone()).ok())
                .unwrap_or_default();
            SubscriptionType::Logs(filter)
        }
        "syncing" => SubscriptionType::Syncing,
        _ => return Err(WsError::Subscription("Invalid subscription type".into())),
    };
    
    // Create subscription
    let id = manager.create_subscription(sub_type);
    
    Ok(serde_json::json!(id))
}

/// Handle eth_unsubscribe
fn handle_unsubscribe(request: &WsRequest, manager: &Arc<SubscriptionManager>) -> Result<serde_json::Value> {
    let params = request.params.as_ref()
        .ok_or_else(|| WsError::Subscription("Missing params".into()))?;
    
    let id = params.get(0)
        .and_then(|v| v.as_str())
        .ok_or_else(|| WsError::Subscription("Missing subscription id".into()))?;
    
    let removed = manager.remove_subscription(id);
    
    Ok(serde_json::json!(removed))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_subscribe_request() {
        let manager = Arc::new(SubscriptionManager::new(10));
        
        let request = WsRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_subscribe".to_string(),
            params: Some(vec![serde_json::json!("newHeads")]),
            id: serde_json::json!(1),
        };
        
        let result = handle_subscribe(&request, &manager);
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_unsubscribe_request() {
        let manager = Arc::new(SubscriptionManager::new(10));
        
        // Create subscription first
        let sub_request = WsRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_subscribe".to_string(),
            params: Some(vec![serde_json::json!("newHeads")]),
            id: serde_json::json!(1),
        };
        
        let sub_id = handle_subscribe(&sub_request, &manager).unwrap();
        
        // Unsubscribe
        let unsub_request = WsRequest {
            jsonrpc: "2.0".to_string(),
            method: "eth_unsubscribe".to_string(),
            params: Some(vec![sub_id.clone()]),
            id: serde_json::json!(2),
        };
        
        let result = handle_unsubscribe(&unsub_request, &manager);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), serde_json::json!(true));
    }
}
