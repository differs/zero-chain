//! Run command - Full node implementation

use crate::Result;
use zeroapi::rpc::RpcConfig;
use zeroapi::{ApiConfig, ApiService};
use zerocore::account::U256;
use zerocore::block::create_genesis_block;
use zerocore::consensus::PowConsensus;
use zerocore::crypto::Hash;
use zeronet::{NetworkConfig, NetworkService};

pub async fn run_node(
    mine: bool,
    coinbase: Option<String>,
    http_port: u16,
    ws_port: u16,
    data_dir: &str,
    rpc_config: Option<RpcConfig>,
    p2p_listen_addr: String,
    p2p_listen_port: u16,
    bootnodes: Vec<String>,
    max_peers: u32,
    enable_discovery: bool,
    enable_sync: bool,
    p2p_banlist_path: Option<String>,
    p2p_ban_duration_secs: u64,
    p2p_max_inbound_per_ip: u32,
    p2p_max_inbound_rate_per_minute: u32,
    p2p_max_gossip_per_peer_per_minute: u32,
    p2p_bootnode_retry_interval_secs: u64,
) -> Result<()> {
    println!("🚀 Starting ZeroChain node...");
    println!("   Data directory: {}", data_dir);
    println!("   HTTP RPC port: {}", http_port);
    println!("   WebSocket port: {}", ws_port);
    println!("   Mining: {}", if mine { "enabled" } else { "disabled" });
    println!("   P2P listen: {}:{}", p2p_listen_addr, p2p_listen_port);
    println!("   P2P max peers: {}", max_peers);
    println!(
        "   Discovery: {}",
        if enable_discovery {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!(
        "   Sync: {}",
        if enable_sync { "enabled" } else { "disabled" }
    );
    println!(
        "   P2P DoS guard: max_per_ip={}, inbound_rate/min={}, gossip_rate/min={}",
        p2p_max_inbound_per_ip, p2p_max_inbound_rate_per_minute, p2p_max_gossip_per_peer_per_minute
    );
    println!("   P2P ban duration: {}s", p2p_ban_duration_secs);
    println!(
        "   Bootnode reconnect interval: {}s",
        p2p_bootnode_retry_interval_secs
    );
    if !bootnodes.is_empty() {
        println!("   Bootnodes: {}", bootnodes.join(", "));
    }
    if let Some(ref banlist_path) = p2p_banlist_path {
        println!("   P2P banlist: {}", banlist_path);
    }
    if let Some(ref cb) = coinbase {
        println!("   Coinbase: {}", cb);
    }

    // Initialize genesis block
    let genesis = create_genesis_block();
    println!(
        "   Genesis block hash: 0x{}",
        hex::encode(genesis.header.hash.as_bytes())
    );
    println!("   Genesis difficulty: {}", genesis.header.difficulty);

    // Start mining task if enabled
    if mine {
        let coinbase_addr =
            coinbase.unwrap_or_else(|| "ZER0x0000000000000000000000000000000000000000".to_string());
        println!("   🎯 Starting mining with coinbase: {}", coinbase_addr);

        // Spawn mining task
        tokio::spawn(async move {
            let mut block_number = 1u64;
            let mut last_hash = genesis.header.hash;

            loop {
                let timestamp = current_timestamp();

                // Simple PoW - find nonce that makes hash have leading zeros
                let mut nonce = 0u64;
                let mut found_hash = Hash::zero();

                // Target: hash must be less than difficulty (simplified)
                let target_leading_zeros = 2; // Number of leading zero bytes required

                loop {
                    // Create block data for hashing
                    let mut data = Vec::new();
                    data.extend_from_slice(last_hash.as_bytes());
                    data.extend_from_slice(&block_number.to_be_bytes());
                    data.extend_from_slice(&timestamp.to_be_bytes());
                    data.extend_from_slice(&nonce.to_be_bytes());

                    found_hash = Hash::from_bytes(zerocore::crypto::keccak256(&data));

                    // Check if hash meets target (simplified: check leading zeros)
                    let leading_zeros = found_hash
                        .as_bytes()
                        .iter()
                        .take_while(|&&b| b == 0)
                        .count();

                    if leading_zeros >= target_leading_zeros {
                        println!(
                            "   ⛏️  Block #{} mined! Hash: 0x{}... Nonce: {}",
                            block_number,
                            &hex::encode(found_hash.as_bytes())[..16],
                            nonce
                        );
                        last_hash = found_hash;
                        block_number += 1;
                        break;
                    }

                    nonce += 1;
                    if nonce.is_multiple_of(50000) {
                        println!(
                            "   Mining block #{}... nonce: {} (leading zeros: {})",
                            block_number, nonce, leading_zeros
                        );
                    }
                }

                // Small delay between blocks for demo
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            }
        });
    }

    println!("✅ Node started successfully!");
    println!("   Press Ctrl+C to stop");

    let network_id = rpc_config
        .as_ref()
        .map(|cfg| cfg.network_id)
        .unwrap_or(10086);
    let _api_service = if let Some(mut cfg) = rpc_config.clone() {
        cfg.port = http_port;
        let mut api_cfg = ApiConfig {
            http_rpc: cfg,
            ..ApiConfig::default()
        };
        api_cfg.ws.port = ws_port;
        api_cfg.rest.port = http_port.saturating_add(10);

        let api = ApiService::try_new(api_cfg)
            .map_err(|e| anyhow::anyhow!("failed to create API service: {e}"))?;
        api.start()
            .await
            .map_err(|e| anyhow::anyhow!("failed to start API service: {e}"))?;
        println!("   HTTP RPC service started on port {}", http_port);
        Some(api)
    } else {
        None
    };

    let network_cfg = NetworkConfig {
        network_id,
        listen_addr: p2p_listen_addr,
        listen_port: p2p_listen_port,
        max_peers,
        min_peers: max_peers.min(25),
        bootnodes,
        enable_discovery,
        enable_sync,
        banlist_path: p2p_banlist_path,
        ban_duration_secs: p2p_ban_duration_secs,
        max_inbound_per_ip: p2p_max_inbound_per_ip,
        max_inbound_rate_per_minute: p2p_max_inbound_rate_per_minute,
        max_gossip_per_peer_per_minute: p2p_max_gossip_per_peer_per_minute,
        bootnode_retry_interval_secs: p2p_bootnode_retry_interval_secs,
        sync_auto_advance: mine,
        sync_auto_advance_interval_secs: 2,
        ..NetworkConfig::default()
    };
    let network_service = NetworkService::new(network_cfg)?;
    network_service.start().await?;
    println!("   P2P service started on port {}", p2p_listen_port);

    // Keep running
    loop {
        println!("   Peers connected: {}", network_service.peer_count());
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    }
}

fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
