//! ZeroChain Storage Layer
//!
//! Provides:
//! - Merkle Patricia Trie (MPT) for state storage
//! - Database abstraction (RocksDB/Redb)
//! - Index services for fast lookups

#![warn(missing_docs)]
#![warn(rustdoc::missing_crate_level_docs)]

pub mod compute;
pub mod db;
pub mod index;
pub mod trie;

pub use compute::ComputeStore;
pub use db::{KeyValueDB, RedbDatabase, RocksDb};
pub use index::{BlockIndex, IndexDB, TxIndex};
pub use trie::{MerklePatriciaTrie, TrieDB, TrieNode, TrieProof};

/// Storage error types
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Database error: {0}")]
    Database(String),
    #[error("Trie error: {0}")]
    Trie(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Key not found: {0}")]
    NotFound(String),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("Crypto error: {0}")]
    Crypto(String),
}

impl From<zerocore::crypto::CryptoError> for StorageError {
    fn from(e: zerocore::crypto::CryptoError) -> Self {
        StorageError::Crypto(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, StorageError>;
