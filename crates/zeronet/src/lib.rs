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
pub use protocol::{
    BlockMessage, Protocol, ProtocolMessage, SyncBlockBody, SyncComputeTxRecord, SyncHeader,
    SyncStateSnapshot,
};
pub use sync::{SyncManager, SyncState};

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{interval, timeout, MissedTickBehavior};
use uuid::Uuid;
use zerocore::account::Account;
use zerocore::crypto::Hash;

static GLOBAL_PEER_COUNT: AtomicUsize = AtomicUsize::new(0);
static GLOBAL_PEER_INFOS: Lazy<RwLock<Vec<PeerInfo>>> = Lazy::new(|| RwLock::new(Vec::new()));
static GLOBAL_SYNCED_HEIGHT: AtomicU64 = AtomicU64::new(0);
static GLOBAL_BLOCKS: Lazy<RwLock<BTreeMap<u64, zerocore::block::Block>>> =
    Lazy::new(|| RwLock::new(BTreeMap::new()));
static GLOBAL_SYNC_ACCOUNTS: Lazy<RwLock<HashMap<zerocore::crypto::Address, Account>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
static GLOBAL_SYNC_COMPUTE_TXS: Lazy<RwLock<HashMap<Hash, SyncComputeTxRecord>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
static GLOBAL_SYNC_COMPUTE_ORDER: Lazy<RwLock<VecDeque<Hash>>> =
    Lazy::new(|| RwLock::new(VecDeque::new()));
static SEEN_TX_HASHES: Lazy<RwLock<HashMap<String, u64>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
static SEEN_BLOCK_HASHES: Lazy<RwLock<HashMap<String, u64>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

const HANDSHAKE_PREFIX: &str = "ZERO/1";
const HANDSHAKE_MAX_LEN: usize = 512;
const HANDSHAKE_TIMEOUT_SECS: u64 = 5;
const HEARTBEAT_PING: &[u8] = b"ZERO/PING\n";
const HEARTBEAT_PONG: &[u8] = b"ZERO/PONG\n";
const CONTROL_FRAME_MAX_LEN: usize = 16 * 1024 * 1024;
const HEARTBEAT_INTERVAL_SECS: u64 = 15;
const PEER_IDLE_TIMEOUT_SECS: u64 = 45;
const PEER_SEND_BUFFER: usize = 256;
const DEFAULT_DEDUP_TTL_SECS: u64 = 5 * 60;
const MAX_DEDUP_ENTRIES: usize = 8192;
const MAX_GLOBAL_BLOCKS: usize = 8192;
const MAX_GLOBAL_SYNC_TX_INDEX: usize = 100_000;
const DISCOVERY_DIAL_INTERVAL_SECS: u64 = 5;
const SYNC_HEAD_ANNOUNCE_INTERVAL_SECS: u64 = 10;

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

/// Returns the latest synchronized height reported by network sync.
pub fn global_synced_height() -> u64 {
    GLOBAL_SYNCED_HEIGHT.load(Ordering::Relaxed)
}

pub fn set_global_synced_height(height: u64) {
    GLOBAL_SYNCED_HEIGHT.store(height, Ordering::Relaxed);
}

/// Store a canonical block snapshot for sync/read APIs.
pub fn global_store_block(block: zerocore::block::Block) {
    let height = block.header.number.as_u64();
    let mut blocks = GLOBAL_BLOCKS.write();
    blocks.insert(height, block);
    while blocks.len() > MAX_GLOBAL_BLOCKS {
        let Some(oldest) = blocks.keys().next().copied() else {
            break;
        };
        blocks.remove(&oldest);
    }
    let prev = global_synced_height();
    if height > prev {
        set_global_synced_height(height);
    }
}

/// Read a canonical block snapshot by number.
pub fn global_block_by_number(number: u64) -> Option<zerocore::block::Block> {
    GLOBAL_BLOCKS.read().get(&number).cloned()
}

/// Read latest canonical block snapshot.
pub fn global_latest_block() -> Option<zerocore::block::Block> {
    GLOBAL_BLOCKS
        .read()
        .last_key_value()
        .map(|(_, b)| b.clone())
}

/// Resolve block number from hash in canonical snapshot cache.
pub fn global_block_number_for_hash(target: &Hash) -> Option<u64> {
    GLOBAL_BLOCKS
        .read()
        .iter()
        .find_map(|(n, b)| (b.header.hash == *target).then_some(*n))
}

/// Reset in-process sync cache (blocks + advertised synced height).
pub fn global_reset_sync_cache() {
    GLOBAL_BLOCKS.write().clear();
    GLOBAL_SYNC_ACCOUNTS.write().clear();
    GLOBAL_SYNC_COMPUTE_TXS.write().clear();
    GLOBAL_SYNC_COMPUTE_ORDER.write().clear();
    set_global_synced_height(0);
}

