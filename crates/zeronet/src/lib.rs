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

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio::time::{interval, timeout, MissedTickBehavior};
use uuid::Uuid;

static GLOBAL_PEER_COUNT: AtomicUsize = AtomicUsize::new(0);
static GLOBAL_PEER_INFOS: Lazy<RwLock<Vec<PeerInfo>>> = Lazy::new(|| RwLock::new(Vec::new()));
const HANDSHAKE_PREFIX: &str = "ZERO/1";
const HANDSHAKE_MAX_LEN: usize = 512;
const HANDSHAKE_TIMEOUT_SECS: u64 = 5;
const HEARTBEAT_PING: &[u8] = b"ZERO/PING\n";
const HEARTBEAT_PONG: &[u8] = b"ZERO/PONG\n";
const CONTROL_FRAME_MAX_LEN: usize = 256;
const HEARTBEAT_INTERVAL_SECS: u64 = 15;
const PEER_IDLE_TIMEOUT_SECS: u64 = 45;

/// Returns the current process-level peer count.
pub fn global_peer_count() -> usize {
    GLOBAL_PEER_COUNT.load(Ordering::Relaxed)
}

pub(crate) fn set_global_peer_count(count: usize) {
    GLOBAL_PEER_COUNT.store(count, Ordering::Relaxed);
}

/// Returns snapshots for all currently tracked peers.
pub fn global_peers() -> Vec<PeerInfo> {
    GLOBAL_PEER_INFOS.read().clone()
}

pub(crate) fn set_global_peers(peers: Vec<PeerInfo>) {
    *GLOBAL_PEER_INFOS.write() = peers;
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
    local_peer_id: String,
    peer_manager: Arc<PeerManager>,
    discovery: Option<Arc<Discovery>>,
    sync_manager: Option<Arc<SyncManager>>,
    is_running: RwLock<bool>,
    listener_task: RwLock<Option<JoinHandle<()>>>,
}

