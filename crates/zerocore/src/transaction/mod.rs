//! Transaction module

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
    /// Legacy transaction
    Legacy,
    /// EIP-2930 (with access list)
    AccessList,
    /// EIP-1559 (with priority fee)
    Eip1559,
    /// UTXO transaction
    Utxo,
    /// Contract deployment
    ContractCreation,
}

/// Access list entry
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccessListItem {
    /// Address
    pub address: Address,
    /// Storage keys
    pub storage_keys: Vec<Hash>,
}

/// Transaction access list
pub type AccessList = Vec<AccessListItem>;

/// Unsigned transaction
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnsignedTransaction {
    /// Transaction type
    pub tx_type: TransactionType,
    /// Nonce
    pub nonce: u64,
    /// Gas price (legacy and EIP-2930)
    pub gas_price: Option<U256>,
    /// Max priority fee per gas (EIP-1559)
    pub max_priority_fee_per_gas: Option<U256>,
    /// Max fee per gas (EIP-1559)
    pub max_fee_per_gas: Option<U256>,
    /// Gas limit
    pub gas_limit: U256,
    /// Recipient address (None for contract creation)
    pub to: Option<Address>,
    /// Value (in wei)
    pub value: U256,
    /// Input data
    pub input: Vec<u8>,
    /// Access list (EIP-2930 and EIP-1559)
    pub access_list: AccessList,
    /// Chain ID
    pub chain_id: u64,
}

impl UnsignedTransaction {
    /// Create a new legacy transaction
    pub fn new_legacy(
        nonce: u64,
        gas_price: U256,
        gas_limit: U256,
        to: Option<Address>,
        value: U256,
        input: Vec<u8>,
        chain_id: u64,
    ) -> Self {
        Self {
            tx_type: TransactionType::Legacy,
            nonce,
            gas_price: Some(gas_price),
            max_priority_fee_per_gas: None,
            max_fee_per_gas: None,
            gas_limit,
            to,
            value,
            input,
            access_list: Vec::new(),
            chain_id,
        }
    }

    /// Create a new EIP-1559 transaction
    pub fn new_eip1559(
        nonce: u64,
        max_priority_fee_per_gas: U256,
        max_fee_per_gas: U256,
        gas_limit: U256,
        to: Option<Address>,
        value: U256,
        input: Vec<u8>,
        chain_id: u64,
    ) -> Self {
        Self {
            tx_type: TransactionType::Eip1559,
            nonce,
            gas_price: None,
            max_priority_fee_per_gas: Some(max_priority_fee_per_gas),
            max_fee_per_gas: Some(max_fee_per_gas),
            gas_limit,
            to,
            value,
            input,
            access_list: Vec::new(),
            chain_id,
        }
    }

    /// Create contract creation transaction
    pub fn new_contract_creation(
        nonce: u64,
        gas_price: U256,
        gas_limit: U256,
        value: U256,
        init_code: Vec<u8>,
        chain_id: u64,
    ) -> Self {
        Self {
            tx_type: TransactionType::ContractCreation,
            nonce,
            gas_price: Some(gas_price),
            max_priority_fee_per_gas: None,
            max_fee_per_gas: None,
            gas_limit,
            to: None,
            value,
            input: init_code,
            access_list: Vec::new(),
            chain_id,
        }
    }

    /// Get effective gas price
    pub fn effective_gas_price(&self, base_fee: Option<U256>) -> U256 {
        match self.tx_type {
            TransactionType::Legacy | TransactionType::AccessList => {
                self.gas_price.unwrap_or_default()
            }
            TransactionType::Eip1559 => {
                if let Some(base_fee) = base_fee {
                    let priority_fee = self.max_priority_fee_per_gas.unwrap_or_default();
                    let max_fee = self.max_fee_per_gas.unwrap_or_default();

                    // effective_price = min(base_fee + priority_fee, max_fee)
                    let capped_fee = base_fee + priority_fee;
                    if capped_fee < max_fee {
                        capped_fee
                    } else {
                        max_fee
                    }
                } else {
                    self.max_fee_per_gas.unwrap_or_default()
                }
            }
            _ => U256::zero(),
        }
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
                &[signing_hash.as_bytes(), signature.as_bytes()].concat(),
            )),
        }
    }

    /// Encode to RLP (simplified)
    fn encode_rlp(&self) -> Vec<u8> {
        // Simplified RLP encoding for demonstration
        let mut data = Vec::new();
        data.extend_from_slice(&self.nonce.to_be_bytes());
        if let Some(gas_price) = self.gas_price {
            data.extend_from_slice(&gas_price.to_big_endian());
        }
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
        self.tx.gas_price.unwrap_or_default()
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
        self.signature
            .verify(
                signing_hash.as_bytes(),
                &crate::crypto::PublicKey::from_bytes({
                    let mut bytes = [0u8; 65];
                    bytes[0] = 0x04;
                    bytes
                })
                .unwrap(),
            )
            .map_err(|_| TransactionError::InvalidSignature)
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
        data.extend_from_slice(self.signature.as_bytes());
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
    pub logs_bloom: [u8; 256],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_creation() {
        let private_key = PrivateKey::random();

        let tx = UnsignedTransaction::new_legacy(
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

    #[test]
    fn test_eip1559_transaction() {
        let private_key = PrivateKey::random();

        let tx = UnsignedTransaction::new_eip1559(
            0,
            U256::from(100_000_000),
            U256::from(1_000_000_000),
            U256::from(21000),
            Some(Address::from_bytes([1u8; 20])),
            U256::from(1000),
            vec![],
            10086,
        );

        // Test effective gas price calculation
        let base_fee = U256::from(900_000_000);
        let effective_price = tx.effective_gas_price(Some(base_fee));

        // effective_price = min(base_fee + priority_fee, max_fee)
        // = min(900_000_000 + 100_000_000, 1_000_000_000)
        // = 1_000_000_000
        assert_eq!(effective_price, U256::from(1_000_000_000));
    }
}