/// Record or update an account snapshot visible to RPC readers.
pub fn global_record_account(account: Account) {
    GLOBAL_SYNC_ACCOUNTS
        .write()
        .insert(account.address, account);
}

/// Replace account snapshot cache with the provided full snapshot.
pub fn global_replace_accounts(accounts: Vec<Account>) {
    let mut map = GLOBAL_SYNC_ACCOUNTS.write();
    map.clear();
    for account in accounts {
        map.insert(account.address, account);
    }
}

/// Read a synchronized account snapshot.
pub fn global_synced_account(address: &zerocore::crypto::Address) -> Option<Account> {
    GLOBAL_SYNC_ACCOUNTS.read().get(address).cloned()
}

/// Export synchronized account snapshot.
pub fn global_synced_accounts() -> Vec<Account> {
    GLOBAL_SYNC_ACCOUNTS.read().values().cloned().collect()
}

/// Record or update a synchronized compute tx result record.
pub fn global_record_compute_tx(record: SyncComputeTxRecord) {
    let tx_hash = record.tx_hash;
    let mut map = GLOBAL_SYNC_COMPUTE_TXS.write();
    let mut order = GLOBAL_SYNC_COMPUTE_ORDER.write();
    map.insert(tx_hash, record);
    order.retain(|h| h != &tx_hash);
    order.push_back(tx_hash);
    while order.len() > MAX_GLOBAL_SYNC_TX_INDEX {
        if let Some(stale) = order.pop_front() {
            map.remove(&stale);
        }
    }
}

/// Replace synchronized compute tx index.
pub fn global_replace_compute_txs(records: Vec<SyncComputeTxRecord>) {
    let mut map = GLOBAL_SYNC_COMPUTE_TXS.write();
    let mut order = GLOBAL_SYNC_COMPUTE_ORDER.write();
    map.clear();
    order.clear();
    for record in records {
        let tx_hash = record.tx_hash;
        map.insert(tx_hash, record);
        order.push_back(tx_hash);
    }
}

/// Read a synchronized compute tx result record.
pub fn global_synced_compute_tx(hash: &Hash) -> Option<SyncComputeTxRecord> {
    GLOBAL_SYNC_COMPUTE_TXS.read().get(hash).cloned()
}

/// Export synchronized compute tx records (oldest -> newest).
pub fn global_synced_compute_txs() -> Vec<SyncComputeTxRecord> {
    let map = GLOBAL_SYNC_COMPUTE_TXS.read();
    GLOBAL_SYNC_COMPUTE_ORDER
        .read()
        .iter()
        .filter_map(|h| map.get(h).cloned())
        .collect()
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
    /// Optional persisted banlist path.
    pub banlist_path: Option<String>,
    /// Default ban duration for abusive peers.
    pub ban_duration_secs: u64,
    /// Maximum active inbound peers accepted per source IP.
    pub max_inbound_per_ip: u32,
    /// Maximum inbound connection attempts per IP per minute.
    pub max_inbound_rate_per_minute: u32,
    /// Maximum inbound gossip frames per peer per minute.
    pub max_gossip_per_peer_per_minute: u32,
    /// Retry interval for reconnecting bootnodes.
    pub bootnode_retry_interval_secs: u64,
    /// For development/mining mode, periodically advance local sync head.
    pub sync_auto_advance: bool,
    /// Interval in seconds for auto-advancing sync head.
    pub sync_auto_advance_interval_secs: u64,
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
            banlist_path: None,
            ban_duration_secs: 10 * 60,
            max_inbound_per_ip: 8,
            max_inbound_rate_per_minute: 120,
            max_gossip_per_peer_per_minute: 240,
            bootnode_retry_interval_secs: 15,
            sync_auto_advance: false,
            sync_auto_advance_interval_secs: 3,
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
    bootnode_task: RwLock<Option<JoinHandle<()>>>,
    discovery_dial_task: RwLock<Option<JoinHandle<()>>>,
    sync_head_task: RwLock<Option<JoinHandle<()>>>,
}

