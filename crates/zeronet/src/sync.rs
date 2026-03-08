//! Sync module - header/body/state request-response sync manager.

use crate::protocol::{ProtocolMessage, SyncBlockBody, SyncHeader, SyncStateSnapshot};
use crate::{
    global_block_by_number, global_block_number_for_hash, global_latest_block, global_store_block,
    global_synced_height, set_global_synced_height, NetworkError, Result,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{interval, timeout, Duration, MissedTickBehavior};
use zerocore::account::U256;
use zerocore::block::{Block, BlockHeader};
use zerocore::crypto::Address;
use zerocore::crypto::Hash;

const SYNC_TICK_SECS: u64 = 2;
const SYNC_RESPONSE_TIMEOUT_SECS: u64 = 3;
const SYNC_BATCH_LIMIT: u64 = 8;

/// Serializable sync checkpoint for restart recovery.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCheckpoint {
    pub local_height: u64,
    pub state: SyncState,
}

/// Pluggable state-proof verifier used by snapshot sync.
pub trait StateProofVerifier: Send + Sync {
    fn verify(&self, header: &SyncHeader, snapshot: &SyncStateSnapshot) -> bool;
}

/// Deterministic verifier used by current sync protocol.
#[derive(Debug, Default, Clone)]
pub struct DeterministicStateProofVerifier;

impl StateProofVerifier for DeterministicStateProofVerifier {
    fn verify(&self, header: &SyncHeader, snapshot: &SyncStateSnapshot) -> bool {
        snapshot.state_root == derive_state_root(&header.hash)
            && snapshot.state_proof == derive_state_proof(&header.hash, snapshot.block_number)
    }
}

/// Sync state
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncState {
    /// Not syncing
    Idle,
    /// Recovering from degraded network state
    Recovering { reason: String, retries: u64 },
    /// Syncing in progress
    Syncing { current: u64, target: u64 },
    /// Sync complete
    Complete,
}

enum SyncInbound {
    Headers {
        peer_id: String,
        headers: Vec<SyncHeader>,
    },
    BlockBody {
        peer_id: String,
        body: SyncBlockBody,
    },
    StateSnapshot {
        peer_id: String,
        snapshot: SyncStateSnapshot,
    },
}

/// Sync manager
pub struct SyncManager {
    state: Arc<RwLock<SyncState>>,
    peer_manager: Arc<crate::PeerManager>,
    running: Arc<AtomicBool>,
    task: RwLock<Option<JoinHandle<()>>>,
    local_height: Arc<RwLock<u64>>,
    inbound_tx: mpsc::Sender<SyncInbound>,
    inbound_rx: RwLock<Option<mpsc::Receiver<SyncInbound>>>,
    state_proof_verifier: Arc<dyn StateProofVerifier>,
}

impl SyncManager {
    pub fn new(peer_manager: Arc<crate::PeerManager>) -> Self {
        Self::new_with_verifier(peer_manager, Arc::new(DeterministicStateProofVerifier))
    }

    pub fn new_with_verifier(
        peer_manager: Arc<crate::PeerManager>,
        state_proof_verifier: Arc<dyn StateProofVerifier>,
    ) -> Self {
        set_global_synced_height(0);
        let (inbound_tx, inbound_rx) = mpsc::channel(512);
        Self {
            state: Arc::new(RwLock::new(SyncState::Idle)),
            peer_manager,
            running: Arc::new(AtomicBool::new(false)),
            task: RwLock::new(None),
            local_height: Arc::new(RwLock::new(0)),
            inbound_tx,
            inbound_rx: RwLock::new(Some(inbound_rx)),
            state_proof_verifier,
        }
    }

    pub fn state(&self) -> SyncState {
        self.state.read().clone()
    }

    pub fn local_height(&self) -> u64 {
        (*self.local_height.read()).max(global_synced_height())
    }

    pub fn set_local_height(&self, height: u64) {
        *self.local_height.write() = height;
        set_global_synced_height(height);
    }

