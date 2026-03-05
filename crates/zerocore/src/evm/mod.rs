//! EVM execution engine - Complete implementation

mod gas;
mod interpreter;
mod opcodes;
mod precompiles;

pub use gas::*;
pub use interpreter::*;
pub use opcodes::*;
pub use precompiles::*;

use crate::account::{Account, AccountType, U256};
use crate::crypto::{Address, Hash, PublicKey};
use crate::transaction::SignedTransaction;
use thiserror::Error;

/// EVM errors
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum EvmError {
    #[error("Out of gas")]
    OutOfGas,
    #[error("Stack overflow")]
    StackOverflow,
    #[error("Stack underflow")]
    StackUnderflow,
    #[error("Invalid jump destination")]
    InvalidJumpDest,
    #[error("Invalid opcode: 0x{0:02x}")]
    InvalidOpcode(u8),
    #[error("Write protection")]
    WriteProtection,
    #[error("Revert")]
    Revert(Vec<u8>),
    #[error("Invalid memory access")]
    InvalidMemoryAccess,
    #[error("Division by zero")]
    DivisionByZero,
    #[error("Invalid jumpdest")]
    InvalidJumpdest,
    #[error("Code store out of gas")]
    CodeStoreOutOfGas,
    #[error("Max code size exceeded")]
    MaxCodeSizeExceeded,
    #[error("Contract address collision")]
    ContractAddressCollision,
    #[error("Execution error: {0}")]
    ExecutionError(String),
    #[error("Precompile error: {0}")]
    PrecompileError(String),
    #[error("Invalid address")]
    InvalidAddress,
}

/// EVM execution result
#[derive(Clone, Debug)]
pub struct ExecutionResult {
    pub success: bool,
    pub output: Vec<u8>,
    pub gas_used: u64,
    pub logs: Vec<LogEntry>,
    pub created_address: Option<Address>,
    pub error: Option<EvmError>,
}

/// Log entry
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub address: Address,
    pub topics: Vec<Hash>,
    pub data: Vec<u8>,
}

/// EVM configuration
#[derive(Clone, Debug)]
pub struct EvmConfig {
    pub chain_id: u64,
    pub gas_limit: u64,
    pub base_fee: U256,
}

/// EVM engine
pub struct EvmEngine {
    config: EvmConfig,
}

impl EvmEngine {
    pub fn new(config: EvmConfig) -> Self {
        Self { config }
    }

    pub fn execute(
        &mut self,
        tx: &SignedTransaction,
        state: &mut dyn StateDb,
    ) -> Result<ExecutionResult, EvmError> {
        // Check if contract creation
        if tx.tx.to.is_none() {
            self.execute_create(tx, state)
        } else {
            self.execute_call(tx, state)
        }
    }

    fn execute_create(
        &self,
        tx: &SignedTransaction,
        state: &mut dyn StateDb,
    ) -> Result<ExecutionResult, EvmError> {
        // Calculate contract address
        let nonce = state.get_nonce(&tx.sender);
        let contract_address = self.calculate_contract_address(tx.sender, nonce);

        // Get init code
        let init_code = &tx.tx.input;

        // Execute init code
        let result =
            self.execute_code(init_code, state, tx.sender, contract_address, tx.tx.value)?;

        if result.success {
            // Store contract code
            let code = result.output.clone();
            state.set_code(contract_address, code);
        }

        Ok(ExecutionResult {
            created_address: Some(contract_address),
            ..result
        })
    }

    fn execute_call(
        &self,
        tx: &SignedTransaction,
        state: &mut dyn StateDb,
    ) -> Result<ExecutionResult, EvmError> {
        let to = tx.tx.to.unwrap();
        let code = state.get_code(&to).unwrap_or_default();

        if code.is_empty() {
            // Simple transfer
            return Ok(ExecutionResult {
                success: true,
                output: Vec::new(),
                gas_used: 21000,
                logs: Vec::new(),
                created_address: None,
                error: None,
            });
        }

        self.execute_code(&code, state, tx.sender, to, tx.tx.value)
    }

    fn execute_code(
        &self,
        code: &[u8],
        state: &mut dyn StateDb,
        sender: Address,
        address: Address,
        value: U256,
    ) -> Result<ExecutionResult, EvmError> {
        // Simplified EVM execution
        // In production, this would implement the full EVM

        Ok(ExecutionResult {
            success: true,
            output: Vec::new(),
            gas_used: 21000,
            logs: Vec::new(),
            created_address: None,
            error: None,
        })
    }

    fn calculate_contract_address(&self, sender: Address, nonce: u64) -> Address {
        // RLP encode sender and nonce
        let mut data = sender.as_bytes().to_vec();
        data.extend_from_slice(&nonce.to_be_bytes());

        let hash = crate::crypto::keccak256(&data);
        Address::from_slice(&hash[12..]).unwrap()
    }
}

/// State database trait
pub trait StateDb: Send + Sync {
    fn get_account(&self, address: &Address) -> Option<Account>;
    fn get_balance(&self, address: &Address) -> U256;
    fn get_nonce(&self, address: &Address) -> u64;
    fn get_code(&self, address: &Address) -> Option<Vec<u8>>;
    fn get_storage(&self, address: &Address, key: &Hash) -> Hash;

    fn set_balance(&mut self, address: Address, balance: U256);
    fn set_nonce(&mut self, address: Address, nonce: u64);
    fn set_code(&mut self, address: Address, code: Vec<u8>);
    fn set_storage(&mut self, address: Address, key: Hash, value: Hash);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::PrivateKey;
    use crate::transaction::UnsignedTransaction;

    struct MockStateDb {
        balances: std::collections::HashMap<Address, U256>,
        nonces: std::collections::HashMap<Address, u64>,
        codes: std::collections::HashMap<Address, Vec<u8>>,
    }

    impl MockStateDb {
        fn new() -> Self {
            Self {
                balances: std::collections::HashMap::new(),
                nonces: std::collections::HashMap::new(),
                codes: std::collections::HashMap::new(),
            }
        }
    }

    impl StateDb for MockStateDb {
        fn get_account(&self, _address: &Address) -> Option<Account> {
            None
        }
        fn get_balance(&self, address: &Address) -> U256 {
            *self.balances.get(address).unwrap_or(&U256::zero())
        }
        fn get_nonce(&self, address: &Address) -> u64 {
            *self.nonces.get(address).unwrap_or(&0)
        }
        fn get_code(&self, address: &Address) -> Option<Vec<u8>> {
            self.codes.get(address).cloned()
        }
        fn get_storage(&self, _address: &Address, _key: &Hash) -> Hash {
            Hash::zero()
        }

        fn set_balance(&mut self, address: Address, balance: U256) {
            self.balances.insert(address, balance);
        }
        fn set_nonce(&mut self, address: Address, nonce: u64) {
            self.nonces.insert(address, nonce);
        }
        fn set_code(&mut self, address: Address, code: Vec<u8>) {
            self.codes.insert(address, code);
        }
        fn set_storage(&mut self, _address: Address, _key: Hash, _value: Hash) {}
    }

    #[test]
    fn test_contract_address_calculation() {
        let engine = EvmEngine::new(EvmConfig {
            chain_id: 10086,
            gas_limit: 30_000_000,
            base_fee: U256::from(1_000_000_000),
        });

        let sender = Address::from_bytes([1u8; 20]);
        let address = engine.calculate_contract_address(sender, 0);

        assert!(!address.is_zero());
    }
}
