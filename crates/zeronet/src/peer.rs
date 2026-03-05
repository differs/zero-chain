//! Peer management module

use crate::{NetworkConfig, NetworkError, Result};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use zerocore::crypto::Hash;

/// Peer ID
pub type PeerId = String;

/// Peer information
#[derive(Clone, Debug)]
pub struct PeerInfo {
    /// Peer ID
    pub peer_id: PeerId,
    /// Client version
    pub client_version: String,
    /// Protocol version
    pub protocol_version: u32,
    /// Network ID
    pub network_id: u64,
    /// Remote address
    pub remote_addr: SocketAddr,
    /// Local address
    pub local_addr: SocketAddr,
    /// Capabilities
    pub capabilities: Vec<String>,
    /// Connection time
    pub connected_at: u64,
    /// Last activity
    pub last_activity: u64,
    /// Reputation score
    pub reputation: i32,
}

impl PeerInfo {
    pub fn new(
        peer_id: PeerId,
        remote_addr: SocketAddr,
        local_addr: SocketAddr,
        network_id: u64,
    ) -> Self {
        let now = current_timestamp();

        Self {
            peer_id,
            client_version: "Unknown".to_string(),
            protocol_version: 1,
            network_id,
            remote_addr,
            local_addr,
            capabilities: Vec::new(),
            connected_at: now,
            last_activity: now,
            reputation: 100,
        }
    }

    /// Update last activity
    pub fn update_activity(&mut self) {
        self.last_activity = current_timestamp();
    }

    /// Increase reputation
    pub fn increase_reputation(&mut self, amount: i32) {
        self.reputation = (self.reputation + amount).min(1000);
    }

    /// Decrease reputation
    pub fn decrease_reputation(&mut self, amount: i32) {
        self.reputation = (self.reputation - amount).max(-1000);
    }
}

/// Peer connection status
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerStatus {
    Connecting,
    Connected,
    Disconnecting,
    Disconnected,
}

/// Peer connection
pub struct Peer {
    /// Peer info
    pub info: PeerInfo,
    /// Status
    pub status: PeerStatus,
    /// Send channel
    tx: mpsc::Sender<ProtocolMessage>,
    /// Known transactions
    known_transactions: RwLock<HashSet<Hash>>,
}

use crate::protocol::ProtocolMessage;
use std::collections::HashSet;

impl Peer {
    /// Create new peer
    pub fn new(info: PeerInfo, tx: mpsc::Sender<ProtocolMessage>) -> Self {
        Self {
            info,
            status: PeerStatus::Connected,
            tx,
            known_transactions: RwLock::new(HashSet::new()),
        }
    }

    /// Send message to peer
    pub fn send(&self, message: ProtocolMessage) -> Result<()> {
        self.tx
            .try_send(message)
            .map_err(|_| NetworkError::ChannelError)?;

        Ok(())
    }

    /// Check if peer knows transaction
    pub fn knows_transaction(&self, tx_hash: &Hash) -> bool {
        self.known_transactions.read().contains(tx_hash)
    }

    /// Mark transaction as known
    pub fn mark_transaction_known(&self, tx_hash: Hash) {
        let mut known = self.known_transactions.write();

        // Limit size
        if known.len() > 10000 {
            known.clear();
        }

        known.insert(tx_hash);
    }

    /// Update activity timestamp
    pub fn update_activity(&self) {
        // Would update in peer manager
    }
}

/// Peer manager
pub struct PeerManager {
    /// Maximum peers
    max_peers: u32,
    /// Connected peers
    peers: RwLock<HashMap<PeerId, Arc<Peer>>>,
    /// Peer scores
    scores: RwLock<HashMap<PeerId, i32>>,
}

impl PeerManager {
    /// Create new peer manager
    pub fn new(max_peers: u32) -> Self {
        Self {
            max_peers,
            peers: RwLock::new(HashMap::new()),
            scores: RwLock::new(HashMap::new()),
        }
    }

    /// Add peer
    pub fn add_peer(&self, node_record: crate::discovery::NodeRecord) -> Result<()> {
        if self.peers.read().len() >= self.max_peers as usize {
            return Err(NetworkError::ConnectionError("Max peers reached".into()));
        }

        let peer_id = node_record.peer_id.clone();

        // Would create actual peer connection
        // For now, just a placeholder

        Ok(())
    }

    /// Remove peer
    pub fn remove_peer(&self, peer_id: &str) -> Result<()> {
        self.peers.write().remove(peer_id);
        self.scores.write().remove(peer_id);
        Ok(())
    }

    /// Get peer by ID
    pub fn get_peer(&self, peer_id: &str) -> Option<Arc<Peer>> {
        self.peers.read().get(peer_id).cloned()
    }

    /// Get all active peers
    pub fn get_active_peers(&self) -> Vec<PeerInfo> {
        self.peers
            .read()
            .values()
            .filter(|p| p.status == PeerStatus::Connected)
            .map(|p| p.info.clone())
            .collect()
    }

    /// Get peer count
    pub fn peer_count(&self) -> usize {
        self.peers.read().len()
    }

    /// Disconnect all peers
    pub fn disconnect_all_peers(&self) {
        for peer in self.peers.read().values() {
            let _ = peer.send(ProtocolMessage::Disconnect("Shutting down".into()));
        }

        self.peers.write().clear();
    }

    /// Get best peers for sync (by score)
    pub fn get_best_peers(&self, limit: usize) -> Vec<Arc<Peer>> {
        let mut peers: Vec<_> = self.peers.read().values().cloned().collect();

        // Sort by score
        peers.sort_by(|a, b| {
            let score_a = self
                .scores
                .read()
                .get(&a.info.peer_id)
                .copied()
                .unwrap_or(0);
            let score_b = self
                .scores
                .read()
                .get(&b.info.peer_id)
                .copied()
                .unwrap_or(0);
            score_b.cmp(&score_a)
        });

        peers.into_iter().take(limit).collect()
    }

    /// Update peer score
    pub fn update_score(&self, peer_id: &str, delta: i32) {
        let mut scores = self.scores.write();
        let score = scores.entry(peer_id.to_string()).or_insert(0);
        *score = (*score + delta).clamp(-1000, 1000);
    }

    /// Ban peer
    pub fn ban_peer(&self, peer_id: &str, duration_secs: u64) {
        // Would add to ban list
        let _ = self.remove_peer(peer_id);

        tracing::info!("Banned peer {} for {} seconds", peer_id, duration_secs);
    }

    /// Check if peer is banned
    pub fn is_banned(&self, peer_id: &str) -> bool {
        // Would check ban list
        false
    }
}

fn current_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_manager() {
        let manager = PeerManager::new(10);

        assert_eq!(manager.peer_count(), 0);

        // Would add peers in real test
    }

    #[test]
    fn test_peer_info() {
        let mut info = PeerInfo::new(
            "test_peer".into(),
            "127.0.0.1:30303".parse().unwrap(),
            "127.0.0.1:8080".parse().unwrap(),
            10086,
        );

        assert_eq!(info.reputation, 100);

        info.increase_reputation(50);
        assert_eq!(info.reputation, 150);

        info.decrease_reputation(200);
        assert_eq!(info.reputation, -50);
    }
}
