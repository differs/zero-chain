//! Transaction module

pub mod pool;

use crate::account::{Account, AccountError, U256};
use crate::crypto::{Address, Hash, PrivateKey, Signature};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Transaction errors
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum TransactionError {
    /// Invalid signature
    #[error("Invalid signature")]
    InvalidSignature,
    /// Invalid nonce
    #[error("Invalid nonce: expected {expected}, got {got}")]
    InvalidNonce { expected: u64, got: u64 },
    /// Insufficient balance
    #[error("Insufficient balance: have {have}, need {need}")]
    InsufficientBalance { have: U256, need: U256 },
    /// Insufficient gas
    #[error("Insufficient gas for transaction")]
    InsufficientGas,
    /// Gas price too low
    #[error("Gas price too low")]
    GasPriceTooLow,
    /// Invalid transaction type
    #[error("Invalid transaction type")]
    InvalidType,
    /// Invalid chain ID
    #[error("Invalid chain ID")]
    InvalidChainId,
    /// Transaction too large
    #[error("Transaction too large: {size} bytes")]
    TooLarge { size: usize },
    /// Replay protection error
    #[error("Replay protection error")]
    ReplayProtection,
}

/// Transaction type
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionType {
    /// Balance transfer transaction
    Transfer,
    /// UTXO transaction
    Utxo,
}

/// Unsigned transaction
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnsignedTransaction {
    /// Transaction type
    pub tx_type: TransactionType,
    /// Nonce
    pub nonce: u64,
    /// Execution fee rate
    pub gas_price: U256,
    /// Gas limit
    pub gas_limit: U256,
    /// Recipient address
    pub to: Option<Address>,
    /// Value
    pub value: U256,
    /// Input data
    pub input: Vec<u8>,
    /// Chain ID
    pub chain_id: u64,
}

impl UnsignedTransaction {
    /// Create a new transfer transaction
    pub fn new_transfer(
        nonce: u64,
        gas_price: U256,
        gas_limit: U256,
        to: Option<Address>,
        value: U256,
        input: Vec<u8>,
        chain_id: u64,
    ) -> Self {
        Self {
            tx_type: TransactionType::Transfer,
            nonce,
            gas_price,
            gas_limit,
            to,
            value,
            input,
            chain_id,
        }
    }

    /// Get effective gas price
    pub fn effective_gas_price(&self, _base_fee: Option<U256>) -> U256 {
        self.gas_price
    }

    /// Calculate transaction hash for signing
    pub fn signing_hash(&self) -> Hash {
        // RLP encode and hash
        let encoded = self.encode_rlp();
        Hash::from_bytes(crate::crypto::keccak256(&encoded))
    }

    /// Sign the transaction
    pub fn sign(self, private_key: &PrivateKey) -> SignedTransaction {
        let signing_hash = self.signing_hash();
        let signature = private_key.sign(signing_hash.as_bytes());

        SignedTransaction {
            tx: self,
            signature,
            sender: Address::from_public_key(&private_key.public_key()),
            hash: Hash::from_bytes(crate::crypto::keccak256(
                &[signing_hash.as_bytes(), &signature.as_bytes()].concat(),
            )),
        }
    }

    /// Encode to RLP (simplified)
    fn encode_rlp(&self) -> Vec<u8> {
        // Simplified RLP encoding for demonstration
        let mut data = Vec::new();
        data.extend_from_slice(&self.nonce.to_be_bytes());
        data.extend_from_slice(&self.gas_price.to_big_endian());
        data.extend_from_slice(&self.gas_limit.to_big_endian());
        if let Some(to) = self.to {
            data.extend_from_slice(to.as_bytes());
        }
        data.extend_from_slice(&self.value.to_big_endian());
        data.extend_from_slice(&self.input);
        data.extend_from_slice(&self.chain_id.to_be_bytes());
        data
    }
}

/// Signed transaction
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedTransaction {
    /// Unsigned transaction
    pub tx: UnsignedTransaction,
    /// Signature
    pub signature: Signature,
    /// Sender address (recovered from signature)
    pub sender: Address,
    /// Transaction hash
    pub hash: Hash,
}

impl SignedTransaction {
    /// Get transaction nonce
    pub fn nonce(&self) -> u64 {
        self.tx.nonce
    }

    /// Get sender address
    pub fn sender(&self) -> Address {
        self.sender
    }

    /// Get recipient address
    pub fn to(&self) -> Option<Address> {
        self.tx.to
    }

    /// Get value
    pub fn value(&self) -> U256 {
        self.tx.value
    }

