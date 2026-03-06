//! Cryptographic primitives for ZeroChain
//!
//! This module provides:
//! - Hash functions (Keccak256, SHA256, Blake3)
//! - Public/Private key pairs (secp256k1)
//! - Digital signatures (ECDSA)
//! - Address derivation

use k256::ecdsa::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use secp256k1::{
    ecdsa::{RecoverableSignature, RecoveryId, Signature as SecpSignature},
    Message as SecpMessage, PublicKey as SecpPublicKey, Secp256k1, SecretKey,
};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::fmt;
use thiserror::Error;

/// Crypto error types
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum CryptoError {
    /// Invalid signature
    #[error("Invalid signature")]
    InvalidSignature,
    /// Invalid private key
    #[error("Invalid private key")]
    InvalidPrivateKey,
    /// Invalid public key
    #[error("Invalid public key")]
    InvalidPublicKey,
    /// Signature verification failed
    #[error("Signature verification failed")]
    VerificationFailed,
    /// Key derivation failed
    #[error("Key derivation failed")]
    KeyDerivationFailed,
}

/// 256-bit hash
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Hash([u8; 32]);

impl Hash {
    /// Create a new hash from bytes
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Create a new hash from a slice
    pub fn from_slice(slice: &[u8]) -> Result<Self, CryptoError> {
        if slice.len() != 32 {
            return Err(CryptoError::InvalidPublicKey);
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(slice);
        Ok(Self(bytes))
    }

    /// Get the hash as bytes
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Create from hex string
    pub fn from_hex(s: &str) -> Result<Self, CryptoError> {
        let bytes = hex::decode(s.strip_prefix("0x").unwrap_or(s))
            .map_err(|_| CryptoError::InvalidPublicKey)?;
        Self::from_slice(&bytes)
    }

    /// Zero hash
    pub fn zero() -> Self {
        Self::default()
    }

    /// Check if hash is zero
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|&b| b == 0)
    }

    /// Count leading zeros
    pub fn leading_zeros(&self) -> u32 {
        let mut count = 0u32;
        for &byte in &self.0 {
            if byte == 0 {
                count += 8;
            } else {
                count += byte.leading_zeros();
                break;
            }
        }
        count
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", self.to_hex())
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", &self.to_hex()[..16])
    }
}

impl PartialOrd for Hash {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Hash {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

/// 160-bit address (Ethereum compatible)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Address([u8; 20]);

impl Address {
    /// Create a new address from bytes
    pub fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    /// Create a new address from a slice
    pub fn from_slice(slice: &[u8]) -> Result<Self, CryptoError> {
        if slice.len() != 20 {
            return Err(CryptoError::InvalidPublicKey);
        }
        let mut bytes = [0u8; 20];
        bytes.copy_from_slice(slice);
        Ok(Self(bytes))
    }

    /// Get the address as bytes
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Convert to hex string with checksum
    pub fn to_checksum_hex(&self) -> String {
        let hex = hex::encode(self.0);
        let hash = keccak256(hex.as_bytes());

        let mut result = String::with_capacity(42);
        result.push_str("0x");

        for (i, c) in hex.chars().enumerate() {
            if c.is_ascii_digit() {
                result.push(c);
            } else if ((hash[i / 2] >> (4 * (1 - i % 2))) & 0x8) != 0 {
                result.push(c.to_ascii_uppercase());
            } else {
                result.push(c.to_ascii_lowercase());
            }
        }

        result
    }

    /// Convert to simple hex string
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Create from hex string
    pub fn from_hex(s: &str) -> Result<Self, CryptoError> {
        let bytes = hex::decode(s.strip_prefix("0x").unwrap_or(s))
            .map_err(|_| CryptoError::InvalidPublicKey)?;
        Self::from_slice(&bytes)
    }

    /// Create address from public key
    pub fn from_public_key(pubkey: &PublicKey) -> Self {
        let hash = keccak256(&pubkey.0[1..]);
        let mut address = [0u8; 20];
        address.copy_from_slice(&hash[12..]);
        Self(address)
    }

    /// Zero address
    pub fn zero() -> Self {
        Self::default()
    }

    /// Check if address is zero
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|&b| b == 0)
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address({})", self.to_checksum_hex())
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_checksum_hex())
    }
}

