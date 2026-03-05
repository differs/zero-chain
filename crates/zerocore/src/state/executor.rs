//! State Executor - Transaction Execution and State Transitions

use super::StateDb;
use crate::account::{Account, AccountType, I256, U256};
use crate::block::Block;
use crate::crypto::{keccak256, Address, Hash};
use crate::evm::{EvmConfig, EvmEngine, ExecutionContext, ExecutionResult, StateDb as EvmStateDb};
use crate::transaction::{Log, SignedTransaction, TransactionReceipt};
use parking_lot::RwLock;
use std::sync::Arc;
use thiserror::Error;

/// Execution errors
#[derive(Error, Debug, Clone)]
pub enum ExecutionError {
    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),
    #[error("Insufficient balance")]
    InsufficientBalance,
    #[error("Nonce mismatch")]
    NonceMismatch,
    #[error("Gas limit exceeded")]
    GasLimitExceeded,
    #[error("EVM error: {0}")]
    Evm(String),
    #[error("State error: {0}")]
    State(String),
    #[error("Block error: {0}")]
    Block(String),
}

pub type Result<T> = std::result::Result<T, ExecutionError>;

/// State transition
#[derive(Clone, Debug)]
pub struct StateTransition {
    /// From state root
    pub from_root: Hash,
    /// To state root
    pub to_root: Hash,
    /// Transaction receipts
    pub receipts: Vec<TransactionReceipt>,
    /// Total gas used
    pub gas_used: u64,
    /// Logs bloom filter
    pub logs_bloom: [u8; 256],
}

/// State executor
pub struct StateExecutor {
    /// State database
    state_db: Arc<StateDb>,
    /// EVM engine
    evm_engine: RwLock<EvmEngine>,
    /// Chain ID
    chain_id: u64,
}

impl StateExecutor {
    /// Create new state executor
    pub fn new(state_db: Arc<StateDb>, chain_id: u64) -> Self {
        let evm_config = EvmConfig {
            chain_id,
            gas_limit: 30_000_000,
            base_fee: U256::from(1_000_000_000),
        };

        Self {
            state_db,
            evm_engine: RwLock::new(EvmEngine::new(evm_config)),
            chain_id,
        }
    }

    /// Execute a block
    pub fn execute_block(&self, block: &Block, parent_state_root: Hash) -> Result<StateTransition> {
        tracing::info!(
            "Executing block #{} with {} transactions",
            block.header.number.as_u64(),
            block.transactions.len()
        );

        let mut receipts = Vec::with_capacity(block.transactions.len());
        let mut cumulative_gas_used = 0u64;
        let mut logs_bloom = [0u8; 256];

        // Execute each transaction
        for (index, tx) in block.transactions.iter().enumerate() {
            let receipt = self.execute_transaction(tx, block, index as u32, cumulative_gas_used)?;

            // Update cumulative gas
            cumulative_gas_used += receipt.gas_used.as_u64();

            // Update logs bloom
            for (i, &byte) in receipt.logs_bloom.iter().enumerate() {
                logs_bloom[i] |= byte;
            }

            receipts.push(receipt);
        }

        // Calculate new state root
        let to_root = self.state_db.state_root();

        Ok(StateTransition {
            from_root: parent_state_root,
            to_root,
            receipts,
            gas_used: cumulative_gas_used,
            logs_bloom,
        })
    }