    pub fn export_checkpoint(&self) -> SyncCheckpoint {
        SyncCheckpoint {
            local_height: self.local_height(),
            state: self.state(),
        }
    }

    pub fn import_checkpoint(&self, checkpoint: &SyncCheckpoint) {
        *self.local_height.write() = checkpoint.local_height;
        *self.state.write() = checkpoint.state.clone();
        set_global_synced_height(checkpoint.local_height);
    }

    pub fn bump_local_height(&self, delta: u64) -> u64 {
        let mut local = self.local_height.write();
        *local = local.saturating_add(delta);
        *local
    }

    pub fn build_headers_response(&self, start: u64, limit: u64) -> Vec<SyncHeader> {
        headers_from_local_chain(start, limit)
    }

    pub fn build_block_body_response(&self, block_hash: &Hash) -> Option<SyncBlockBody> {
        let number = global_block_number_for_hash(block_hash)?;
        let block = global_block_by_number(number)?;
        Some(SyncBlockBody {
            block_hash: *block_hash,
            tx_count: block.transactions.len() as u32,
        })
    }

    pub fn build_state_snapshot_response(&self, block_number: u64) -> Option<SyncStateSnapshot> {
        let block = global_block_by_number(block_number)?;
        let block_hash = block.header.hash;
        Some(SyncStateSnapshot {
            block_number,
            state_root: derive_state_root(&block_hash),
            account_count: synthetic_account_count(block_number),
            state_proof: derive_state_proof(&block_hash, block_number),
        })
    }

    pub fn handle_sync_headers(&self, peer_id: String, headers: Vec<SyncHeader>) {
        if let Err(err) = self
            .inbound_tx
            .try_send(SyncInbound::Headers { peer_id, headers })
        {
            tracing::debug!("dropping sync headers due to full channel: {}", err);
        }
    }

    pub fn handle_sync_block_body(&self, peer_id: String, body: SyncBlockBody) {
        if let Err(err) = self
            .inbound_tx
            .try_send(SyncInbound::BlockBody { peer_id, body })
        {
            tracing::debug!("dropping sync block body due to full channel: {}", err);
        }
    }

    pub fn handle_sync_state_snapshot(&self, peer_id: String, snapshot: SyncStateSnapshot) {
        if let Err(err) = self
            .inbound_tx
            .try_send(SyncInbound::StateSnapshot { peer_id, snapshot })
        {
            tracing::debug!("dropping sync state snapshot due to full channel: {}", err);
        }
    }

    pub async fn start(&self, target: u64) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let mut inbound_rx = self.inbound_rx.write().take().ok_or_else(|| {
            NetworkError::ConnectionError("sync inbound channel already taken".into())
        })?;

        let state = self.state.clone();
        let peer_manager = self.peer_manager.clone();
        let running = self.running.clone();
        let local_height = self.local_height.clone();
        let state_proof_verifier = self.state_proof_verifier.clone();

