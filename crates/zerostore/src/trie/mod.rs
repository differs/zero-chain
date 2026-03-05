//! Merkle Patricia Trie Implementation
//!
//! ZeroChain uses a modified Merkle Patricia Trie similar to Ethereum's,
//! optimized for our hybrid account model.

mod node;
mod proof;
mod trie;

pub use node::*;
pub use proof::*;
pub use trie::*;