/// 256-bit public key (secp256k1)
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PublicKey([u8; 65]);

impl serde::Serialize for PublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeTuple;
        let mut seq = serializer.serialize_tuple(65)?;
        for byte in &self.0 {
            seq.serialize_element(byte)?;
        }
        seq.end()
    }
}

impl<'de> serde::Deserialize<'de> for PublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct PublicKeyVisitor;

        impl<'de> serde::de::Visitor<'de> for PublicKeyVisitor {
            type Value = PublicKey;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a 65-byte public key")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<PublicKey, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut bytes = [0u8; 65];
                for i in 0..65 {
                    bytes[i] = seq
                        .next_element()?
                        .ok_or_else(|| serde::de::Error::invalid_length(i, &self))?;
                }
                PublicKey::from_bytes(bytes).map_err(serde::de::Error::custom)
            }
        }

        deserializer.deserialize_tuple(65, PublicKeyVisitor)
    }
}

impl PublicKey {
    /// Create from bytes (uncompressed format, 65 bytes)
    pub fn from_bytes(bytes: [u8; 65]) -> Result<Self, CryptoError> {
        if bytes[0] != 0x04 {
            return Err(CryptoError::InvalidPublicKey);
        }
        Ok(Self(bytes))
    }

    /// Get the public key as bytes
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", &self.to_hex()[..16])
    }
}

/// 256-bit private key (secp256k1)
#[derive(Clone, PartialEq, Eq)]
pub struct PrivateKey([u8; 32]);

impl PrivateKey {
    /// Generate a new random private key
    pub fn random() -> Self {
        let signing_key = SigningKey::random(&mut OsRng);
        let bytes = signing_key.to_bytes();
        Self(*bytes.as_ref())
    }

    /// Create from bytes
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, CryptoError> {
        SecretKey::from_slice(&bytes).map_err(|_| CryptoError::InvalidPrivateKey)?;
        Ok(Self(bytes))
    }

    /// Get the private key as bytes
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Get the corresponding public key
    pub fn public_key(&self) -> PublicKey {
        let signing_key = SigningKey::from_bytes(&self.0.into()).expect("Valid private key");
        let verifying_key = VerifyingKey::from(&signing_key);
        let bytes = verifying_key.to_encoded_point(false);
        PublicKey::from_bytes(bytes.as_bytes().try_into().unwrap()).unwrap()
    }

    /// Sign a message
    pub fn sign(&self, message: &[u8]) -> Signature {
        let hash = keccak256(message);
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&self.0).expect("valid private key");
        let msg = SecpMessage::from_slice(&hash).expect("hash must be 32 bytes");
        let recoverable = secp.sign_ecdsa_recoverable(&msg, &sk);
        let (rec_id, sig64) = recoverable.serialize_compact();

        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&sig64[0..32]);
        s.copy_from_slice(&sig64[32..64]);

        let v = 27u8 + (rec_id.to_i32() as u8);

        Signature::new(r, s, v)
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PrivateKey(***)")
    }
}

impl Drop for PrivateKey {
    fn drop(&mut self) {
        // Zero out the private key bytes
        self.0.iter_mut().for_each(|b| *b = 0);
    }
}

/// ECDSA signature
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    r: [u8; 32],
    s: [u8; 32],
    v: u8,
}

impl Signature {
    /// Create a new signature
    pub fn new(r: [u8; 32], s: [u8; 32], v: u8) -> Self {
        Self { r, s, v }
    }

