//! Protocol module - Placeholder

use zerocore::{block::Block, crypto::Hash, transaction::SignedTransaction};

/// Protocol message types
#[derive(Clone, Debug)]
pub enum ProtocolMessage {
    /// Disconnect from peer
    Disconnect(String),
    /// New transaction announcement
    NewTransaction(Hash),
    /// New block announcement
    NewBlock(Box<Block>),
    /// Request transactions
    GetTransactions(Vec<Hash>),
    /// Request block
    GetBlock(Hash),
    /// Transaction response
    Transactions(Vec<SignedTransaction>),
    /// Block response
    Block(Block),
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
