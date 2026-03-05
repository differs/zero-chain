//! Merkle Patricia Trie Implementation
//! 
//! ZeroChain uses a modified Merkle Patricia Trie similar to Ethereum's,
//! optimized for our hybrid account model.

mod node;
mod trie;
mod proof;

pub use node::*;
pub use trie::*;
pub use proof::*;
