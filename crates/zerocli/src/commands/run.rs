//! Run command - Full node implementation

use crate::Result;
use anyhow::{anyhow, bail};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use zeroapi::rpc::RpcConfig;
use zeroapi::{ApiConfig, ApiService};
use zerocore::block::create_genesis_block;
use zerocore::crypto::keccak256;
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
    sync_source_rpcs: Vec<String>,
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
    if !sync_source_rpcs.is_empty() {
        println!("   Sync source RPCs: {}", sync_source_rpcs.join(", "));
    }
    if mine {
        let display_coinbase = coinbase
            .clone()
            .unwrap_or_else(|| "ZER0x0000000000000000000000000000000000000000".to_string());
        println!("   🎯 Mining worker coinbase: {}", display_coinbase);
    } else if let Some(ref cb) = coinbase {
        println!("   Coinbase: {}", cb);
    }

    // Initialize genesis block
    let genesis = create_genesis_block();
    println!(
        "   Genesis block hash: 0x{}",
        hex::encode(genesis.header.hash.as_bytes())
    );
    println!("   Genesis difficulty: {}", genesis.header.difficulty);
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

        if mine {
            let rpc_url = format!("http://127.0.0.1:{http_port}");
            println!("   🎯 Starting RPC-backed mining worker at {}", rpc_url);
            tokio::spawn(async move {
                run_rpc_backed_miner(rpc_url, "zerochain-local".to_string()).await;
            });
        }

        if !mine && !sync_source_rpcs.is_empty() {
            let local_rpc_url = format!("http://127.0.0.1:{http_port}");
            let sources = sync_source_rpcs.clone();
            println!(
                "   🔄 Starting RPC pull sync worker from {} source(s)",
                sources.len()
            );
            tokio::spawn(async move {
                run_rpc_sync_puller(local_rpc_url, sources).await;
            });
        }
        Some(api)
    } else {
        if mine {
            println!("   ⚠️ Mining requested but HTTP RPC is disabled; mining worker not started");
        }
        if !sync_source_rpcs.is_empty() {
            println!("   ⚠️ Sync source RPC configured but HTTP RPC is disabled; sync worker not started");
        }
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
        sync_auto_advance: false,
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

#[derive(Debug, Deserialize)]
struct RpcResponseEnvelope<T> {
    result: Option<T>,
    error: Option<RpcErrorEnvelope>,
}

#[derive(Debug, Deserialize)]
struct RpcErrorEnvelope {
    message: String,
}

#[derive(Debug, Deserialize)]
struct WorkPayload {
    work_id: String,
    prev_hash: String,
    height: u64,
    target_leading_zero_bytes: usize,
}

#[derive(Debug, Deserialize)]
struct SubmitWorkPayload {
    accepted: bool,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    block_hash: Option<String>,
    #[serde(default)]
    height: Option<u64>,
}

async fn rpc_call<T: DeserializeOwned>(
    client: &reqwest::Client,
    rpc_url: &str,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<T> {
    let response = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await?;

    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        bail!("rpc {} failed with status {}: {}", method, status, body);
    }

    let envelope: RpcResponseEnvelope<T> = serde_json::from_str(&body).map_err(|e| {
        anyhow!(
            "decode rpc {} response failed: {} (body={})",
            method,
            e,
            body
        )
    })?;

    if let Some(err) = envelope.error {
        bail!("rpc {} returned error: {}", method, err.message);
    }

    envelope
        .result
        .ok_or_else(|| anyhow!("rpc {} returned empty result", method))
}

fn compute_pow_hash(prev_hash: &[u8; 32], height: u64, nonce: u64) -> [u8; 32] {
    let mut data = Vec::with_capacity(32 + 8 + 8);
    data.extend_from_slice(prev_hash);
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&nonce.to_be_bytes());
    keccak256(&data)
}