impl NetworkService {
    /// Create new network service
    pub fn new(config: NetworkConfig) -> Result<Self> {
        let local_peer_id = format!("node-{}", Uuid::new_v4().simple());
        let peer_manager = Arc::new(PeerManager::new_with_policy(
            config.max_peers,
            config.banlist_path.clone().map(PathBuf::from),
            config.ban_duration_secs,
        ));

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
            bootnode_task: RwLock::new(None),
            discovery_dial_task: RwLock::new(None),
            sync_head_task: RwLock::new(None),
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

        // Connect to bootnodes immediately
        if !self.config.bootnodes.is_empty() {
            self.connect_bootnodes_once().await;
            self.start_bootnode_reconnector();
        }

        // Start discovery
        if let Some(discovery) = &self.discovery {
            discovery.start().await?;
            if let Some(local_enr) = discovery.local_enr_base64() {
                tracing::info!("discovery local ENR: {}", local_enr);
            }
            self.start_discovery_dialer(discovery.clone());
        }

        // Start sync
        if let Some(sync) = &self.sync_manager {
            sync.start_default().await?;
            if self.config.sync_auto_advance {
                self.start_sync_head_advancer(sync.clone());
            }
        }

        *self.is_running.write() = true;
        set_global_peer_count(self.peer_manager.peer_count());
        set_global_peers(self.peer_manager.get_active_peer_infos());

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
        if let Some(task) = self.bootnode_task.write().take() {
            task.abort();
        }
        if let Some(task) = self.discovery_dial_task.write().take() {
            task.abort();
        }
        if let Some(task) = self.sync_head_task.write().take() {
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

    /// Broadcast compute transaction hash to all peers
    pub fn broadcast_compute_tx(&self, tx_hash: Hash) {
        let message = ProtocolMessage::NewComputeTx(tx_hash);
        self.broadcast_with_backpressure(message);
    }

    /// Broadcast block to all peers
    pub fn broadcast_block(&self, block: zerocore::block::Block) {
        let message = ProtocolMessage::NewBlock(Box::new(block));
        self.broadcast_with_backpressure(message);
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
            set_global_peers(self.peer_manager.get_active_peer_infos());
        }
        result
    }

    /// Remove peer
    pub fn remove_peer(&self, peer_id: &str) -> Result<()> {
        let result = self.peer_manager.remove_peer(peer_id);
        if result.is_ok() {
            set_global_peer_count(self.peer_manager.peer_count());
            set_global_peers(self.peer_manager.get_active_peer_infos());
        }
        result
    }

    fn broadcast_with_backpressure(&self, message: ProtocolMessage) {
        let mut dropped = Vec::new();
        for peer in self.peer_manager.get_active_peers() {
            if peer.send(message.clone()).is_err() {
                dropped.push(peer.info.peer_id.clone());
            }
        }

        if dropped.is_empty() {
            return;
        }

        for peer_id in dropped {
            let _ = self.peer_manager.remove_peer(&peer_id);
            tracing::warn!("dropped overloaded peer from gossip path: {}", peer_id);
        }
        set_global_peer_count(self.peer_manager.peer_count());
        set_global_peers(self.peer_manager.get_active_peer_infos());
    }

    async fn start_listening(&self) -> Result<()> {
        let bind_addr = format!("{}:{}", self.config.listen_addr, self.config.listen_port);
        let listener = TcpListener::bind(&bind_addr)
            .await
            .map_err(|e| NetworkError::ConnectionError(format!("bind {bind_addr} failed: {e}")))?;

        let expected_network_id = self.config.network_id;
        let local_peer_id = self.local_peer_id.clone();
        let peer_manager = self.peer_manager.clone();
        let max_inbound_per_ip = self.config.max_inbound_per_ip.max(1);
        let max_inbound_rate_per_minute = self.config.max_inbound_rate_per_minute.max(1);
        let max_gossip_per_peer_per_minute = self.config.max_gossip_per_peer_per_minute.max(1);
        let ban_duration_secs = self.config.ban_duration_secs;
        let sync_manager = self.sync_manager.clone();

        let task = tokio::spawn(async move {
            let mut inbound_windows: HashMap<String, VecDeque<u64>> = HashMap::new();
            tracing::info!("P2P listener started on {}", bind_addr);
            loop {
                match listener.accept().await {
                    Ok((mut stream, remote_addr)) => {
                        peer_manager.cleanup_expired_bans();
                        let remote_ip = remote_addr.ip().to_string();

                        if peer_manager.is_ip_banned(&remote_ip) {
                            tracing::warn!("drop inbound from banned ip {}", remote_addr);
                            continue;
                        }

                        if peer_manager.connected_peers_for_ip(&remote_ip)
                            >= max_inbound_per_ip as usize
                        {
                            tracing::warn!(
                                "ip {} exceeded max inbound peers ({})",
                                remote_ip,
                                max_inbound_per_ip
                            );
                            peer_manager.ban_ip(&remote_ip, ban_duration_secs.min(300));
                            continue;
                        }

                        if !allow_ip_rate(
                            &mut inbound_windows,
                            &remote_ip,
                            max_inbound_rate_per_minute,
                            current_timestamp(),
                        ) {
                            tracing::warn!(
                                "ip {} exceeded inbound connection rate ({} / min)",
                                remote_ip,
                                max_inbound_rate_per_minute
                            );
                            peer_manager.ban_ip(&remote_ip, ban_duration_secs.min(180));
                            continue;
                        }

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

                        let (tx, rx) = mpsc::channel(PEER_SEND_BUFFER);
                        match peer_manager.add_peer_with_sender(node_record, tx) {
                            Ok(inserted) => {
                                if !inserted {
                                    tracing::debug!(
                                        "skipping duplicate inbound peer {} from {}",
                                        remote_peer_id,
                                        remote_addr
                                    );
                                    continue;
                                }
                            }
                            Err(err) => {
                                tracing::warn!(
                                    "failed to register inbound peer {}: {}",
                                    remote_addr,
                                    err
                                );
                                continue;
                            }
                        }

                        set_global_peer_count(peer_manager.peer_count());
                        set_global_peers(peer_manager.get_active_peer_infos());
                        tokio::spawn(monitor_peer_socket(
                            peer_manager.clone(),
                            remote_peer_id,
                            stream,
                            rx,
                            ban_duration_secs,
                            max_gossip_per_peer_per_minute,
                            sync_manager.clone(),
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

    fn start_bootnode_reconnector(&self) {
        if self.config.bootnodes.is_empty() {
            return;
        }

        let bootnodes = self.config.bootnodes.clone();
        let expected_network_id = self.config.network_id;
        let local_peer_id = self.local_peer_id.clone();
        let retry_secs = self.config.bootnode_retry_interval_secs.max(3);
        let peer_manager = self.peer_manager.clone();
        let ban_duration_secs = self.config.ban_duration_secs;
        let max_gossip_per_peer_per_minute = self.config.max_gossip_per_peer_per_minute.max(1);
        let sync_manager = self.sync_manager.clone();

        let task = tokio::spawn(async move {
            let mut ticker = interval(std::time::Duration::from_secs(retry_secs));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                peer_manager.cleanup_expired_bans();
                for bootnode in &bootnodes {
                    if let Err(err) = connect_single_bootnode(
                        bootnode,
                        expected_network_id,
                        &local_peer_id,
                        peer_manager.clone(),
                        ban_duration_secs,
                        max_gossip_per_peer_per_minute,
                        sync_manager.clone(),
                    )
                    .await
                    {
                        tracing::debug!("bootnode reconnect skipped {}: {}", bootnode, err);
                    }
                }
            }
        });

        *self.bootnode_task.write() = Some(task);
    }

    async fn connect_bootnodes_once(&self) {
        for bootnode in &self.config.bootnodes {
            if let Err(err) = connect_single_bootnode(
                bootnode,
                self.config.network_id,
                &self.local_peer_id,
                self.peer_manager.clone(),
                self.config.ban_duration_secs,
                self.config.max_gossip_per_peer_per_minute.max(1),
                self.sync_manager.clone(),
            )
            .await
            {
                tracing::warn!("Failed to connect bootnode {}: {}", bootnode, err);
            }
        }
    }

    fn start_discovery_dialer(&self, discovery: Arc<Discovery>) {
        let peer_manager = self.peer_manager.clone();
        let expected_network_id = self.config.network_id;
        let local_peer_id = self.local_peer_id.clone();
        let ban_duration_secs = self.config.ban_duration_secs;
        let max_gossip_per_peer_per_minute = self.config.max_gossip_per_peer_per_minute.max(1);
        let sync_manager = self.sync_manager.clone();

        let task = tokio::spawn(async move {
            let mut ticker = interval(std::time::Duration::from_secs(DISCOVERY_DIAL_INTERVAL_SECS));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                for node in discovery.get_random_nodes(32) {
                    if node.network_id != 0 && node.network_id != expected_network_id {
                        continue;
                    }
                    if let Err(err) = connect_node_record(
                        node,
                        expected_network_id,
                        &local_peer_id,
                        peer_manager.clone(),
                        ban_duration_secs,
                        max_gossip_per_peer_per_minute,
                        sync_manager.clone(),
                    )
                    .await
                    {
                        tracing::debug!("discovery dial skipped: {}", err);
                    }
                }
            }
        });

        *self.discovery_dial_task.write() = Some(task);
    }

    fn start_sync_head_advancer(&self, sync: Arc<SyncManager>) {
        let interval_secs = self.config.sync_auto_advance_interval_secs.max(1);
        let task = tokio::spawn(async move {
            let mut ticker = interval(std::time::Duration::from_secs(interval_secs));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let head = sync.bump_local_height(1);
                tracing::debug!("sync auto-advanced local head to {}", head);
            }
        });
        *self.sync_head_task.write() = Some(task);
    }
}

async fn connect_single_bootnode(
    bootnode: &str,
    expected_network_id: u64,
    local_peer_id: &str,
    peer_manager: Arc<PeerManager>,
    ban_duration_secs: u64,
    max_gossip_per_peer_per_minute: u32,
    sync_manager: Option<Arc<SyncManager>>,
) -> Result<()> {
    let record = NodeRecord::from_bootnode(bootnode, expected_network_id)?;
    connect_node_record(
        record,
        expected_network_id,
        local_peer_id,
        peer_manager,
        ban_duration_secs,
        max_gossip_per_peer_per_minute,
        sync_manager,
    )
    .await
}

async fn connect_node_record(
    record: NodeRecord,
    expected_network_id: u64,
    local_peer_id: &str,
    peer_manager: Arc<PeerManager>,
    ban_duration_secs: u64,
    max_gossip_per_peer_per_minute: u32,
    sync_manager: Option<Arc<SyncManager>>,
) -> Result<()> {
    if peer_manager.get_peer(&record.peer_id).is_some() {
        return Ok(());
    }
    if peer_manager.get_active_peer_infos().iter().any(|peer| {
        peer.remote_addr.ip().to_string() == record.ip && peer.remote_addr.port() == record.tcp_port
    }) {
        return Ok(());
    }

    if peer_manager.is_ip_banned(&record.ip) {
        return Err(NetworkError::ConnectionError(format!(
            "bootnode ip {} is banned",
            record.ip
        )));
    }

    let addr = format!("{}:{}", record.ip, record.tcp_port);
    let mut stream =
        tokio::time::timeout(std::time::Duration::from_secs(5), TcpStream::connect(&addr))
            .await
            .map_err(|_| {
                NetworkError::ConnectionError(format!(
                    "connect timeout: {}:{}",
                    record.ip, record.tcp_port
                ))
            })?
            .map_err(|e| {
                NetworkError::ConnectionError(format!(
                    "connect failed {}:{}: {e}",
                    record.ip, record.tcp_port
                ))
            })?;

    let (remote_network_id, remote_peer_id) =
        outbound_handshake(&mut stream, expected_network_id, local_peer_id).await?;

    let node_record = NodeRecord {
        peer_id: remote_peer_id.clone(),
        ip: record.ip,
        tcp_port: record.tcp_port,
        udp_port: record.udp_port,
        network_id: remote_network_id,
    };

    if peer_manager.get_peer(&remote_peer_id).is_some() {
        tracing::debug!("skipping duplicate outbound peer {}", remote_peer_id);
        return Ok(());
    }

    let (tx, rx) = mpsc::channel(PEER_SEND_BUFFER);
    let inserted = peer_manager.add_peer_with_sender(node_record, tx)?;
    if !inserted {
        tracing::debug!(
            "skipping duplicate outbound registration {}",
            remote_peer_id
        );
        return Ok(());
    }
    set_global_peer_count(peer_manager.peer_count());
    set_global_peers(peer_manager.get_active_peer_infos());

    tokio::spawn(monitor_peer_socket(
        peer_manager,
        remote_peer_id,
        stream,
        rx,
        ban_duration_secs,
        max_gossip_per_peer_per_minute,
        sync_manager,
    ));

    Ok(())
}

async fn monitor_peer_socket(
    peer_manager: Arc<PeerManager>,
    peer_id: String,
    mut stream: TcpStream,
    mut outbound_rx: mpsc::Receiver<ProtocolMessage>,
    ban_duration_secs: u64,
    max_gossip_per_peer_per_minute: u32,
    sync_manager: Option<Arc<SyncManager>>,
) {
    let _ = peer_manager.touch_peer(&peer_id);
    let mut heartbeat = interval(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut sync_head_announce = interval(std::time::Duration::from_secs(
        SYNC_HEAD_ANNOUNCE_INTERVAL_SECS,
    ));
    sync_head_announce.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut inbound_window: VecDeque<u64> = VecDeque::new();

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
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
            _ = sync_head_announce.tick() => {
                if let Some(sync) = &sync_manager {
                    if let Err(err) = write_protocol_message(
                        &mut stream,
                        ProtocolMessage::AnnounceHead(sync.local_height()),
                    )
                    .await
                    {
                        tracing::debug!("sync head announce failed for {}: {}", peer_id, err);
                        break;
                    }
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
                    Ok(ControlFrame::ComputeTx(tx_hash)) => {
                        let now = current_timestamp();
                        if !allow_rate_window(&mut inbound_window, max_gossip_per_peer_per_minute, now) {
                            tracing::warn!("peer {} exceeded gossip rate limit", peer_id);
                            peer_manager.ban_peer(&peer_id, ban_duration_secs.min(300));
                            break;
                        }
                        let _ = peer_manager.touch_peer(&peer_id);
                        if mark_seen_hash(&SEEN_TX_HASHES, hash_to_hex(&tx_hash), now) {
                            peer_manager.broadcast_except(
                                &peer_id,
                                ProtocolMessage::NewComputeTx(tx_hash),
                            );
                        }
                    }
                    Ok(ControlFrame::BlockHash(block_hash)) => {
                        let now = current_timestamp();
                        if !allow_rate_window(&mut inbound_window, max_gossip_per_peer_per_minute, now) {
                            tracing::warn!("peer {} exceeded gossip rate limit", peer_id);
                            peer_manager.ban_peer(&peer_id, ban_duration_secs.min(300));
                            break;
                        }
                        let _ = peer_manager.touch_peer(&peer_id);
                        if mark_seen_hash(&SEEN_BLOCK_HASHES, hash_to_hex(&block_hash), now) {
                            peer_manager.broadcast_except(&peer_id, ProtocolMessage::NewBlockHash(block_hash));
                        }
                    }
                    Ok(ControlFrame::Head(height)) => {
                        let _ = peer_manager.touch_peer(&peer_id);
                        let _ = peer_manager.update_peer_height(&peer_id, height);
                    }
                    Ok(ControlFrame::SyncGetHeaders { start, limit }) => {
                        let _ = peer_manager.touch_peer(&peer_id);
                        if let Some(sync) = &sync_manager {
                            let headers = sync.build_headers_response(start, limit);
                            let _ = write_protocol_message(&mut stream, ProtocolMessage::SyncHeaders(headers)).await;
                        }
                    }
                    Ok(ControlFrame::SyncHeaders(headers)) => {
                        let _ = peer_manager.touch_peer(&peer_id);
                        if let Some(sync) = &sync_manager {
                            sync.handle_sync_headers(peer_id.clone(), headers);
                        }
                    }
                    Ok(ControlFrame::SyncGetBlockBody { block_hash }) => {
                        let _ = peer_manager.touch_peer(&peer_id);
                        if let Some(sync) = &sync_manager {
                            if let Some(body) = sync.build_block_body_response(&block_hash) {
                                let _ = write_protocol_message(&mut stream, ProtocolMessage::SyncBlockBody(body)).await;
                            }
                        }
                    }
                    Ok(ControlFrame::SyncBlockBody(body)) => {
                        let _ = peer_manager.touch_peer(&peer_id);
                        if let Some(sync) = &sync_manager {
                            sync.handle_sync_block_body(peer_id.clone(), body);
                        }
                    }
                    Ok(ControlFrame::SyncGetStateSnapshot { block_number }) => {
                        let _ = peer_manager.touch_peer(&peer_id);
                        if let Some(sync) = &sync_manager {
                            if let Some(snapshot) = sync.build_state_snapshot_response(block_number) {
                                let _ = write_protocol_message(
                                    &mut stream,
                                    ProtocolMessage::SyncStateSnapshot(snapshot),
                                )
                                .await;
                            }
                        }
                    }
                    Ok(ControlFrame::SyncStateSnapshot(snapshot)) => {
                        let _ = peer_manager.touch_peer(&peer_id);
                        if let Some(sync) = &sync_manager {
                            sync.handle_sync_state_snapshot(peer_id.clone(), snapshot);
                        }
                    }
                    Ok(ControlFrame::Disconnect(reason)) => {
                        tracing::debug!("peer {} requested disconnect: {}", peer_id, reason);
                        break;
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
            outbound = outbound_rx.recv() => {
                match outbound {
                    Some(message) => {
                        if let Err(err) = write_protocol_message(&mut stream, message).await {
                            tracing::debug!("write protocol message to {} failed: {}", peer_id, err);
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    let _ = peer_manager.remove_peer(&peer_id);
    set_global_peer_count(peer_manager.peer_count());
    set_global_peers(peer_manager.get_active_peer_infos());
}

enum ControlFrame {
    Ping,
    Pong,
    ComputeTx(Hash),
    BlockHash(Hash),
    Head(u64),
    SyncGetHeaders { start: u64, limit: u64 },
    SyncHeaders(Vec<SyncHeader>),
    SyncGetBlockBody { block_hash: Hash },
    SyncBlockBody(SyncBlockBody),
    SyncGetStateSnapshot { block_number: u64 },
    SyncStateSnapshot(SyncStateSnapshot),
    Disconnect(String),
    Other(String),
    Eof,
}

async fn read_control_frame(stream: &mut TcpStream) -> std::io::Result<ControlFrame> {
    let mut line = Vec::with_capacity(64);
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
    if normalized == "ZERO/PING" {
        return Ok(ControlFrame::Ping);
    }
    if normalized == "ZERO/PONG" {
        return Ok(ControlFrame::Pong);
    }
    if let Some(raw) = normalized.strip_prefix("ZERO/COMPUTE_TX ") {
        if let Some(hash) = parse_hash(raw.trim()) {
            return Ok(ControlFrame::ComputeTx(hash));
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid compute tx hash frame",
        ));
    }
    if let Some(raw) = normalized.strip_prefix("ZERO/GET_HEADERS ") {
        let mut parts = raw.split_whitespace();
        let start = parts
            .next()
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "missing headers start")
            })?
            .parse::<u64>()
            .map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid headers start: {err}"),
                )
            })?;
        let limit = parts
            .next()
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "missing headers limit")
            })?
            .parse::<u64>()
            .map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid headers limit: {err}"),
                )
            })?;
        return Ok(ControlFrame::SyncGetHeaders { start, limit });
    }
    if let Some(raw) = normalized.strip_prefix("ZERO/HEADERS ") {
        return Ok(ControlFrame::SyncHeaders(parse_sync_headers(raw).map_err(
            |err| std::io::Error::new(std::io::ErrorKind::InvalidData, err),
        )?));
    }
    if let Some(raw) = normalized.strip_prefix("ZERO/GET_BLOCK_BODY ") {
        if let Some(block_hash) = parse_hash(raw.trim()) {
            return Ok(ControlFrame::SyncGetBlockBody { block_hash });
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid get block body hash frame",
        ));
    }
    if let Some(raw) = normalized.strip_prefix("ZERO/BLOCK_BODY_V2 ") {
        return Ok(ControlFrame::SyncBlockBody(
            decode_sync_payload(raw).map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid block body payload: {err}"),
                )
            })?,
        ));
    }
    if let Some(raw) = normalized.strip_prefix("ZERO/GET_STATE_SNAPSHOT ") {
        let block_number = raw.trim().parse::<u64>().map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid state snapshot block number: {err}"),
            )
        })?;
        return Ok(ControlFrame::SyncGetStateSnapshot { block_number });
    }
    if let Some(raw) = normalized.strip_prefix("ZERO/STATE_SNAPSHOT_V2 ") {
        return Ok(ControlFrame::SyncStateSnapshot(
            decode_sync_payload(raw).map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid state snapshot payload: {err}"),
                )
            })?,
        ));
    }
    if let Some(raw) = normalized.strip_prefix("ZERO/BLOCK ") {
        if let Some(hash) = parse_hash(raw.trim()) {
            return Ok(ControlFrame::BlockHash(hash));
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid block hash frame",
        ));
    }
    if let Some(raw) = normalized.strip_prefix("ZERO/HEAD ") {
        let height = raw.trim().parse::<u64>().map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid head height: {err}"),
            )
        })?;
        return Ok(ControlFrame::Head(height));
    }
    if let Some(reason) = normalized.strip_prefix("ZERO/DISCONNECT ") {
        return Ok(ControlFrame::Disconnect(reason.to_string()));
    }

    Ok(ControlFrame::Other(normalized.to_string()))
}

