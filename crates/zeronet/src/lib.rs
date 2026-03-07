//! ZeroChain P2P Network Layer
//!
//! Provides:
//! - Peer discovery and management
//! - Block and transaction propagation
//! - Chain synchronization
//! - RLPx protocol implementation

#![allow(missing_docs)]
#![allow(rustdoc::missing_crate_level_docs)]
#![allow(unused)]

pub mod discovery;
pub mod peer;
pub mod protocol;
pub mod sync;

pub use discovery::{Discovery, NodeRecord};
pub use peer::{Peer, PeerInfo, PeerManager, PeerStatus};
pub use protocol::{BlockMessage, Protocol, ProtocolMessage, TxMessage};
pub use sync::{SyncManager, SyncState};

use parking_lot::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

static GLOBAL_PEER_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Returns the current process-level peer count.
pub fn global_peer_count() -> usize {
    GLOBAL_PEER_COUNT.load(Ordering::Relaxed)
}

pub(crate) fn set_global_peer_count(count: usize) {
    GLOBAL_PEER_COUNT.store(count, Ordering::Relaxed);
}

/// Network error types
#[derive(Error, Debug)]
pub enum NetworkError {
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("Peer not found: {0}")]
    PeerNotFound(String),
    #[error("Connection error: {0}")]
    ConnectionError(String),
    #[error("Protocol error: {0}")]
    ProtocolError(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Channel error")]
    ChannelError,
}

pub type Result<T> = std::result::Result<T, NetworkError>;

/// Network configuration
#[derive(Clone, Debug)]
pub struct NetworkConfig {
    /// Network ID
    pub network_id: u64,
    /// Listen address
    pub listen_addr: String,
    /// Listen port
    pub listen_port: u16,
    /// External address (optional)
    pub external_addr: Option<String>,
    /// Maximum peers
    pub max_peers: u32,
    /// Minimum peers
    pub min_peers: u32,
    /// Bootstrap nodes
    pub bootnodes: Vec<String>,
    /// Node name
    pub node_name: String,
    /// Enable discovery
    pub enable_discovery: bool,
    /// Enable sync
    pub enable_sync: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            network_id: 10086,
            listen_addr: "0.0.0.0".to_string(),
            listen_port: 30303,
            external_addr: None,
            max_peers: 50,
            min_peers: 25,
            bootnodes: Vec::new(),
            node_name: "ZeroChain/v0.1.0".to_string(),
            enable_discovery: true,
            enable_sync: true,
        }
    }
}

/// Network service
pub struct NetworkService {
    config: NetworkConfig,
    peer_manager: Arc<PeerManager>,
    discovery: Option<Arc<Discovery>>,
    sync_manager: Option<Arc<SyncManager>>,
    is_running: RwLock<bool>,
    listener_task: RwLock<Option<JoinHandle<()>>>,
}

impl NetworkService {
    /// Create new network service
    pub fn new(config: NetworkConfig) -> Result<Self> {
        let peer_manager = Arc::new(PeerManager::new(config.max_peers));

        let discovery = if config.enable_discovery {
            Some(Arc::new(Discovery::new(&config)?))
        } else {
            None
        };

        let sync_manager = if config.enable_sync {
            Some(Arc::new(SyncManager::new(peer_manager.clone())))
        } else {
            None
        };

        Ok(Self {
            config,
            peer_manager,
            discovery,
            sync_manager,
            is_running: RwLock::new(false),
            listener_task: RwLock::new(None),
        })
    }

    /// Start network service
    pub async fn start(&self) -> Result<()> {
        if *self.is_running.read() {
            return Err(NetworkError::ConnectionError("Already running".into()));
        }

        tracing::info!(
            "Starting network service on port {}",
            self.config.listen_port
        );

        // Start listening
        self.start_listening().await?;

        // Connect to bootnodes
        if !self.config.bootnodes.is_empty() {
            self.connect_bootnodes().await?;
        }

        // Start discovery
        if let Some(discovery) = &self.discovery {
            discovery.start().await?;
        }

        // Start sync
        if let Some(sync) = &self.sync_manager {
            sync.start_default().await?;
        }

        *self.is_running.write() = true;
        set_global_peer_count(self.peer_manager.peer_count());

        tracing::info!("Network service started");

        Ok(())
    }

    /// Stop network service
    pub async fn stop(&self) -> Result<()> {
        if !*self.is_running.read() {
            return Ok(());
        }

        tracing::info!("Stopping network service");

        // Stop discovery
        if let Some(discovery) = &self.discovery {
            discovery.stop().await?;
        }

        // Stop sync
        if let Some(sync) = &self.sync_manager {
            sync.stop().await?;
        }

        if let Some(task) = self.listener_task.write().take() {
            task.abort();
        }

        // Disconnect all peers
        self.peer_manager.disconnect_all_peers();
        set_global_peer_count(0);

        *self.is_running.write() = false;

        tracing::info!("Network service stopped");

        Ok(())
    }