async fn run_rpc_backed_miner(rpc_url: String, miner_label: String) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            println!("   ⚠️ Failed to build mining RPC client: {}", err);
            return;
        }
    };

    loop {
        let work =
            match rpc_call::<WorkPayload>(&client, &rpc_url, "zero_getWork", serde_json::json!([]))
                .await
            {
                Ok(work) => work,
                Err(err) => {
                    println!("   ⚠️ Failed to fetch mining work: {}", err);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };

        let prev_hash_hex = work.prev_hash.trim_start_matches("0x");
        let prev_hash_bytes = match hex::decode(prev_hash_hex) {
            Ok(bytes) if bytes.len() == 32 => bytes,
            Ok(_) => {
                println!("   ⚠️ Invalid prev_hash length in work {}", work.work_id);
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
            Err(err) => {
                println!(
                    "   ⚠️ Invalid prev_hash hex in work {}: {}",
                    work.work_id, err
                );
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
        };

        let mut prev_hash = [0u8; 32];
        prev_hash.copy_from_slice(&prev_hash_bytes);

        let mut nonce = 0u64;
        loop {
            let hash = compute_pow_hash(&prev_hash, work.height, nonce);
            let leading_zeros = hash.iter().take_while(|&&b| b == 0).count();

            if leading_zeros >= work.target_leading_zero_bytes {
                let hash_hex = format!("0x{}", hex::encode(hash));
                let submit = rpc_call::<SubmitWorkPayload>(
                    &client,
                    &rpc_url,
                    "zero_submitWork",
                    serde_json::json!([{
                        "work_id": work.work_id,
                        "nonce": nonce,
                        "hash_hex": hash_hex,
                        "miner": miner_label.clone(),
                    }]),
                )
                .await;

                match submit {
                    Ok(result) if result.accepted => {
                        let block_hash = result.block_hash.unwrap_or_else(|| "0x".to_string());
                        let hash_body = block_hash.trim_start_matches("0x");
                        let short_hash = if hash_body.is_empty() {
                            "unknown".to_string()
                        } else {
                            hash_body.chars().take(16).collect::<String>()
                        };
                        let display_height = result.height.unwrap_or(work.height);
                        println!(
                            "   ⛏️  Block #{} mined! Hash: 0x{}... Nonce: {}",
                            display_height, short_hash, nonce
                        );
                    }
                    Ok(result) => {
                        println!(
                            "   ⚠️ Share rejected at height {}: {}",
                            work.height,
                            result
                                .reason
                                .unwrap_or_else(|| "unknown_reason".to_string())
                        );
                    }
                    Err(err) => {
                        println!(
                            "   ⚠️ Failed to submit share at height {}: {}",
                            work.height, err
                        );
                    }
                }

                break;
            }

            nonce = nonce.saturating_add(1);
            if nonce.is_multiple_of(50_000) {
                println!(
                    "   Mining block #{}... nonce: {} (leading zeros: {})",
                    work.height, nonce, leading_zeros
                );
            }
            if nonce.is_multiple_of(10_000) {
                // Keep RPC/network tasks responsive while this CPU-heavy loop runs.
                tokio::task::yield_now().await;
            }
        }
    }
}

fn parse_block_number_hex(block: &serde_json::Value) -> Option<u64> {
    let raw = block.get("number")?.as_str()?;
    let trimmed = raw.strip_prefix("0x").unwrap_or(raw);
    if trimmed.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(trimmed, 16).ok()
}

async fn run_rpc_sync_puller(local_rpc_url: String, sync_sources: Vec<String>) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            println!("   ⚠️ Failed to build sync RPC client: {}", err);
            return;
        }
    };

    if sync_sources.is_empty() {
        return;
    }

    const BATCH_LIMIT: u64 = 16;
    const BASE_SLEEP: std::time::Duration = std::time::Duration::from_secs(3);
    const RATE_LIMIT_BACKOFF: std::time::Duration = std::time::Duration::from_secs(8);

    loop {
        let mut sleep_for = BASE_SLEEP;
        let local_latest = match rpc_call::<serde_json::Value>(
            &client,
            &local_rpc_url,
            "zero_getLatestBlock",
            serde_json::json!([]),
        )
        .await
        {
            Ok(v) => v,
            Err(err) => {
                println!("   ⚠️ Sync puller failed to query local head: {}", err);
                if err.to_string().contains("Rate limit exceeded") {
                    sleep_for = RATE_LIMIT_BACKOFF;
                }
                tokio::time::sleep(sleep_for).await;
                continue;
            }
        };

        let local_head = parse_block_number_hex(&local_latest).unwrap_or(0);
        let mut best_source: Option<(String, u64)> = None;

        for source in &sync_sources {
            match rpc_call::<serde_json::Value>(
                &client,
                source,
                "zero_getLatestBlock",
                serde_json::json!([]),
            )
            .await
            {
                Ok(v) => {
                    if let Some(head) = parse_block_number_hex(&v) {
                        if best_source
                            .as_ref()
                            .map(|(_, current)| head > *current)
                            .unwrap_or(true)
                        {
                            best_source = Some((source.clone(), head));
                        }
                    }
                }
                Err(err) => {
                    println!("   ⚠️ Sync puller failed source {}: {}", source, err);
                    if err.to_string().contains("Rate limit exceeded") {
                        sleep_for = RATE_LIMIT_BACKOFF;
                    }
                }
            }
        }

        let Some((best_source_url, best_head)) = best_source else {
            tokio::time::sleep(sleep_for).await;
            continue;
        };

        if best_head <= local_head {
            tokio::time::sleep(sleep_for).await;
            continue;
        }

        let from = local_head.saturating_add(1);
        let to = std::cmp::min(best_head, local_head.saturating_add(BATCH_LIMIT));
        let range = match rpc_call::<serde_json::Value>(
            &client,
            &best_source_url,
            "zero_getBlocksRange",
            serde_json::json!([{
                "from": from,
                "to": to,
                "limit": BATCH_LIMIT,
            }]),
        )
        .await
        {
            Ok(v) => v,
            Err(err) => {
                println!(
                    "   ⚠️ Sync puller failed to fetch range {}-{} from {}: {}",
                    from, to, best_source_url, err
                );
                if err.to_string().contains("Rate limit exceeded") {
                    sleep_for = RATE_LIMIT_BACKOFF;
                }
                tokio::time::sleep(sleep_for).await;
                continue;
            }
        };

        let mut items = range
            .get("items")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        if items.is_empty() {
            tokio::time::sleep(sleep_for).await;
            continue;
        }

        items.sort_by_key(|block| parse_block_number_hex(block).unwrap_or(u64::MAX));
        let mut imported = 0u64;
        let mut import_failed = false;

        for block in items {
            match rpc_call::<serde_json::Value>(
                &client,
                &local_rpc_url,
                "zero_importBlock",
                serde_json::json!([block]),
            )
            .await
            {
                Ok(_) => imported = imported.saturating_add(1),
                Err(err) => {
                    println!("   ⚠️ Sync puller failed zero_importBlock: {}", err);
                    if err.to_string().contains("Rate limit exceeded") {
                        sleep_for = RATE_LIMIT_BACKOFF;
                    }
                    import_failed = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        if imported > 0 {
            println!(
                "   🔄 Sync puller imported {} block(s), local_head {} -> <= {}",
                imported, local_head, to
            );
        }

        if import_failed {
            tokio::time::sleep(sleep_for).await;
        } else if imported > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        } else {
            tokio::time::sleep(sleep_for).await;
        }
    }
}
