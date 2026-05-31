//! Block module

use crate::account::U256;
use crate::crypto::{Address, Hash};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Block errors
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum BlockError {
    #[error("Invalid parent hash")]
    InvalidParentHash,
    #[error("Invalid block number")]
    InvalidBlockNumber,
    #[error("Invalid timestamp")]
    InvalidTimestamp,
    #[error("Invalid difficulty")]
    InvalidDifficulty,
    #[error("Invalid PoW")]
    InvalidPow,
    #[error("Gas limit too high")]
    GasLimitTooHigh,
    #[error("Extra data too large")]
    ExtraDataTooLarge,
    #[error("Invalid transaction root")]
    InvalidTransactionRoot,
    #[error("Invalid state root")]
    InvalidStateRoot,
    #[error("Block too large")]
    BlockTooLarge,
}

/// Block header
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockHeader {
    pub version: u32,
    pub parent_hash: Hash,
    pub uncle_hashes: Vec<Hash>,
    pub coinbase: Address,
    pub state_root: Hash,
    pub transactions_root: Hash,
    pub receipts_root: Hash,
    pub number: U256,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub timestamp: u64,
    pub difficulty: U256,
    pub nonce: u64,
    pub extra_data: Vec<u8>,
    pub mix_hash: Hash,
    pub base_fee_per_gas: U256,
    #[serde(skip)]
    pub hash: Hash,
}

impl BlockHeader {
    pub fn compute_hash(&self) -> Hash {
        let encoded = self.encode_rlp();
        Hash::from_bytes(crate::crypto::keccak256(&encoded))
    }

    pub fn validate(&self, parent: &BlockHeader) -> Result<(), BlockError> {
        if self.parent_hash != parent.hash {
            return Err(BlockError::InvalidParentHash);
        }

        if self.number != parent.number + U256::one() {
            return Err(BlockError::InvalidBlockNumber);
        }

        if self.timestamp <= parent.timestamp {
            return Err(BlockError::InvalidTimestamp);
        }

        if self.extra_data.len() > 32 {
            return Err(BlockError::ExtraDataTooLarge);
        }

        Ok(())
    }

    pub fn verify_pow(&self) -> Result<(), BlockError> {
        let target = pow_target_from_difficulty(self.difficulty);
        let pow_hash = compute_pow_hash(self, self.nonce);

        if !pow_hash_meets_target(pow_hash.as_bytes(), target) {
            return Err(BlockError::InvalidPow);
        }

        Ok(())
    }

    fn encode_rlp(&self) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&self.version.to_be_bytes());
        data.extend_from_slice(self.parent_hash.as_bytes());
        data.extend_from_slice(&self.number.to_big_endian());
        data.extend_from_slice(&self.timestamp.to_be_bytes());
        data.extend_from_slice(&self.nonce.to_be_bytes());
        data.extend_from_slice(self.difficulty.to_big_endian().as_ref());
        data
    }
}

/// Complete block
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    pub uncles: Vec<BlockHeader>,
}

impl Block {
    pub fn new(header: BlockHeader) -> Self {
        Self {
            header,
            uncles: Vec::new(),
        }
    }

    pub fn encode_rlp(&self) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&self.header.number.to_big_endian());
        data
    }
}

/// Genesis block
pub fn create_genesis_block() -> Block {
    let header = BlockHeader {
        version: 1,
        parent_hash: Hash::zero(),
        uncle_hashes: Vec::new(),
        coinbase: Address::zero(),
        state_root: Hash::from_bytes([0u8; 32]),
        transactions_root: Hash::from_bytes([0u8; 32]),
        receipts_root: Hash::from_bytes([0u8; 32]),
        number: U256::zero(),
        gas_limit: 30_000_000,
        gas_used: 0,
        timestamp: 0,
        difficulty: U256::from_u128(1_000_000_000_000_000u128),
        nonce: 0,
        extra_data: b"ZeroChain Genesis".to_vec(),
        mix_hash: Hash::zero(),
        base_fee_per_gas: U256::from(1_000_000_000),
        hash: Hash::zero(),
    };

    let hash = header.compute_hash();
    let mut header = header;
    header.hash = hash;

    Block {
        header,
        uncles: Vec::new(),
    }
}

pub fn max_pow_target() -> U256 {
    U256::from_big_endian(&[0xFFu8; 32])
}

pub fn pow_target_from_difficulty(difficulty: U256) -> U256 {
    if difficulty.is_zero() {
        return max_pow_target();
    }
    let divisor = difficulty.as_u128();
    if divisor == 0 {
        return U256::zero();
    }
    if divisor > (u128::MAX >> 8) {
        return U256::zero();
    }

    let mut quotient = [0u8; 32];
    let mut remainder = 0u128;
    for slot in &mut quotient {
        let value = remainder * 256 + 0xFF;
        *slot = (value / divisor).min(0xFF) as u8;
        remainder = value % divisor;
    }
    U256::from_big_endian(&quotient)
}

pub fn pow_hash_meets_target(pow_hash: &[u8], target: U256) -> bool {
    U256::from_big_endian(pow_hash) <= target
}

pub fn pow_target_to_hex(target: U256) -> String {
    format!("0x{}", hex::encode(target.to_big_endian()))
}

pub fn pow_target_from_hex(input: &str) -> Result<U256, String> {
    let decoded = hex::decode(input.strip_prefix("0x").unwrap_or(input))
        .map_err(|err| format!("invalid pow target hex: {err}"))?;
    if decoded.len() != 32 {
        return Err(format!(
            "pow target must decode to 32 bytes, got {}",
            decoded.len()
        ));
    }
    Ok(U256::from_big_endian(&decoded))
}

fn compute_pow_hash(header: &BlockHeader, nonce: u64) -> Hash {
    let mut data = header.encode_rlp();
    data.extend_from_slice(&nonce.to_be_bytes());
    Hash::from_bytes(crate::crypto::keccak256(&data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_genesis_block() {
        let genesis = create_genesis_block();

        assert_eq!(genesis.header.number, U256::zero());
        assert_eq!(genesis.header.parent_hash, Hash::zero());
    }

    #[test]
    fn test_pow_target_uses_full_256_bit_comparison() {
        let mut target_bytes = [0xFFu8; 32];
        target_bytes[..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x10]);
        let target = U256::from_big_endian(&target_bytes);
        let mut below = [0u8; 32];
        below[..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x10]);
        let mut above = [0u8; 32];
        above[..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x11]);

        assert!(pow_hash_meets_target(&below, target));
        assert!(!pow_hash_meets_target(&above, target));
    }

    #[test]
    fn test_pow_target_from_difficulty_is_continuous_below_byte_boundary() {
        let target = pow_target_from_difficulty(U256::from_u128(1_000_000));
        assert!(!target.is_zero());
        assert_eq!(target.leading_zeros() / 8, 2);
        assert!(
            target
                < U256::from_big_endian(&[
                    0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
                    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
                    0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
                ])
        );
    }
}
