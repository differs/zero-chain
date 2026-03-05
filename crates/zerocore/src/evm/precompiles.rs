//! EVM Precompiled Contracts

use crate::crypto::{keccak256, Address, Hash};
use crate::evm::EvmError;

/// Precompiled contract addresses
pub const ECRECOVER_ADDRESS: u16 = 0x01;
pub const SHA256_ADDRESS: u16 = 0x02;
pub const RIPEMD160_ADDRESS: u16 = 0x03;
pub const IDENTITY_ADDRESS: u16 = 0x04;
pub const MODEXP_ADDRESS: u16 = 0x05;
pub const ECADD_ADDRESS: u16 = 0x06;
pub const ECMUL_ADDRESS: u16 = 0x07;
pub const ECPAIRING_ADDRESS: u16 = 0x08;
pub const BLAKE2F_ADDRESS: u16 = 0x09;

/// ZeroChain custom precompiles
pub const ACCOUNT_VALIDATOR_ADDRESS: u16 = 0x100;
pub const UTXO_VALIDATOR_ADDRESS: u16 = 0x101;
pub const BATCH_TRANSFER_ADDRESS: u16 = 0x102;

/// Check if address is a precompiled contract
pub fn is_precompile(address: &Address) -> bool {
    let addr_bytes = address.as_bytes();
    let addr_u16 = u16::from_be_bytes([addr_bytes[18], addr_bytes[19]]);
    addr_u16 >= 1 && addr_u16 <= 9
        || addr_u16 >= ACCOUNT_VALIDATOR_ADDRESS && addr_u16 <= BATCH_TRANSFER_ADDRESS
}

/// Get precompile gas cost
pub fn precompile_gas_cost(address: u16, input_size: usize) -> u64 {
    match address {
        ECRECOVER_ADDRESS => 3000,
        SHA256_ADDRESS => {
            let words = ((input_size + 31) / 32) as u64;
            60 + 12 * words
        }
        RIPEMD160_ADDRESS => {
            let words = ((input_size + 31) / 32) as u64;
            600 + 120 * words
        }
        IDENTITY_ADDRESS => {
            let words = ((input_size + 31) / 32) as u64;
            15 + 3 * words
        }
        MODEXP_ADDRESS => {
            // Simplified gas calculation for MODEXP
            200
        }
        ECADD_ADDRESS => 150,
        ECMUL_ADDRESS => 6000,
        ECPAIRING_ADDRESS => {
            let pairs = input_size / 192;
            45000 + 34000 * pairs as u64
        }
        BLAKE2F_ADDRESS => {
            if input_size == 213 {
                1
            } else {
                0 // Invalid input
            }
        }
        ACCOUNT_VALIDATOR_ADDRESS => 5000,
        UTXO_VALIDATOR_ADDRESS => 10000,
        BATCH_TRANSFER_ADDRESS => 21000,
        _ => 0,
    }
}

/// Execute precompiled contract
pub fn execute_precompile(address: u16, input: &[u8], gas_limit: u64) -> Result<Vec<u8>, EvmError> {
    let required_gas = precompile_gas_cost(address, input.len());

    if gas_limit < required_gas {
        return Err(EvmError::OutOfGas);
    }

    match address {
        ECRECOVER_ADDRESS => ecrecover(input),
        SHA256_ADDRESS => sha256_precompile(input),
        RIPEMD160_ADDRESS => ripemd160_precompile(input),
        IDENTITY_ADDRESS => identity(input),
        MODEXP_ADDRESS => modexp(input),
        ECADD_ADDRESS => ecadd(input),
        ECMUL_ADDRESS => ecmul(input),
        ECPAIRING_ADDRESS => ecpairing(input),
        BLAKE2F_ADDRESS => blake2f(input),
        ACCOUNT_VALIDATOR_ADDRESS => account_validator(input),
        UTXO_VALIDATOR_ADDRESS => utxo_validator(input),
        BATCH_TRANSFER_ADDRESS => batch_transfer(input),
        _ => Err(EvmError::PrecompileError(format!(
            "Unknown precompile address: 0x{:04x}",
            address
        ))),
    }
}

/// ECDSA public key recovery
fn ecrecover(input: &[u8]) -> Result<Vec<u8>, EvmError> {
    // Input format: 32 bytes hash, 32 bytes v, 32 bytes r, 32 bytes s
    if input.len() < 128 {
        return Ok(Vec::new());
    }

    let hash = &input[0..32];
    let v = input[64];
    let r = &input[65..97];
    let s = &input[97..129];

    // Simplified implementation - in production, use secp256k1
    // This is a placeholder that returns a dummy address
    let mut result = vec![0u8; 32];

    // Hash the input to create a deterministic "recovered" address
    let recovered = keccak256(hash);
    result[12..].copy_from_slice(&recovered[12..]);

    Ok(result)
}

/// SHA256 hash
fn sha256_precompile(input: &[u8]) -> Result<Vec<u8>, EvmError> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(input);
    let result = hasher.finalize();

    // Pad to 32 bytes
    let mut output = vec![0u8; 32];
    output.copy_from_slice(&result);

    Ok(output)
}

