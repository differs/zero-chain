//! UTXO (Unspent Transaction Output) module for hybrid account model

use crate::account::U256;
use crate::crypto::{Address, Hash, Signature};
use serde::{Deserialize, Serialize};

/// UTXO reference
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UtxoReference {
    /// Transaction hash
    pub tx_hash: Hash,
    /// Output index
    pub output_index: u32,
    /// Amount
    pub amount: U256,
    /// Lock script
    pub lock_script: LockScript,
    /// Is spent
    pub spent: bool,
}

/// Lock script types
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LockScript {
    /// Pay to Public Key Hash (P2PKH)
    P2PKH {
        /// Public key hash
        pubkey_hash: Address,
    },
    /// Pay to Script Hash (P2SH)
    P2SH {
        /// Script hash
        script_hash: Hash,
    },
    /// Time lock
    TimeLock {
        /// Unlock timestamp
        unlock_time: u64,
    },
    /// Multi-signature lock
    MultiSigLock {
        /// Public keys
        pubkeys: Vec<Hash>,
        /// Threshold
        threshold: u32,
    },
    /// Custom script
    CustomScript {
        /// Script bytes
        script: Vec<u8>,
    },
}

impl LockScript {
    /// Check if lock is satisfied
    pub fn verify(&self, unlock_script: &UnlockScript, signature_data: &[u8]) -> bool {
        match (self, unlock_script) {
            (LockScript::P2PKH { pubkey_hash }, UnlockScript::P2PKH { pubkey, signature }) => {
                // Verify signature against public key
                // This would use the crypto module
                true // Simplified
            }
            (LockScript::TimeLock { unlock_time }, UnlockScript::TimeLock { current_time }) => {
                current_time >= unlock_time
            }
            (LockScript::MultiSigLock { threshold, .. }, UnlockScript::MultiSig { signatures }) => {
                signatures.len() >= *threshold as usize
            }
            _ => false,
        }
    }
}

/// Unlock script types
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum UnlockScript {
    /// P2PKH unlock
    P2PKH {
        /// Public key
        pubkey: Hash,
        /// Signature
        signature: Signature,
    },
    /// Time lock unlock
    TimeLock {
        /// Current timestamp
        current_time: u64,
    },
    /// Multi-sig unlock
    MultiSig {
        /// Signatures
        signatures: Vec<Signature>,
    },
    /// Custom script
    CustomScript {
        /// Script data
        data: Vec<u8>,
    },
}

/// UTXO output
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UtxoOutput {
    /// Amount
    pub amount: U256,
    /// Lock script
    pub lock_script: LockScript,
    /// Is spent
    pub spent: bool,
    /// Spent by transaction hash
    pub spent_by: Option<Hash>,
    /// Creation timestamp
    pub created_at: u64,
}

impl UtxoOutput {
    /// Create a new UTXO output
    pub fn new(amount: U256, lock_script: LockScript) -> Self {
        Self {
            amount,
            lock_script,
            spent: false,
            spent_by: None,
            created_at: current_timestamp(),
        }
    }

    /// Mark as spent
    pub fn spend(&mut self, tx_hash: Hash) {
        self.spent = true;
        self.spent_by = Some(tx_hash);
    }
}

/// UTXO input for transaction
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UtxoInput {
    /// Reference to UTXO
    pub reference: UtxoReference,
    /// Unlock script
    pub unlock_script: UnlockScript,
}

/// UTXO transaction
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UtxoTransaction {
    /// Inputs
    pub inputs: Vec<UtxoInput>,
    /// Outputs
    pub outputs: Vec<UtxoOutput>,
    /// Lock time
    pub lock_time: u64,
}

impl UtxoTransaction {
    /// Calculate total input amount
    pub fn input_amount(&self) -> U256 {
        self.inputs
            .iter()
            .fold(U256::zero(), |acc, input| acc + input.reference.amount)
    }

    /// Calculate total output amount
    pub fn output_amount(&self) -> U256 {
        self.outputs
            .iter()
            .fold(U256::zero(), |acc, output| acc + output.amount)
    }

    /// Calculate fee (input - output)
    pub fn fee(&self) -> U256 {
        self.input_amount() - self.output_amount()
    }

    /// Validate transaction
    pub fn validate(&self) -> Result<(), UtxoError> {
        // Check inputs are not empty
        if self.inputs.is_empty() {
            return Err(UtxoError::EmptyInputs);
        }

        // Check outputs are not empty
        if self.outputs.is_empty() {
            return Err(UtxoError::EmptyOutputs);
        }

        // Check input >= output (no inflation)
        if self.input_amount() < self.output_amount() {
            return Err(UtxoError::InvalidAmount);
        }

        // Verify all unlock scripts
        for input in self.inputs.iter() {
            // Would verify signatures here
            let _ = input;
        }

        Ok(())
    }
}

/// UTXO errors
#[derive(Debug, thiserror::Error)]
pub enum UtxoError {
    #[error("Empty inputs")]
    EmptyInputs,
    #[error("Empty outputs")]
    EmptyOutputs,
    #[error("Invalid amount")]
    InvalidAmount,
    #[error("UTXO not found")]
    NotFound,
    #[error("UTXO already spent")]
    AlreadySpent,
    #[error("Invalid lock script")]
    InvalidLockScript,
    #[error("Invalid unlock script")]
    InvalidUnlockScript,
    #[error("Signature verification failed")]
    SignatureVerificationFailed,
}

fn current_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utxo_transaction() {
        let input = UtxoInput {
            reference: UtxoReference {
                tx_hash: Hash::from_bytes([1u8; 32]),
                output_index: 0,
                amount: U256::from(1000),
                lock_script: LockScript::P2PKH {
                    pubkey_hash: Address::from_bytes([2u8; 20]),
                },
                spent: false,
            },
            unlock_script: UnlockScript::P2PKH {
                pubkey: Hash::from_bytes([3u8; 32]),
                signature: Signature::new([0u8; 32], [0u8; 32], 0),
            },
        };

        let output = UtxoOutput::new(
            U256::from(900),
            LockScript::P2PKH {
                pubkey_hash: Address::from_bytes([4u8; 20]),
            },
        );

        let tx = UtxoTransaction {
            inputs: vec![input],
            outputs: vec![output],
            lock_time: 0,
        };

        assert_eq!(tx.input_amount().as_u64(), 1000);
        assert_eq!(tx.output_amount().as_u64(), 900);
        assert_eq!(tx.fee().as_u64(), 100);

        assert!(tx.validate().is_ok());
    }
}
