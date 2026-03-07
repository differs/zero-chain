//! Node discovery module using Kademlia DHT

use crate::{NetworkConfig, NetworkError, Result};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
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
    /// Network ID announced by remote peer (0 if unknown)
    pub network_id: u64,
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
            network_id: 0,
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
    buckets: Arc<RwLock<Vec<KBucket>>>,
    /// Known nodes
    nodes: Arc<RwLock<HashMap<String, NodeRecord>>>,
    /// Background running flag
    running: Arc<AtomicBool>,
    /// Background UDP task
    task: RwLock<Option<JoinHandle<()>>>,
}

impl Discovery {
    pub fn new(config: &NetworkConfig) -> Result<Self> {
        // Generate node ID from key
        let node_id = generate_node_id();

        let buckets = (0..256).map(|_| KBucket::new()).collect();

        Ok(Self {
            config: config.clone(),
            node_id,
            buckets: Arc::new(RwLock::new(buckets)),
            nodes: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(AtomicBool::new(false)),
            task: RwLock::new(None),
        })
    }

    pub async fn start(&self) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let bind_addr = format!("{}:{}", self.config.listen_addr, self.config.listen_port);
        let socket = UdpSocket::bind(&bind_addr)
            .await
            .map_err(|e| NetworkError::ConnectionError(format!("discovery bind failed: {e}")))?;

        tracing::info!("Starting discovery service on {}", bind_addr);

        let running = self.running.clone();
        let buckets = self.buckets.clone();
        let nodes = self.nodes.clone();
        let node_id = self.node_id.clone();
        let task = tokio::spawn(async move {
            let mut buf = [0u8; 512];
            while running.load(Ordering::Relaxed) {
                match socket.recv_from(&mut buf).await {
                    Ok((len, addr)) => {
                        let msg = String::from_utf8_lossy(&buf[..len]);
                        let peer_id = msg
                            .strip_prefix("zero-discovery:")
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| format!("udp-{}", addr));
                        let node = NodeRecord {
                            peer_id,
                            ip: addr.ip().to_string(),
                            tcp_port: addr.port(),
                            udp_port: addr.port(),
                            network_id: 0,
                        };
                        let _ = insert_node_record(&node_id, &buckets, &nodes, node);
                        let _ = socket.send_to(b"zero-discovery:ack", addr).await;
                    }
                    Err(err) => {
                        tracing::debug!("discovery recv error: {}", err);
                    }
                }
            }
        });
        *self.task.write() = Some(task);
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(task) = self.task.write().take() {
            task.abort();
        }
        tracing::info!("Stopping discovery service");
        Ok(())
    }

    /// Add node to routing table
    pub fn add_node(&self, node: NodeRecord) -> bool {
        insert_node_record(&self.node_id, &self.buckets, &self.nodes, node)
    }

    /// Get closest nodes to target
    pub fn get_closest_nodes(&self, target: &str, limit: usize) -> Vec<NodeRecord> {
        let distance = calculate_distance(&self.node_id, target);
        let bucket_index = 255 - distance.leading_zeros() as usize;

        let buckets = self.buckets.read();
        let mut nodes = Vec::new();

        // Collect from closest buckets
        for i in 0..256 {
            let idx = bucket_index.abs_diff(i);

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

fn insert_node_record(
    local_node_id: &str,
    buckets: &RwLock<Vec<KBucket>>,
    nodes: &RwLock<HashMap<String, NodeRecord>>,
    node: NodeRecord,
) -> bool {
    // Calculate distance and bucket index
    let distance = calculate_distance(local_node_id, &node.peer_id);
    let bucket_index = 255 - distance.leading_zeros() as usize;

    let mut buckets = buckets.write();
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
        nodes.write().insert(node.peer_id.clone(), node);
        true
    } else {
        false
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
    for (i, slot) in distance.iter_mut().enumerate() {
        let b1 = bytes1.get(i).copied().unwrap_or(0);
        let b2 = bytes2.get(i).copied().unwrap_or(0);
        *slot = b1 ^ b2;
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