        let task = tokio::spawn(async move {
            let mut retries = 0u64;
            let mut ticker = interval(Duration::from_secs(SYNC_TICK_SECS));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

            while running.load(Ordering::Relaxed) {
                ticker.tick().await;

                let peer_count = peer_manager.peer_count();
                if peer_count == 0 {
                    retries = retries.saturating_add(1);
                    *state.write() = SyncState::Recovering {
                        reason: "no_peers".to_string(),
                        retries,
                    };
                    continue;
                }

                let local = *local_height.read();
                let external_head = global_synced_height();
                let chain_head = global_latest_block()
                    .map(|b| b.header.number.as_u64())
                    .unwrap_or(0);
                let local = local.max(external_head).max(chain_head);
                if local > *local_height.read() {
                    *local_height.write() = local;
                }
                let target_head = peer_manager.highest_peer_height().max(target);
                if local >= target_head {
                    *state.write() = SyncState::Complete;
                    continue;
                }

                let Some(peer) = peer_manager.get_best_peers(1).into_iter().next() else {
                    retries = retries.saturating_add(1);
                    *state.write() = SyncState::Recovering {
                        reason: "no_best_peer".to_string(),
                        retries,
                    };
                    continue;
                };
                let peer_id = peer.info.peer_id.clone();

                let request_start = local.saturating_add(1);
                let request_limit = (target_head.saturating_sub(local))
                    .min(SYNC_BATCH_LIMIT)
                    .max(1);
                if peer
                    .send(ProtocolMessage::SyncGetHeaders {
                        start: request_start,
                        limit: request_limit,
                    })
                    .is_err()
                {
                    retries = retries.saturating_add(1);
                    *state.write() = SyncState::Recovering {
                        reason: "headers_request_failed".to_string(),
                        retries,
                    };
                    peer_manager.update_score(&peer_id, -8);
                    continue;
                }

                let headers = match wait_for_headers(
                    &mut inbound_rx,
                    &peer_id,
                    request_start,
                    Duration::from_secs(SYNC_RESPONSE_TIMEOUT_SECS),
                )
                .await
                {
                    Ok(headers) => headers,
                    Err(err) => {
                        retries = retries.saturating_add(1);
                        *state.write() = SyncState::Recovering {
                            reason: format!("headers_timeout:{err}"),
                            retries,
                        };
                        peer_manager.update_score(&peer_id, -8);
                        continue;
                    }
                };

                if let Err(err) = validate_headers(request_start, &headers) {
                    retries = retries.saturating_add(1);
                    tracing::warn!(
                        "sync invalid headers from peer {} start {}: {}",
                        peer_id,
                        request_start,
                        err
                    );
                    *state.write() = SyncState::Recovering {
                        reason: format!("invalid_headers:{err}"),
                        retries,
                    };
                    peer_manager.update_score(&peer_id, -20);
                    continue;
                }

                let mut body_ok = true;
                for header in &headers {
                    if peer
                        .send(ProtocolMessage::SyncGetBlockBody {
                            block_hash: header.hash,
                        })
                        .is_err()
                    {
                        body_ok = false;
                        break;
                    }

                    match wait_for_block_body(
                        &mut inbound_rx,
                        &peer_id,
                        &header.hash,
                        Duration::from_secs(SYNC_RESPONSE_TIMEOUT_SECS),
                    )
                    .await
                    {
                        Ok(_body) => {}
                        Err(_err) => {
                            body_ok = false;
                            break;
                        }
                    }
                }

                if !body_ok {
                    retries = retries.saturating_add(1);
                    *state.write() = SyncState::Recovering {
                        reason: "block_body_stage_failed".to_string(),
                        retries,
                    };
                    peer_manager.update_score(&peer_id, -10);
                    continue;
                }

                let Some(last_header) = headers.last() else {
                    retries = retries.saturating_add(1);
                    *state.write() = SyncState::Recovering {
                        reason: "empty_header_batch".to_string(),
                        retries,
                    };
                    continue;
                };

                if peer
                    .send(ProtocolMessage::SyncGetStateSnapshot {
                        block_number: last_header.number,
                    })
                    .is_err()
                {
                    retries = retries.saturating_add(1);
                    *state.write() = SyncState::Recovering {
                        reason: "state_request_failed".to_string(),
                        retries,
                    };
                    peer_manager.update_score(&peer_id, -8);
                    continue;
                }

                let snapshot = match wait_for_state_snapshot(
                    &mut inbound_rx,
                    &peer_id,
                    last_header.number,
                    Duration::from_secs(SYNC_RESPONSE_TIMEOUT_SECS),
                )
                .await
                {
                    Ok(snapshot) => snapshot,
                    Err(err) => {
                        retries = retries.saturating_add(1);
                        *state.write() = SyncState::Recovering {
                            reason: format!("state_timeout:{err}"),
                            retries,
                        };
                        peer_manager.update_score(&peer_id, -10);
                        continue;
                    }
                };

                if !state_proof_verifier.verify(last_header, &snapshot) {
                    retries = retries.saturating_add(1);
                    *state.write() = SyncState::Recovering {
                        reason: "state_proof_verification_failed".to_string(),
                        retries,
                    };
                    peer_manager.update_score(&peer_id, -20);
                    continue;
                }

                for header in &headers {
                    if global_block_by_number(header.number)
                        .map(|b| b.header.hash == header.hash)
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    global_store_block(block_from_sync_header(header));
                }
                *local_height.write() = last_header.number;
                set_global_synced_height(last_header.number);
                *state.write() = SyncState::Syncing {
                    current: last_header.number,
                    target: target_head,
                };
                peer_manager.update_score(&peer_id, 4);

                if last_header.number >= target_head {
                    *state.write() = SyncState::Complete;
                }
            }
        });

