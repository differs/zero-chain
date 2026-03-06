//! Account management module
//!
//! ZeroChain uses a hybrid account model combining:
//! - Balance-based model (Ethereum compatible)
//! - UTXO model (for privacy and parallel execution)
//! - Account abstraction (smart contract wallets)

mod account;
mod manager;
mod utxo;

pub use account::*;
pub use manager::*;
pub use utxo::*;