async fn write_protocol_message(
    stream: &mut TcpStream,
    message: ProtocolMessage,
) -> std::io::Result<()> {
    let maybe_line = match message {
        ProtocolMessage::Disconnect(reason) => {
            Some(format!("ZERO/DISCONNECT {}\n", sanitize_line(&reason)))
        }
        ProtocolMessage::NewComputeTx(tx_hash) => {
            Some(format!("ZERO/COMPUTE_TX {}\n", hash_to_hex(&tx_hash)))
        }
        ProtocolMessage::NewBlock(block) => {
            Some(format!("ZERO/BLOCK {}\n", hash_to_hex(&block.header.hash)))
        }
        ProtocolMessage::NewBlockHash(block_hash) => {
            Some(format!("ZERO/BLOCK {}\n", hash_to_hex(&block_hash)))
        }
        ProtocolMessage::AnnounceHead(height) => Some(format!("ZERO/HEAD {}\n", height)),
        ProtocolMessage::GetBlock(block_hash) => {
            Some(format!("ZERO/GETBLOCK {}\n", hash_to_hex(&block_hash)))
        }
        ProtocolMessage::SyncGetHeaders { start, limit } => {
            Some(format!("ZERO/GET_HEADERS {} {}\n", start, limit))
        }
        ProtocolMessage::SyncHeaders(headers) => {
            Some(format!("ZERO/HEADERS {}\n", format_sync_headers(&headers)))
        }
        ProtocolMessage::SyncGetBlockBody { block_hash } => Some(format!(
            "ZERO/GET_BLOCK_BODY {}\n",
            hash_to_hex(&block_hash)
        )),
        ProtocolMessage::SyncBlockBody(body) => Some(format!(
            "ZERO/BLOCK_BODY_V2 {}\n",
            encode_sync_payload(&body)?
        )),
        ProtocolMessage::SyncGetStateSnapshot { block_number } => {
            Some(format!("ZERO/GET_STATE_SNAPSHOT {}\n", block_number))
        }
        ProtocolMessage::SyncStateSnapshot(snapshot) => Some(format!(
            "ZERO/STATE_SNAPSHOT_V2 {}\n",
            encode_sync_payload(&snapshot)?
        )),
        ProtocolMessage::Block(_) => None,
    };

    if let Some(line) = maybe_line {
        stream.write_all(line.as_bytes()).await?;
    }
    Ok(())
}