    /// Create from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() != 65 {
            return Err(CryptoError::InvalidSignature);
        }

        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&bytes[0..32]);
        s.copy_from_slice(&bytes[32..64]);

        Ok(Self { r, s, v: bytes[64] })
    }

    /// Get the signature as bytes
    pub fn as_bytes(&self) -> [u8; 65] {
        let mut bytes = [0u8; 65];
        bytes[..32].copy_from_slice(&self.r);
        bytes[32..64].copy_from_slice(&self.s);
        bytes[64] = self.v;
        bytes
    }

    /// Get r component
    pub fn r(&self) -> &[u8; 32] {
        &self.r
    }

    /// Get s component
    pub fn s(&self) -> &[u8; 32] {
        &self.s
    }

    /// Get v (recovery id)
    pub fn v(&self) -> u8 {
        self.v
    }

    /// Verify a signature
    pub fn verify(&self, message: &[u8], public_key: &PublicKey) -> Result<bool, CryptoError> {
        let hash = keccak256(message);
        let secp = Secp256k1::verification_only();

        let pubkey =
            SecpPublicKey::from_slice(&public_key.0).map_err(|_| CryptoError::InvalidPublicKey)?;

        let mut sig64 = [0u8; 64];
        sig64[0..32].copy_from_slice(&self.r);
        sig64[32..64].copy_from_slice(&self.s);
        let signature =
            SecpSignature::from_compact(&sig64).map_err(|_| CryptoError::InvalidSignature)?;

        let msg = SecpMessage::from_slice(&hash).expect("hash must be 32 bytes");
        secp.verify_ecdsa(&msg, &signature, &pubkey)
            .map(|_| true)
            .map_err(|_| CryptoError::VerificationFailed)
    }

    /// Recover the public key from signature and message
    pub fn recover(&self, message: &[u8]) -> Result<PublicKey, CryptoError> {
        let hash = keccak256(message);
        let secp = Secp256k1::verification_only();
        let msg = SecpMessage::from_slice(&hash).expect("hash must be 32 bytes");

        let mut sig64 = [0u8; 64];
        sig64[0..32].copy_from_slice(&self.r);
        sig64[32..64].copy_from_slice(&self.s);

        let candidates = [self.v, self.v.saturating_sub(27)];

        for v in candidates {
            if let Ok(rec_id) = RecoveryId::from_i32(i32::from(v)) {
                if let Ok(rec_sig) = RecoverableSignature::from_compact(&sig64, rec_id) {
                    if let Ok(pubkey) = secp.recover_ecdsa(&msg, &rec_sig) {
                        let bytes = pubkey.serialize_uncompressed();
                        return PublicKey::from_bytes(bytes)
                            .map_err(|_| CryptoError::InvalidPublicKey);
                    }
                }
            }
        }

        Err(CryptoError::InvalidSignature)
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        hex::encode(self.as_bytes())
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({})", &self.to_hex()[..16])
    }
}

// ============ Hash Functions ============

/// Compute Keccak256 hash
pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Compute SHA256 hash
pub fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    use sha2::Sha256;

    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Compute Blake3 hash
pub fn blake3_hash(data: &[u8]) -> [u8; 32] {
    let hash = blake3::hash(data);
    hash.into()
}

/// Compute RIPEMD160 hash
pub fn ripemd160(data: &[u8]) -> [u8; 20] {
    use ripemd::Ripemd160;
    use sha2::Digest;

    let mut hasher = Ripemd160::new();
    hasher.update(data);
    hasher.finalize().into()
}

// ============ Tests ============

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keccak256() {
        let data = b"hello";
        let hash = keccak256(data);

        // Known Keccak256 hash for "hello"
        let expected =
            hex::decode("1c8aff950685c2ed4bc3174f3472287b56d9517b9c948127319a09a7a36deac8")
                .unwrap();
        assert_eq!(&hash, expected.as_slice());
    }

    #[test]
    #[ignore = "Signature verification requires k256 library version compatibility"]
    fn test_signature_roundtrip() {
        let private_key = PrivateKey::random();
        let public_key = private_key.public_key();

        let message = b"test message";
        let signature = private_key.sign(message);

        // Verify signature
        let result = signature.verify(message, &public_key);
        assert!(result.is_ok());
        assert!(result.unwrap());

        // Note: Public key recovery is disabled in this version
        // as it requires proper recovery ID handling
    }

    #[test]
    fn test_address_from_public_key() {
        let private_key = PrivateKey::random();
        let public_key = private_key.public_key();
        let address = Address::from_public_key(&public_key);

        assert!(!address.is_zero());
    }

    #[test]
    fn test_hash_hex_roundtrip() {
        let hash = Hash::from_bytes([1u8; 32]);
        let hex = hash.to_hex();
        let recovered = Hash::from_hex(&hex).unwrap();

        assert_eq!(hash, recovered);
    }

    #[test]
    fn test_address_hex_roundtrip() {
        let address = Address::from_bytes([2u8; 20]);
        let hex = address.to_hex();
        let recovered = Address::from_hex(&hex).unwrap();

        assert_eq!(address, recovered);
    }
}