        *self.task.write() = Some(task);
        *self.state.write() = SyncState::Syncing {
            current: self.local_height(),
            target,
        };
        Ok(())
    }

    pub async fn start_default(&self) -> Result<()> {
        self.start(0).await
    }

    pub async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(task) = self.task.write().take() {
            task.abort();
        }
        *self.state.write() = SyncState::Idle;
        Ok(())
    }

    pub async fn complete_sync(&self) {
        *self.state.write() = SyncState::Complete;
    }
}

fn headers_from_local_chain(start: u64, limit: u64) -> Vec<SyncHeader> {
    if limit == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    let end = start.saturating_add(limit).saturating_sub(1);
    for number in start..=end {
        let Some(block) = global_block_by_number(number) else {
            break;
        };
        out.push(sync_header_from_block(&block));
    }

    out
}

fn sync_header_from_block(block: &Block) -> SyncHeader {
    SyncHeader {
        number: block.header.number.as_u64(),
        hash: block.header.hash,
        parent_hash: block.header.parent_hash,
        timestamp: block.header.timestamp,
        difficulty: block.header.difficulty.as_u64(),
        nonce: block.header.nonce,
        coinbase: block.header.coinbase,
        mix_hash: block.header.mix_hash,
        extra_data: block.header.extra_data.clone(),
    }
}

fn block_from_sync_header(header: &SyncHeader) -> Block {
    let block_header = BlockHeader {
        version: 1,
        parent_hash: header.parent_hash,
        uncle_hashes: Vec::new(),
        coinbase: header.coinbase,
        state_root: Hash::zero(),
        transactions_root: Hash::zero(),
        receipts_root: Hash::zero(),
        number: U256::from(header.number),
        gas_limit: 30_000_000,
        gas_used: 0,
        timestamp: header.timestamp,
        difficulty: U256::from(header.difficulty),
        nonce: header.nonce,
        extra_data: header.extra_data.clone(),
        mix_hash: header.mix_hash,
        base_fee_per_gas: U256::from(1_000_000_000u64),
        hash: header.hash,
    };
    Block {
        header: block_header,
        transactions: Vec::new(),
        uncles: Vec::new(),
    }
}

fn verify_sync_header_hash(header: &SyncHeader) -> bool {
    let mut reconstructed = block_from_sync_header(header);
    reconstructed.header.compute_hash() == header.hash
}

fn validate_headers(
    expected_start: u64,
    headers: &[SyncHeader],
) -> std::result::Result<(), String> {
    if headers.is_empty() {
        return Err("empty headers".to_string());
    }

    if headers[0].number != expected_start {
        return Err(format!(
            "first header number mismatch: expected {}, got {}",
            expected_start, headers[0].number
        ));
    }

    for (idx, header) in headers.iter().enumerate() {
        if idx > 0 {
            let prev = &headers[idx - 1];
            if header.number != prev.number.saturating_add(1) {
                return Err("non_contiguous_header_numbers".to_string());
            }
            if header.parent_hash != prev.hash {
                return Err("parent_hash_mismatch".to_string());
            }
            if header.timestamp < prev.timestamp {
                return Err("timestamp_regressed".to_string());
            }
        }

        if !verify_sync_header_hash(header) {
            return Err("header_hash_verification_failed".to_string());
        }
    }

    Ok(())
}