fn sanitize_line(input: &str) -> String {
    input
        .chars()
        .filter(|c| *c != '\n' && *c != '\r')
        .take(256)
        .collect()
}

fn parse_hash(raw: &str) -> Option<Hash> {
    let bytes = hex::decode(raw.strip_prefix("0x").unwrap_or(raw)).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Some(Hash::from_bytes(out))
}

fn parse_u64_decimal_or_hex(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        if hex.is_empty() {
            return Some(0);
        }
        return u64::from_str_radix(hex, 16).ok();
    }
    trimmed.parse::<u64>().ok()
}

fn hash_to_hex(hash: &Hash) -> String {
    format!("0x{}", hex::encode(hash.as_bytes()))
}

fn format_sync_headers(headers: &[SyncHeader]) -> String {
    headers
        .iter()
        .map(|header| {
            format!(
                "{}@{}@{}@{}@0x{:x}@{}@0x{}@{}@{}",
                header.number,
                hash_to_hex(&header.hash),
                hash_to_hex(&header.parent_hash),
                header.timestamp,
                header.difficulty,
                header.nonce,
                header.coinbase.to_hex(),
                hash_to_hex(&header.mix_hash),
                hex::encode(&header.extra_data),
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_sync_headers(raw: &str) -> std::result::Result<Vec<SyncHeader>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    trimmed
        .split(',')
        .map(|item| {
            let mut parts = item.split('@');
            let number = parts
                .next()
                .ok_or_else(|| "missing header number".to_string())?
                .parse::<u64>()
                .map_err(|e| format!("invalid header number: {e}"))?;
            let hash = parse_hash(
                parts
                    .next()
                    .ok_or_else(|| "missing header hash".to_string())?,
            )
            .ok_or_else(|| "invalid header hash".to_string())?;
            let parent_hash = parse_hash(
                parts
                    .next()
                    .ok_or_else(|| "missing header parent hash".to_string())?,
            )
            .ok_or_else(|| "invalid header parent hash".to_string())?;
            let timestamp = parts
                .next()
                .ok_or_else(|| "missing header timestamp".to_string())?
                .parse::<u64>()
                .map_err(|e| format!("invalid header timestamp: {e}"))?;
            let difficulty = parse_u64_decimal_or_hex(
                parts
                    .next()
                    .ok_or_else(|| "missing difficulty".to_string())?,
            )
            .ok_or_else(|| "invalid difficulty".to_string())?;
            let nonce = parts
                .next()
                .ok_or_else(|| "missing header nonce".to_string())?
                .parse::<u64>()
                .map_err(|e| format!("invalid header nonce: {e}"))?;
            let coinbase_raw = parts
                .next()
                .ok_or_else(|| "missing header coinbase".to_string())?;
            let coinbase = zerocore::crypto::Address::from_hex(coinbase_raw)
                .map_err(|_| "invalid header coinbase".to_string())?;
            let mix_hash = parse_hash(
                parts
                    .next()
                    .ok_or_else(|| "missing header mix hash".to_string())?,
            )
            .ok_or_else(|| "invalid header mix hash".to_string())?;
            let extra_data = match parts.next() {
                Some(raw_extra) if !raw_extra.is_empty() => {
                    hex::decode(raw_extra).map_err(|e| format!("invalid header extra data: {e}"))?
                }
                _ => Vec::new(),
            };
            Ok(SyncHeader {
                number,
                hash,
                parent_hash,
                timestamp,
                difficulty,
                nonce,
                coinbase,
                mix_hash,
                extra_data,
            })
        })
        .collect::<std::result::Result<Vec<_>, String>>()
}

fn encode_sync_payload<T: serde::Serialize>(payload: &T) -> std::io::Result<String> {
    let bytes = serde_json::to_vec(payload).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("sync payload serialization failed: {err}"),
        )
    })?;
    Ok(hex::encode(bytes))
}

