//! Node discovery module using Kademlia DHT

use crate::{NetworkConfig, NetworkError, Result};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use zerocore::crypto::{Hash, PublicKey};

/// Node record
#[derive(Clone, Debug)]
pub struct NodeRecord {
    /// Peer ID
    pub peer_id: String,
    /// IP address
    pub ip: String,
    /// TCP port
    pub tcp_port: u16,
    /// UDP port
    pub udp_port: u16,
}

impl NodeRecord {
    pub fn from_enode(enode: &str) -> Result<Self> {
        // Parse enode://pubkey@ip:port
        if !enode.starts_with("enode://") {
            return Err(NetworkError::ProtocolError("Invalid enode format".into()));
        }

        let parts: Vec<&str> = enode[8..].split('@').collect();
        if parts.len() != 2 {
            return Err(NetworkError::ProtocolError("Invalid enode format".into()));
        }

        let peer_id = parts[0].to_string();
        let addr_parts: Vec<&str> = parts[1].split(':').collect();

        if addr_parts.len() != 2 {
            return Err(NetworkError::ProtocolError("Invalid address format".into()));
        }

        let ip = addr_parts[0].to_string();
        let port = addr_parts[1].parse().unwrap_or(30303);

        Ok(Self {
            peer_id,
            ip,
            tcp_port: port,
            udp_port: port,
        })
    }

    pub fn to_enode(&self) -> String {
        format!("enode://{}@{}:{}", self.peer_id, self.ip, self.tcp_port)
    }
}

/// Kademlia bucket
#[derive(Clone, Debug)]
pub struct KBucket {
    nodes: Vec<NodeRecord>,
    last_updated: u64,
}

impl KBucket {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            last_updated: current_timestamp(),
        }
    }
}

/// Discovery service
pub struct Discovery {
    config: NetworkConfig,
    /// Local node ID
    node_id: String,
    /// Routing table (256 buckets)
    buckets: RwLock<Vec<KBucket>>,
    /// Known nodes
    nodes: RwLock<HashMap<String, NodeRecord>>,
}

impl Discovery {
    pub fn new(config: &NetworkConfig) -> Result<Self> {
        // Generate node ID from key
        let node_id = generate_node_id();

        let buckets = (0..256).map(|_| KBucket::new()).collect();

        Ok(Self {
            config: config.clone(),
            node_id,
            buckets: RwLock::new(buckets),
            nodes: RwLock::new(HashMap::new()),
        })
    }

    pub async fn start(&self) -> Result<()> {
        tracing::info!("Starting discovery service");

        // Would start UDP listener for discovery protocol
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        tracing::info!("Stopping discovery service");
        Ok(())
    }

    /// Add node to routing table
    pub fn add_node(&self, node: NodeRecord) -> bool {
        // Calculate distance and bucket index
        let distance = calculate_distance(&self.node_id, &node.peer_id);
        let bucket_index = 255 - distance.leading_zeros() as usize;

        let mut buckets = self.buckets.write();
        let bucket = &mut buckets[bucket_index];

        // Check if already exists
        if bucket.nodes.iter().any(|n| n.peer_id == node.peer_id) {
            bucket.last_updated = current_timestamp();
            return false;
        }

        // Add if bucket not full
        if bucket.nodes.len() < 20 {
            bucket.nodes.push(node.clone());
            bucket.last_updated = current_timestamp();
            self.nodes.write().insert(node.peer_id.clone(), node);
            true
        } else {
            false
        }
    }

    /// Get closest nodes to target
    pub fn get_closest_nodes(&self, target: &str, limit: usize) -> Vec<NodeRecord> {
        let distance = calculate_distance(&self.node_id, target);
        let bucket_index = 255 - distance.leading_zeros() as usize;

        let buckets = self.buckets.read();
        let mut nodes = Vec::new();

        // Collect from closest buckets
        for i in 0..256 {
            let idx = if bucket_index >= i {
                bucket_index - i
            } else {
                i - bucket_index
            };

            if idx < buckets.len() {
                for node in &buckets[idx].nodes {
                    nodes.push(node.clone());
                    if nodes.len() >= limit {
                        return nodes;
                    }
                }
            }
        }

        nodes
    }

    /// Get random nodes
    pub fn get_random_nodes(&self, limit: usize) -> Vec<NodeRecord> {
        use rand::Rng;
        let mut rng = rand::thread_rng();

        let nodes = self.nodes.read();
        let mut selected = Vec::new();

        for node in nodes.values() {
            if selected.len() >= limit {
                break;
            }

            if rng.gen_bool(0.5) {
                selected.push(node.clone());
            }
        }

        selected
    }

    /// Remove node
    pub fn remove_node(&self, peer_id: &str) {
        self.nodes.write().remove(peer_id);

        // Would also remove from bucket
    }
}

fn generate_node_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();

    let mut bytes = [0u8; 64];
    for byte in &mut bytes {
        *byte = rng.gen();
    }

    hex::encode(bytes)
}

fn calculate_distance(id1: &str, id2: &str) -> Hash {
    // XOR distance
    let bytes1 = hex::decode(id1).unwrap_or_default();
    let bytes2 = hex::decode(id2).unwrap_or_default();

    let mut distance = [0u8; 32];
    for i in 0..32 {
        let b1 = bytes1.get(i).copied().unwrap_or(0);
        let b2 = bytes2.get(i).copied().unwrap_or(0);
        distance[i] = b1 ^ b2;
    }

    Hash::from_bytes(distance)
}

fn current_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