impl NetworkService {
    /// Create new network service
    pub fn new(config: NetworkConfig) -> Result<Self> {
        let local_peer_id = format!("node-{}", Uuid::new_v4().simple());
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
            local_peer_id,
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
        set_global_peers(Vec::new());

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

        let expected_network_id = self.config.network_id;
        let local_peer_id = self.local_peer_id.clone();
        let peer_manager = self.peer_manager.clone();
        let task = tokio::spawn(async move {
            tracing::info!("P2P listener started on {}", bind_addr);
            loop {
                match listener.accept().await {
                    Ok((mut stream, remote_addr)) => {
                        let (remote_network_id, remote_peer_id) = match inbound_handshake(
                            &mut stream,
                            expected_network_id,
                            &local_peer_id,
                        )
                        .await
                        {
                            Ok(v) => v,
                            Err(err) => {
                                tracing::warn!(
                                    "inbound handshake failed from {}: {}",
                                    remote_addr,
                                    err
                                );
                                continue;
                            }
                        };

                        let node_record = NodeRecord {
                            peer_id: remote_peer_id.clone(),
                            ip: remote_addr.ip().to_string(),
                            tcp_port: remote_addr.port(),
                            udp_port: remote_addr.port(),
                            network_id: remote_network_id,
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
                        tokio::spawn(monitor_peer_socket(
                            peer_manager.clone(),
                            remote_peer_id,
                            stream,
                        ));
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
                        Ok(Ok(mut stream)) => {
                            let (remote_network_id, remote_peer_id) = match outbound_handshake(
                                &mut stream,
                                self.config.network_id,
                                &self.local_peer_id,
                            )
                            .await
                            {
                                Ok(v) => v,
                                Err(err) => {
                                    tracing::warn!(
                                        "bootnode handshake failed {}: {}",
                                        bootnode,
                                        err
                                    );
                                    continue;
                                }
                            };
                            let node_record = NodeRecord {
                                peer_id: remote_peer_id.clone(),
                                ip: record.ip.clone(),
                                tcp_port: record.tcp_port,
                                udp_port: record.udp_port,
                                network_id: remote_network_id,
                            };
                            if let Err(err) = self.add_peer(node_record) {
                                tracing::warn!(
                                    "Failed to register bootnode {} as peer: {}",
                                    bootnode,
                                    err
                                );
                                continue;
                            }
                            tokio::spawn(monitor_peer_socket(
                                self.peer_manager.clone(),
                                remote_peer_id,
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
    let _ = peer_manager.touch_peer(&peer_id);
    let mut ticker = interval(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(err) = stream.write_all(HEARTBEAT_PING).await {
                    tracing::debug!("heartbeat write failed for {}: {}", peer_id, err);
                    break;
                }
                if peer_manager
                    .stale_peers(PEER_IDLE_TIMEOUT_SECS)
                    .iter()
                    .any(|id| id == &peer_id)
                {
                    tracing::info!("peer {} considered stale, disconnecting", peer_id);
                    break;
                }
            }
            frame = read_control_frame(&mut stream) => {
                match frame {
                    Ok(ControlFrame::Ping) => {
                        let _ = peer_manager.touch_peer(&peer_id);
                        if let Err(err) = stream.write_all(HEARTBEAT_PONG).await {
                            tracing::debug!("heartbeat pong write failed for {}: {}", peer_id, err);
                            break;
                        }
                    }
                    Ok(ControlFrame::Pong) => {
                        let _ = peer_manager.touch_peer(&peer_id);
                    }
                    Ok(ControlFrame::Other(line)) => {
                        tracing::debug!("received non-control frame from {}: {}", peer_id, line);
                        let _ = peer_manager.touch_peer(&peer_id);
                    }
                    Ok(ControlFrame::Eof) => break,
                    Err(err) => {
                        tracing::debug!("control frame read failed for {}: {}", peer_id, err);
                        break;
                    }
                }
            }
        }
    }

    let _ = peer_manager.remove_peer(&peer_id);
    set_global_peer_count(peer_manager.peer_count());
}

enum ControlFrame {
    Ping,
    Pong,
    Other(String),
    Eof,
}

async fn read_control_frame(stream: &mut TcpStream) -> std::io::Result<ControlFrame> {
    let mut line = Vec::with_capacity(32);
    loop {
        let mut b = [0u8; 1];
        let read = stream.read(&mut b).await?;
        if read == 0 {
            return Ok(ControlFrame::Eof);
        }
        if b[0] == b'\n' {
            break;
        }
        line.push(b[0]);
        if line.len() > CONTROL_FRAME_MAX_LEN {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "control frame line too long",
            ));
        }
    }

    let line = String::from_utf8(line).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("control frame is not utf8: {err}"),
        )
    })?;
    let normalized = line.trim();
    let frame = match normalized {
        "ZERO/PING" => ControlFrame::Ping,
        "ZERO/PONG" => ControlFrame::Pong,
        _ => ControlFrame::Other(normalized.to_string()),
    };
    Ok(frame)
}

async fn inbound_handshake(
    stream: &mut TcpStream,
    expected_network_id: u64,
    local_peer_id: &str,
) -> Result<(u64, String)> {
    let (remote_network_id, remote_peer_id) = read_handshake(stream).await?;
    if remote_network_id != expected_network_id {
        return Err(NetworkError::ProtocolError(format!(
            "network id mismatch: expected {}, got {}",
            expected_network_id, remote_network_id
        )));
    }
    send_handshake(stream, expected_network_id, local_peer_id).await?;
    Ok((remote_network_id, remote_peer_id))
}

async fn outbound_handshake(
    stream: &mut TcpStream,
    expected_network_id: u64,
    local_peer_id: &str,
) -> Result<(u64, String)> {
    send_handshake(stream, expected_network_id, local_peer_id).await?;
    let (remote_network_id, remote_peer_id) = read_handshake(stream).await?;
    if remote_network_id != expected_network_id {
        return Err(NetworkError::ProtocolError(format!(
            "network id mismatch: expected {}, got {}",
            expected_network_id, remote_network_id
        )));
    }
    Ok((remote_network_id, remote_peer_id))
}

async fn send_handshake(stream: &mut TcpStream, network_id: u64, peer_id: &str) -> Result<()> {
    if peer_id.trim().is_empty() {
        return Err(NetworkError::ProtocolError(
            "empty peer id in handshake".to_string(),
        ));
    }
    let line = format!("{HANDSHAKE_PREFIX} {network_id} {peer_id}\n");
    if line.len() > HANDSHAKE_MAX_LEN {
        return Err(NetworkError::ProtocolError(
            "handshake payload too large".to_string(),
        ));
    }
    timeout(
        std::time::Duration::from_secs(HANDSHAKE_TIMEOUT_SECS),
        stream.write_all(line.as_bytes()),
    )
    .await
    .map_err(|_| NetworkError::ConnectionError("handshake write timeout".to_string()))?
    .map_err(NetworkError::IO)?;
    Ok(())
}

async fn read_handshake(stream: &mut TcpStream) -> Result<(u64, String)> {
    let mut line = Vec::with_capacity(128);
    let read_result = timeout(
        std::time::Duration::from_secs(HANDSHAKE_TIMEOUT_SECS),
        async {
            loop {
                let mut b = [0u8; 1];
                let n = stream.read(&mut b).await?;
                if n == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "peer closed during handshake",
                    ));
                }
                if b[0] == b'\n' {
                    break;
                }
                line.push(b[0]);
                if line.len() > HANDSHAKE_MAX_LEN {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "handshake line too long",
                    ));
                }
            }
            Ok::<(), std::io::Error>(())
        },
    )
    .await
    .map_err(|_| NetworkError::ConnectionError("handshake read timeout".to_string()))?;
    read_result.map_err(NetworkError::IO)?;

    let line = String::from_utf8(line)
        .map_err(|e| NetworkError::ProtocolError(format!("invalid handshake utf8: {e}")))?;
    let mut parts = line.split_whitespace();
    let prefix = parts.next().unwrap_or_default();
    let network_id_str = parts.next().unwrap_or_default();
    let peer_id = parts.next().unwrap_or_default();
    if prefix != HANDSHAKE_PREFIX {
        return Err(NetworkError::ProtocolError(format!(
            "invalid handshake prefix: {prefix}"
        )));
    }
    if peer_id.is_empty() {
        return Err(NetworkError::ProtocolError(
            "missing peer id in handshake".to_string(),
        ));
    }
    let network_id = network_id_str.parse::<u64>().map_err(|e| {
        NetworkError::ProtocolError(format!("invalid network id in handshake: {e}"))
    })?;
    Ok((network_id, peer_id.to_string()))
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