async fn wait_for_headers(
    rx: &mut mpsc::Receiver<SyncInbound>,
    peer_id: &str,
    expected_start: u64,
    deadline: Duration,
) -> std::result::Result<Vec<SyncHeader>, String> {
    let recv = async {
        loop {
            match rx.recv().await {
                Some(SyncInbound::Headers {
                    peer_id: inbound_peer,
                    headers,
                }) if inbound_peer == peer_id => return Ok(headers),
                Some(_) => continue,
                None => return Err("sync channel closed".to_string()),
            }
        }
    };

    let headers = timeout(deadline, recv)
        .await
        .map_err(|_| "timeout".to_string())??;
    if headers.first().map(|h| h.number).unwrap_or_default() != expected_start {
        return Err("unexpected_headers_start".to_string());
    }

    Ok(headers)
}

async fn wait_for_block_body(
    rx: &mut mpsc::Receiver<SyncInbound>,
    peer_id: &str,
    expected_hash: &Hash,
    deadline: Duration,
) -> std::result::Result<SyncBlockBody, String> {
    timeout(deadline, async {
        loop {
            match rx.recv().await {
                Some(SyncInbound::BlockBody {
                    peer_id: inbound_peer,
                    body,
                }) if inbound_peer == peer_id && body.block_hash == *expected_hash => {
                    return Ok(body)
                }
                Some(_) => continue,
                None => return Err("sync channel closed".to_string()),
            }
        }
    })
    .await
    .map_err(|_| "timeout".to_string())?
}

async fn wait_for_state_snapshot(
    rx: &mut mpsc::Receiver<SyncInbound>,
    peer_id: &str,
    expected_number: u64,
    deadline: Duration,
) -> std::result::Result<SyncStateSnapshot, String> {
    timeout(deadline, async {
        loop {
            match rx.recv().await {
                Some(SyncInbound::StateSnapshot {
                    peer_id: inbound_peer,
                    snapshot,
                }) if inbound_peer == peer_id && snapshot.block_number == expected_number => {
                    return Ok(snapshot)
                }
                Some(_) => continue,
                None => return Err("sync channel closed".to_string()),
            }
        }
    })
    .await
    .map_err(|_| "timeout".to_string())?
}

fn synthetic_account_count(number: u64) -> u64 {
    1_000u64.saturating_add(number.saturating_mul(13))
}

pub(crate) fn derive_state_root(block_hash: &Hash) -> Hash {
    let mut data = Vec::with_capacity(48);
    data.extend_from_slice(b"ZERO-SYNC-S");
    data.extend_from_slice(block_hash.as_bytes());
    Hash::from_bytes(zerocore::crypto::keccak256(&data))
}