/// RIPEMD160 hash
fn ripemd160_precompile(input: &[u8]) -> Result<Vec<u8>, EvmError> {
    use ripemd::{Digest, Ripemd160};

    let mut hasher = Ripemd160::new();
    hasher.update(input);
    let result = hasher.finalize();

    // Pad to 32 bytes (RIPEMD160 produces 20 bytes)
    let mut output = vec![0u8; 32];
    output[12..].copy_from_slice(&result);

    Ok(output)
}

/// Identity function (returns input unchanged)
fn identity(input: &[u8]) -> Result<Vec<u8>, EvmError> {
    Ok(input.to_vec())
}

/// Modular exponentiation
fn modexp(input: &[u8]) -> Result<Vec<u8>, EvmError> {
    // Simplified implementation
    // In production, implement full MODEXP algorithm
    Ok(vec![0u8; 32])
}

/// Elliptic curve addition
fn ecadd(input: &[u8]) -> Result<Vec<u8>, EvmError> {
    // Simplified implementation
    // In production, implement BN256 curve addition
    if input.len() < 128 {
        return Err(EvmError::PrecompileError(
            "Invalid input length".to_string(),
        ));
    }

    Ok(vec![0u8; 64])
}

/// Elliptic curve multiplication
fn ecmul(input: &[u8]) -> Result<Vec<u8>, EvmError> {
    // Simplified implementation
    // In production, implement BN256 curve multiplication
    if input.len() < 96 {
        return Err(EvmError::PrecompileError(
            "Invalid input length".to_string(),
        ));
    }

    Ok(vec![0u8; 64])
}

/// Elliptic curve pairing check
fn ecpairing(input: &[u8]) -> Result<Vec<u8>, EvmError> {
    // Simplified implementation
    // In production, implement BN256 pairing check

    // Return 1 (true) or 0 (false) in the last byte
    let mut result = vec![0u8; 32];
    result[31] = 1; // Simplified: always return true

    Ok(result)
}

/// BLAKE2b compression function
fn blake2f(input: &[u8]) -> Result<Vec<u8>, EvmError> {
    // Input must be exactly 213 bytes
    if input.len() != 213 {
        return Err(EvmError::PrecompileError(
            "Invalid input length".to_string(),
        ));
    }

    // Simplified implementation
    // In production, implement full BLAKE2b compression
    let mut result = vec![0u8; 64];

    // Copy input rounds (first 4 bytes) to determine if we need to process
    let rounds = u32::from_be_bytes([input[0], input[1], input[2], input[3]]);

    if rounds > 0 {
        // In production, actually run BLAKE2b compression
        // For now, just hash the input
        let hash = keccak256(input);
        result.copy_from_slice(&hash);
    }

    Ok(result)
}

/// Account abstraction validator (ZeroChain custom)
fn account_validator(input: &[u8]) -> Result<Vec<u8>, EvmError> {
    // Validate account abstraction signature
    // Input: account address + signature data
    // Output: 1 if valid, 0 if invalid

    if input.len() < 20 {
        return Err(EvmError::PrecompileError(
            "Invalid input length".to_string(),
        ));
    }

    // Simplified validation
    let mut result = vec![0u8; 32];
    result[31] = 1; // In production, actually validate

    Ok(result)
}

/// UTXO validator (ZeroChain custom)
fn utxo_validator(input: &[u8]) -> Result<Vec<u8>, EvmError> {
    // Validate UTXO spending
    // Input: UTXO reference + proof
    // Output: 1 if valid, 0 if invalid

    if input.len() < 32 {
        return Err(EvmError::PrecompileError(
            "Invalid input length".to_string(),
        ));
    }

    // Simplified validation
    let mut result = vec![0u8; 32];
    result[31] = 1; // In production, actually validate

    Ok(result)
}

/// Batch transfer optimizer (ZeroChain custom)
fn batch_transfer(input: &[u8]) -> Result<Vec<u8>, EvmError> {
    // Optimize batch transfers
    // Input: array of (address, amount) pairs
    // Output: success status

    if input.len() < 52 {
        // At least one transfer (20 bytes address + 32 bytes amount)
        return Err(EvmError::PrecompileError(
            "Invalid input length".to_string(),
        ));
    }

    // Simplified implementation
    let mut result = vec![0u8; 32];
    result[31] = 1; // Success

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_precompile() {
        let ecrecover_addr = Address::from_bytes(
            [0u8; 18]
                .into_iter()
                .chain([0x00, 0x01].into_iter())
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        );

        assert!(is_precompile(&ecrecover_addr));

        let normal_addr = Address::from_bytes([0x12; 20]);
        assert!(!is_precompile(&normal_addr));
    }

    #[test]
    fn test_identity_precompile() {
        let input = b"hello world";
        let result = execute_precompile(IDENTITY_ADDRESS, input, 100).unwrap();
        assert_eq!(result, input);
    }

    #[test]
    fn test_sha256_precompile() {
        let input = b"test";
        let result = execute_precompile(SHA256_ADDRESS, input, 1000);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 32);
    }
}