    /// Execute a single transaction
    pub fn execute_transaction(
        &self,
        tx: &SignedTransaction,
        block: &Block,
        tx_index: u32,
        cumulative_gas: u64,
    ) -> Result<TransactionReceipt> {
        // Validate transaction
        self.validate_transaction(tx)?;

        // Get sender account
        let sender = tx.sender();
        let mut sender_account = self
            .state_db
            .get_account(&sender)
            .ok_or_else(|| ExecutionError::InvalidTransaction("Sender not found".into()))?;

        // Check nonce
        if tx.nonce() != sender_account.nonce {
            return Err(ExecutionError::NonceMismatch);
        }

        // Check balance
        let max_cost = tx.tx.value + (tx.tx.gas_limit * tx.tx.effective_gas_price(None));
        if sender_account.balance < max_cost {
            return Err(ExecutionError::InsufficientBalance);
        }

        // Deduct gas cost from sender
        let gas_price = tx
            .tx
            .effective_gas_price(Some(block.header.base_fee_per_gas));
        let gas_cost = tx.tx.gas_limit * gas_price;
        sender_account.balance = sender_account.balance - gas_cost;

        // Execute transaction
        let execution_result = self.execute_tx_inner(tx, block)?;

        // Calculate actual gas cost
        let actual_gas_cost = U256::from(execution_result.gas_used) * gas_price;
        let refund = gas_cost - actual_gas_cost;

        // Refund unused gas to sender
        sender_account.balance = sender_account.balance + refund;
        sender_account.nonce += 1;

        // Update sender account
        self.state_db.insert_account(sender, sender_account);

        // Pay gas to miner
        let miner = block.header.coinbase;
        let mut miner_account = self
            .state_db
            .get_account(&miner)
            .unwrap_or_else(|| Account {
                address: miner,
                account_type: AccountType::ExternalOwned {
                    public_key: crate::crypto::PublicKey::from_bytes([0u8; 65]).unwrap(),
                    signature_scheme: Default::default(),
                },
                ..Default::default()
            });

        miner_account.balance = miner_account.balance + actual_gas_cost;
        miner_account.state = crate::account::AccountState::Active;
        self.state_db.insert_account(miner, miner_account);

        // Create receipt
        let receipt = TransactionReceipt {
            transaction_hash: tx.hash,
            transaction_index: tx_index,
            block_hash: block.header.hash,
            block_number: block.header.number.as_u64(),
            from: sender,
            to: tx.to(),
            cumulative_gas_used: U256::from(cumulative_gas + execution_result.gas_used),
            gas_used: U256::from(execution_result.gas_used),
            effective_gas_price: gas_price,
            contract_address: execution_result.created_address,
            logs: execution_result.logs,
            logs_bloom: compute_logs_bloom(&execution_result.logs),
            status: if execution_result.success { 1 } else { 0 },
        };

        Ok(receipt)
    }

    /// Execute transaction inner logic
    fn execute_tx_inner(&self, tx: &SignedTransaction, block: &Block) -> Result<ExecutionResult> {
        // Check if contract creation
        if tx.to().is_none() {
            self.execute_create(tx, block)
        } else {
            self.execute_call(tx, block)
        }
    }

    /// Execute contract creation
    fn execute_create(&self, tx: &SignedTransaction, block: &Block) -> Result<ExecutionResult> {
        // Calculate contract address
        let nonce = self.state_db.get_nonce(&tx.sender());
        let contract_address = self.calculate_contract_address(tx.sender(), nonce);

        // Create contract account
        let contract_account = Account {
            address: contract_address,
            account_type: AccountType::Contract {
                creator: tx.sender(),
                contract_version: 1,
                upgradeable: false,
                admin: None,
            },
            balance: tx.tx.value,
            nonce: 0,
            storage_root: Hash::zero(),
            code_hash: Hash::zero(),
            config: Default::default(),
            state: crate::account::AccountState::Active,
            created_at: block.header.timestamp,
            updated_at: block.header.timestamp,
        };

        self.state_db
            .insert_account(contract_address, contract_account);

        // Transfer value
        self.transfer_value(tx.sender(), contract_address, tx.tx.value)?;

        // Execute init code
        let mut evm = self.evm_engine.write();
        let evm_config = EvmConfig {
            chain_id: self.chain_id,
            gas_limit: tx.tx.gas_limit.as_u64(),
            base_fee: block.header.base_fee_per_gas,
        };

        // Would create proper EVM context and execute
        // Simplified for now

        Ok(ExecutionResult {
            success: true,
            output: tx.tx.input.clone(),
            gas_used: 21000,
            logs: Vec::new(),
            created_address: Some(contract_address),
            error: None,
        })
    }