fn decode_sync_payload<T: DeserializeOwned>(raw: &str) -> std::result::Result<T, String> {
    let bytes = hex::decode(raw.trim()).map_err(|e| format!("payload hex decode failed: {e}"))?;
    serde_json::from_slice::<T>(&bytes).map_err(|e| format!("payload json decode failed: {e}"))
}

fn allow_ip_rate(
    windows: &mut HashMap<String, VecDeque<u64>>,
    ip: &str,
    limit_per_minute: u32,
    now: u64,
) -> bool {
    let window = windows.entry(ip.to_string()).or_default();
    allow_rate_window(window, limit_per_minute, now)
}

fn allow_rate_window(window: &mut VecDeque<u64>, limit_per_minute: u32, now: u64) -> bool {
    while let Some(ts) = window.front() {
        if now.saturating_sub(*ts) > 60 {
            window.pop_front();
        } else {
            break;
        }
    }

    if window.len() >= limit_per_minute as usize {
        return false;
    }

    window.push_back(now);
    true
}

fn mark_seen_hash(seen: &Lazy<RwLock<HashMap<String, u64>>>, key: String, now: u64) -> bool {
    let mut store = seen.write();
    store.retain(|_, ts| now.saturating_sub(*ts) <= DEFAULT_DEDUP_TTL_SECS);
    if store.contains_key(&key) {
        return false;
    }

    if store.len() >= MAX_DEDUP_ENTRIES {
        // Drop oldest half to keep memory bounded.
        let mut items = store
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect::<Vec<_>>();
        items.sort_by_key(|(_, ts)| *ts);
        for (k, _) in items.into_iter().take(MAX_DEDUP_ENTRIES / 2) {
            store.remove(&k);
        }
    }

    store.insert(key, now);
    true
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
