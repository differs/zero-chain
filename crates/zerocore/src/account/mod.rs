//! Account management module
//!
//! ZeroChain uses a hybrid account model combining:
//! - Balance-based model with native account semantics
//! - UTXO model (for privacy and parallel execution)
//! - Account abstraction (smart contract wallets)

#[allow(clippy::module_inception)]
mod account;
mod manager;
mod utxo;

pub use account::*;
pub use manager::*;
pub use utxo::*;
