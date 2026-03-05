//! ZeroChain Storage Layer
//! 
//! Provides:
//! - Merkle Patricia Trie (MPT) for state storage
//! - Database abstraction (RocksDB/Redb)
//! - Index services for fast lookups

#![warn(missing_docs)]
#![warn(rustdoc::missing_crate_level_docs)]

pub mod db;
pub mod index;
pub mod trie;

pub use db::{Database, RocksDb, RedbDatabase, KeyValueDB};
pub use index::{TxIndex, BlockIndex, IndexDB};
pub use trie::{MerklePatriciaTrie, TrieDB, TrieNode, TrieError, TrieProof};

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
}

pub type Result<T> = std::result::Result<T, StorageError>;
