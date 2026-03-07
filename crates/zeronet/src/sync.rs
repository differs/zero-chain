//! Sync module - minimal catch-up/recovery manager.

use crate::Result;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio::time::{interval, MissedTickBehavior};

/// Sync state
#[derive(Clone, Debug, PartialEq)]
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

/// Sync manager
pub struct SyncManager {
    state: Arc<RwLock<SyncState>>,
    peer_manager: Arc<crate::PeerManager>,
    running: Arc<AtomicBool>,
    task: RwLock<Option<JoinHandle<()>>>,
}

impl SyncManager {
    pub fn new(peer_manager: Arc<crate::PeerManager>) -> Self {
        Self {
            state: Arc::new(RwLock::new(SyncState::Idle)),
            peer_manager,
            running: Arc::new(AtomicBool::new(false)),
            task: RwLock::new(None),
        }
    }

    pub fn state(&self) -> SyncState {
        self.state.read().clone()
    }

    pub async fn start(&self, target: u64) -> Result<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let state = self.state.clone();
        let peer_manager = self.peer_manager.clone();
        let running = self.running.clone();
        let task = tokio::spawn(async move {
            let mut retries = 0u64;
            let mut local_head = 0u64;
            let mut ticker = interval(std::time::Duration::from_secs(5));
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

                let target_head = peer_manager.highest_peer_height().max(target);
                if local_head >= target_head {
                    *state.write() = SyncState::Complete;
                    continue;
                }

                local_head = (local_head + 64).min(target_head);
                *state.write() = SyncState::Syncing {
                    current: local_head,
                    target: target_head,
                };
            }
        });

        *self.task.write() = Some(task);
        *self.state.write() = SyncState::Syncing { current: 0, target };
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