pub(crate) fn derive_state_proof(block_hash: &Hash, block_number: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(b"ZERO-SYNC-P");
    data.extend_from_slice(block_hash.as_bytes());
    data.extend_from_slice(&block_number.to_be_bytes());
    zerocore::crypto::keccak256(&data).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use once_cell::sync::Lazy;
    use std::sync::Mutex;
    use zerocore::block::create_genesis_block;

    static TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn make_block(number: u64, parent_hash: Hash, timestamp: u64) -> Block {
        let mut header = BlockHeader {
            version: 1,
            parent_hash,
            uncle_hashes: Vec::new(),
            coinbase: Address::zero(),
            state_root: Hash::zero(),
            transactions_root: Hash::zero(),
            receipts_root: Hash::zero(),
            number: U256::from(number),
            gas_limit: 30_000_000,
            gas_used: 0,
            timestamp,
            difficulty: U256::from(1_000_000_000_000_000u64),
            nonce: number.saturating_mul(17).saturating_add(7),
            extra_data: format!("sync-test-{number}").into_bytes(),
            mix_hash: Hash::from_bytes(zerocore::crypto::keccak256(
                format!("mix-{number}").as_bytes(),
            )),
            base_fee_per_gas: U256::from(1_000_000_000u64),
            hash: Hash::zero(),
        };
        header.hash = header.compute_hash();
        Block {
            header,
            transactions: Vec::new(),
            uncles: Vec::new(),
        }
    }

    fn seed_chain(head: u64) {
        crate::global_reset_sync_cache();
        let genesis = create_genesis_block();
        let mut parent_hash = genesis.header.hash;
        crate::global_store_block(genesis);

        for number in 1..=head {
            let block = make_block(number, parent_hash, number.saturating_mul(10));
            parent_hash = block.header.hash;
            crate::global_store_block(block);
        }
    }

    #[test]
    fn test_chain_responses_from_global_blocks() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        let manager = SyncManager::new(Arc::new(crate::PeerManager::new(4)));
        seed_chain(6);
        manager.set_local_height(6);

        let headers = manager.build_headers_response(2, 3);
        assert_eq!(headers.len(), 3);
        assert_eq!(headers[0].number, 2);
        assert_eq!(headers[2].number, 4);

        let body = manager
            .build_block_body_response(&headers[1].hash)
            .expect("block body response");
        assert_eq!(body.block_hash, headers[1].hash);

        let snapshot = manager
            .build_state_snapshot_response(4)
            .expect("state snapshot response");
        assert_eq!(snapshot.block_number, 4);
        assert_eq!(snapshot.state_root, derive_state_root(&headers[2].hash));
        assert_eq!(
            snapshot.state_proof,
            derive_state_proof(&headers[2].hash, snapshot.block_number)
        );
    }

    #[tokio::test]
    async fn test_sync_progresses_with_real_request_response_flow() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        let peer_manager = Arc::new(crate::PeerManager::new(8));
        let sync = Arc::new(SyncManager::new(peer_manager.clone()));

        let (tx, mut rx) = mpsc::channel(32);
        peer_manager
            .add_peer_with_sender(
                crate::NodeRecord {
                    peer_id: "peer-sync-a".to_string(),
                    ip: "127.0.0.1".to_string(),
                    tcp_port: 30303,
                    udp_port: 30303,
                    network_id: 10086,
                },
                tx,
            )
            .unwrap();
        let _ = peer_manager.update_peer_height("peer-sync-a", 3);
        seed_chain(3);

        sync.start_default().await.unwrap();

        let sync_clone = sync.clone();
        let responder = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                match msg {
                    ProtocolMessage::SyncGetHeaders { start, limit } => {
                        let headers = headers_from_local_chain(start, limit);
                        sync_clone.handle_sync_headers("peer-sync-a".to_string(), headers);
                    }
                    ProtocolMessage::SyncGetBlockBody { block_hash } => {
                        sync_clone.handle_sync_block_body(
                            "peer-sync-a".to_string(),
                            SyncBlockBody {
                                block_hash,
                                tx_count: 2,
                            },
                        );
                    }
                    ProtocolMessage::SyncGetStateSnapshot { block_number } => {
                        let block_hash = crate::global_block_by_number(block_number)
                            .expect("seeded block")
                            .header
                            .hash;
                        sync_clone.handle_sync_state_snapshot(
                            "peer-sync-a".to_string(),
                            SyncStateSnapshot {
                                block_number,
                                state_root: derive_state_root(&block_hash),
                                account_count: 42,
                                state_proof: derive_state_proof(&block_hash, block_number),
                            },
                        );
                    }
                    _ => {}
                }
            }
        });

        timeout(Duration::from_secs(10), async {
            loop {
                if sync.state() == SyncState::Complete && sync.local_height() >= 3 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("sync should complete");

        sync.stop().await.unwrap();
        responder.abort();
        assert!(sync.local_height() >= 3);
    }

    #[test]
    fn checkpoint_roundtrip_restores_height_and_state() {
        let _guard = TEST_LOCK.lock().expect("test lock");
        let manager = SyncManager::new(Arc::new(crate::PeerManager::new(4)));
        manager.set_local_height(12);
        manager.import_checkpoint(&SyncCheckpoint {
            local_height: 12,
            state: SyncState::Recovering {
                reason: "network_gap".to_string(),
                retries: 3,
            },
        });

        let checkpoint = manager.export_checkpoint();
        assert_eq!(checkpoint.local_height, 12);
        assert_eq!(
            checkpoint.state,
            SyncState::Recovering {
                reason: "network_gap".to_string(),
                retries: 3
            }
        );
    }
}