    /// Get gas limit
    pub fn gas_limit(&self) -> U256 {
        self.tx.gas_limit
    }

    /// Get gas price
    pub fn gas_price(&self) -> U256 {
        self.tx.gas_price
    }

    /// Get input data
    pub fn data(&self) -> &[u8] {
        &self.tx.input
    }

    /// Get signature v
    pub fn v(&self) -> u8 {
        self.signature.v()
    }

    /// Get signature r
    pub fn r(&self) -> &[u8; 32] {
        self.signature.r()
    }

    /// Get signature s
    pub fn s(&self) -> &[u8; 32] {
        self.signature.s()
    }

    /// Get transaction hash
    pub fn hash(&self) -> Hash {
        self.hash
    }

    /// Verify transaction signature
    pub fn verify_signature(&self) -> Result<bool, TransactionError> {
        let signing_hash = self.tx.signing_hash();
        let recovered = self
            .signature
            .recover(signing_hash.as_bytes())
            .map_err(|_| TransactionError::InvalidSignature)?;
        let recovered_addr = Address::from_public_key(&recovered);
        Ok(recovered_addr == self.sender)
    }

    /// Validate transaction
    pub fn validate(&self, account: &Account, base_fee: U256) -> Result<(), TransactionError> {
        // Check nonce
        if self.tx.nonce != account.nonce {
            return Err(TransactionError::InvalidNonce {
                expected: account.nonce,
                got: self.tx.nonce,
            });
        }

        // Check gas price vs base fee
        let effective_price = self.tx.effective_gas_price(Some(base_fee));
        if effective_price < base_fee {
            return Err(TransactionError::GasPriceTooLow);
        }

        // Check sufficient balance for value + gas
        let max_cost = self.tx.value + (self.tx.gas_limit * effective_price);
        if account.balance < max_cost {
            return Err(TransactionError::InsufficientBalance {
                have: account.balance,
                need: max_cost,
            });
        }

        Ok(())
    }

    /// Decode from RLP (simplified)
    pub fn decode_rlp(data: &[u8]) -> Result<Self, TransactionError> {
        // Simplified decoding for demonstration
        Err(TransactionError::InvalidType)
    }

    /// Encode to RLP (simplified)
    pub fn encode_rlp(&self) -> Vec<u8> {
        // Simplified encoding for demonstration
        let mut data = Vec::new();
        data.extend_from_slice(&self.tx.nonce.to_be_bytes());
        data.extend_from_slice(&self.signature.as_bytes());
        data
    }
}

/// Transaction receipt
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionReceipt {
    /// Transaction hash
    pub transaction_hash: Hash,
    /// Transaction index in block
    pub transaction_index: u32,
    /// Block hash
    pub block_hash: Hash,
    /// Block number
    pub block_number: u64,
    /// Sender address
    pub from: Address,
    /// Recipient address
    pub to: Option<Address>,
    /// Cumulative gas used in block
    pub cumulative_gas_used: U256,
    /// Gas used by this transaction
    pub gas_used: U256,
    /// Effective gas price
    pub effective_gas_price: U256,
    /// Contract address (if created)
    pub contract_address: Option<Address>,
    /// Logs
    pub logs: Vec<Log>,
    /// Logs bloom filter
    pub logs_bloom: Vec<u8>,
    /// Status code (1 = success, 0 = failure)
    pub status: u8,
}

/// Log entry
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Log {
    /// Contract address that emitted the log
    pub address: Address,
    /// Log topics
    pub topics: Vec<Hash>,
    /// Log data
    pub data: Vec<u8>,
    /// Block number
    pub block_number: u64,
    /// Transaction hash
    pub transaction_hash: Hash,
    /// Transaction index
    pub transaction_index: u32,
    /// Log index in block
    pub log_index: u32,
    /// Removed (for reorg handling)
    pub removed: bool,
}

pub use pool::TransactionPool;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_creation() {
        let private_key = PrivateKey::random();

        let tx = UnsignedTransaction::new_transfer(
            0,
            U256::from(1_000_000_000),
            U256::from(21000),
            Some(Address::from_bytes([1u8; 20])),
            U256::from(1000),
            vec![],
            10086,
        );

        let signed_tx = tx.sign(&private_key);

        assert_eq!(signed_tx.nonce(), 0);
        assert_eq!(
            signed_tx.sender(),
            Address::from_public_key(&private_key.public_key())
        );
        assert!(!signed_tx.hash.is_zero());
    }
}
