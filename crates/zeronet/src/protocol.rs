//! Protocol message definitions.

use zerocore::{block::Block, crypto::Hash, transaction::SignedTransaction};

/// Minimal sync header payload used by header-first sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncHeader {
    pub number: u64,
    pub hash: Hash,
    pub parent_hash: Hash,
}

/// Minimal block-body metadata used by body sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncBlockBody {
    pub block_hash: Hash,
    pub tx_count: u32,
}

/// Minimal state snapshot metadata used by state sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncStateSnapshot {
    pub block_number: u64,
    pub state_root: Hash,
    pub account_count: u64,
}

/// Protocol message types
#[derive(Clone, Debug)]
pub enum ProtocolMessage {
    /// Disconnect from peer
    Disconnect(String),
    /// New transaction announcement
    NewTransaction(Hash),
    /// New block announcement
    NewBlock(Box<Block>),
    /// New block hash announcement
    NewBlockHash(Hash),
    /// Announce current local head height.
    AnnounceHead(u64),
    /// Request transactions
    GetTransactions(Vec<Hash>),
    /// Request block
    GetBlock(Hash),
    /// Request headers in `[start, start + limit)`.
    SyncGetHeaders { start: u64, limit: u64 },
    /// Header response batch.
    SyncHeaders(Vec<SyncHeader>),
    /// Request a block body by hash.
    SyncGetBlockBody { block_hash: Hash },
    /// Block body response.
    SyncBlockBody(SyncBlockBody),
    /// Request snapshot summary at target block number.
    SyncGetStateSnapshot { block_number: u64 },
    /// Snapshot response.
    SyncStateSnapshot(SyncStateSnapshot),
    /// Transaction response
    Transactions(Vec<SignedTransaction>),
    /// Block response
    Block(Box<Block>),
}

/// Protocol trait
pub trait Protocol: Send + Sync {
    fn handle_message(&self, message: ProtocolMessage) -> Result<(), crate::NetworkError>;
}

/// Block message
#[derive(Clone, Debug)]
pub struct BlockMessage {
    pub block: Block,
}

/// Transaction message
#[derive(Clone, Debug)]
pub struct TxMessage {
    pub transactions: Vec<SignedTransaction>,
}