    /// Execute contract call
    fn execute_call(&self, tx: &SignedTransaction, block: &Block) -> Result<ExecutionResult> {
        let to = tx.to().unwrap();

        // Transfer value
        if !tx.tx.value.is_zero() {
            self.transfer_value(tx.sender(), to, tx.tx.value)?;
        }

        // Get contract code
        let code = self.state_db.get_code(&to).unwrap_or_default();

        if code.is_empty() {
            // Simple transfer, no code
            return Ok(ExecutionResult {
                success: true,
                output: Vec::new(),
                gas_used: 21000,
                logs: Vec::new(),
                created_address: None,
                error: None,
            });
        }

        // Execute EVM
        let mut evm = self.evm_engine.write();

        // Would create proper EVM context and execute
        // Simplified for now

        Ok(ExecutionResult {
            success: true,
            output: Vec::new(),
            gas_used: 50000,
            logs: Vec::new(),
            created_address: None,
            error: None,
        })
    }

    /// Transfer value between accounts
    fn transfer_value(&self, from: Address, to: Address, amount: U256) -> Result<()> {
        if amount.is_zero() {
            return Ok(());
        }

        // Deduct from sender
        let mut from_account = self
            .state_db
            .get_account(&from)
            .ok_or_else(|| ExecutionError::State("Sender not found".into()))?;

        if from_account.balance < amount {
            return Err(ExecutionError::InsufficientBalance);
        }

        from_account.balance = from_account.balance - amount;
        self.state_db.insert_account(from, from_account);

        // Add to recipient
        let mut to_account = self.state_db.get_account(&to).unwrap_or_else(|| Account {
            address: to,
            account_type: AccountType::ExternalOwned {
                public_key: crate::crypto::PublicKey::from_bytes([0u8; 65]).unwrap(),
                signature_scheme: Default::default(),
            },
            ..Default::default()
        });

        to_account.balance = to_account.balance + amount;
        to_account.state = crate::account::AccountState::Active;
        self.state_db.insert_account(to, to_account);

        Ok(())
    }

    /// Validate transaction
    fn validate_transaction(&self, tx: &SignedTransaction) -> Result<()> {
        // Verify signature
        if !tx.verify_signature().unwrap_or(false) {
            return Err(ExecutionError::InvalidTransaction(
                "Invalid signature".into(),
            ));
        }

        // Check chain ID
        if tx.tx.chain_id != self.chain_id {
            return Err(ExecutionError::InvalidTransaction(
                "Invalid chain ID".into(),
            ));
        }

        // Check gas limit
        if tx.tx.gas_limit.as_u64() > 30_000_000 {
            return Err(ExecutionError::GasLimitExceeded);
        }

        Ok(())
    }

    /// Calculate contract address
    fn calculate_contract_address(&self, sender: Address, nonce: u64) -> Address {
        use rlp::RlpStream;

        let mut stream = RlpStream::new_list(2);
        stream.append(&sender.as_bytes());
        stream.append(&nonce);

        let encoded = stream.out();
        let hash = keccak256(&encoded);

        Address::from_slice(&hash[12..]).unwrap()
    }
}

/// Compute logs bloom filter
fn compute_logs_bloom(logs: &[Log]) -> [u8; 256] {
    let mut bloom = [0u8; 256];

    for log in logs {
        // Add address to bloom
        add_to_bloom(&mut bloom, log.address.as_bytes());

        // Add topics to bloom
        for topic in &log.topics {
            add_to_bloom(&mut bloom, topic.as_bytes());
        }
    }

    bloom
}

/// Add data to bloom filter
fn add_to_bloom(bloom: &mut [u8; 256], data: &[u8]) {
    let hash = keccak256(data);

    for i in 0..3 {
        let pos = ((hash[i * 2] as usize & 0x1F) << 3) | (hash[i * 2 + 1] as usize & 0x07);
        bloom[511 - pos] |= 1 << (pos >> 3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateDb;

    #[test]
    fn test_executor_creation() {
        let state_db = Arc::new(StateDb::new(Hash::zero()));
        let executor = StateExecutor::new(state_db, 10086);

        assert_eq!(executor.chain_id, 10086);
    }

    #[test]
    fn test_bloom_filter() {
        let mut bloom = [0u8; 256];
        add_to_bloom(&mut bloom, b"test");

        assert!(bloom.iter().any(|&b| b != 0));
    }
}