    /// Broadcast transaction to all peers
    pub fn broadcast_transaction(&self, tx_hash: zerocore::crypto::Hash) {
        let message = ProtocolMessage::NewTransaction(tx_hash);

        for peer in self.peer_manager.get_active_peers() {
            let _ = peer.send(message.clone());
        }
    }

    /// Broadcast block to all peers
    pub fn broadcast_block(&self, block: zerocore::block::Block) {
        let message = ProtocolMessage::NewBlock(Box::new(block));

        for peer in self.peer_manager.get_active_peers() {
            let _ = peer.send(message.clone());
        }
    }

    /// Get connected peer count
    pub fn peer_count(&self) -> usize {
        self.peer_manager.get_active_peer_infos().len()
    }

    /// Get all connected peers
    pub fn get_peers(&self) -> Vec<PeerInfo> {
        self.peer_manager.get_active_peer_infos()
    }

    /// Add peer
    pub fn add_peer(&self, node_record: NodeRecord) -> Result<()> {
        let result = self.peer_manager.add_peer(node_record);
        if result.is_ok() {
            set_global_peer_count(self.peer_manager.peer_count());
        }
        result
    }

    /// Remove peer
    pub fn remove_peer(&self, peer_id: &str) -> Result<()> {
        let result = self.peer_manager.remove_peer(peer_id);
        if result.is_ok() {
            set_global_peer_count(self.peer_manager.peer_count());
        }
        result
    }

    async fn start_listening(&self) -> Result<()> {
        let bind_addr = format!("{}:{}", self.config.listen_addr, self.config.listen_port);
        let listener = TcpListener::bind(&bind_addr)
            .await
            .map_err(|e| NetworkError::ConnectionError(format!("bind {bind_addr} failed: {e}")))?;

        let peer_manager = self.peer_manager.clone();
        let task = tokio::spawn(async move {
            tracing::info!("P2P listener started on {}", bind_addr);
            loop {
                match listener.accept().await {
                    Ok((stream, remote_addr)) => {
                        let peer_id = format!("inbound-{}-{}", remote_addr, current_timestamp());
                        let node_record = NodeRecord {
                            peer_id: peer_id.clone(),
                            ip: remote_addr.ip().to_string(),
                            tcp_port: remote_addr.port(),
                            udp_port: remote_addr.port(),
                        };

                        if let Err(err) = peer_manager.add_peer(node_record) {
                            tracing::warn!(
                                "failed to register inbound peer {}: {}",
                                remote_addr,
                                err
                            );
                            continue;
                        }

                        set_global_peer_count(peer_manager.peer_count());
                        tokio::spawn(monitor_peer_socket(peer_manager.clone(), peer_id, stream));
                    }
                    Err(err) => {
                        tracing::warn!("P2P accept error: {}", err);
                        break;
                    }
                }
            }
        });

        *self.listener_task.write() = Some(task);
        Ok(())
    }

    async fn connect_bootnodes(&self) -> Result<()> {
        for bootnode in &self.config.bootnodes {
            match NodeRecord::from_enode(bootnode) {
                Ok(record) => {
                    let addr = format!("{}:{}", record.ip, record.tcp_port);
                    match tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        TcpStream::connect(&addr),
                    )
                    .await
                    {
                        Ok(Ok(stream)) => {
                            if let Err(err) = self.add_peer(record.clone()) {
                                tracing::warn!(
                                    "Failed to register bootnode {} as peer: {}",
                                    bootnode,
                                    err
                                );
                                continue;
                            }
                            tokio::spawn(monitor_peer_socket(
                                self.peer_manager.clone(),
                                record.peer_id.clone(),
                                stream,
                            ));
                        }
                        Ok(Err(err)) => {
                            tracing::warn!("Failed to connect bootnode {}: {}", bootnode, err);
                        }
                        Err(_) => {
                            tracing::warn!("Bootnode connect timeout: {}", bootnode);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Invalid bootnode {}: {}", bootnode, e);
                }
            }
        }

        Ok(())
    }
}

async fn monitor_peer_socket(
    peer_manager: Arc<PeerManager>,
    peer_id: String,
    mut stream: TcpStream,
) {
    let mut buf = [0u8; 1];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => break,
            Ok(_) => continue,
            Err(_) => break,
        }
    }

    let _ = peer_manager.remove_peer(&peer_id);
    set_global_peer_count(peer_manager.peer_count());
}

fn current_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_network_service() {
        let config = NetworkConfig {
            listen_port: 30304, // Use different port for tests
            enable_discovery: false,
            enable_sync: false,
            ..Default::default()
        };

        let network = NetworkService::new(config).unwrap();

        assert_eq!(network.peer_count(), 0);

        // Start and stop
        network.start().await.unwrap();
        assert!(*network.is_running.read());

        network.stop().await.unwrap();
        assert!(!*network.is_running.read());
    }
}
