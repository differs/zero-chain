//! Authorization/resource policy traits and default policies.

use super::{
    error::{ComputeError, ComputeResult},
    object::{ObjectOutput, Ownership},
    tx::{Command, ComputeTx, SignatureScheme, TxSignature},
};
use crate::crypto::Address;
use ed25519_dalek::{Signature as Ed25519Signature, Verifier as _};

/// Authorization policy checks ownership/capability validity.
pub trait AuthorizationPolicy: Send + Sync {
    /// Validates transaction authorization against resolved inputs/reads.
    fn authorize(
        &self,
        tx: &ComputeTx,
        inputs: &[ObjectOutput],
        reads: &[ObjectOutput],
    ) -> ComputeResult<()>;
}

/// Resource policy validates resource accounting constraints.
pub trait ResourcePolicy: Send + Sync {
    /// Checks resource constraints for one transaction context.
    fn check_resources(
        &self,
        tx: &ComputeTx,
        inputs: &[ObjectOutput],
        reads: &[ObjectOutput],
    ) -> ComputeResult<()>;
}

/// Default authorization policy for scaffolding stage.
#[derive(Debug, Default, Clone)]
pub struct DefaultAuthorizationPolicy;

impl AuthorizationPolicy for DefaultAuthorizationPolicy {
    fn authorize(
        &self,
        tx: &ComputeTx,
        inputs: &[ObjectOutput],
        _reads: &[ObjectOutput],
    ) -> ComputeResult<()> {
        if !tx.has_consistent_tx_id() {
            return Err(ComputeError::TxIdMismatch);
        }

        let required = tx.witness.threshold.unwrap_or(1) as usize;
        if required == 0 {
            return Err(ComputeError::AuthorizationDenied);
        }

        if tx.witness.signatures.len() < required {
            return Err(ComputeError::AuthorizationDenied);
        }

        // Minimal command-level restrictions for scaffolding stage.
        if matches!(tx.command, Command::Mint | Command::Burn) {
            for input in inputs {
                if !matches!(input.owner, Ownership::Program(_) | Ownership::Shared) {
                    return Err(ComputeError::AuthorizationDenied);
                }
            }
        }

        // Minimal owner presence check for transfer/invoke.
        if matches!(tx.command, Command::Transfer | Command::Invoke) {
            for input in inputs {
                if let Ownership::Address(addr) = input.owner {
                    if addr.is_zero() {
                        return Err(ComputeError::OwnershipCheckFailed);
                    }
                }
            }

            for input in inputs {
                match input.owner {
                    Ownership::Address(owner_addr) => {
                        if tx.witness.signatures.is_empty() {
                            return Err(ComputeError::AuthorizationDenied);
                        }
                        match secp_signature_matches_owner(tx, owner_addr, &tx.witness.signatures) {
                            Ok(true) => {}
                            Ok(false) => return Err(ComputeError::SignatureOwnerMismatch),
                            Err(err) => return Err(err),
                        }
                    }
                    Ownership::NativeEd25519(owner_pubkey) => {
                        if tx.witness.signatures.is_empty() {
                            return Err(ComputeError::AuthorizationDenied);
                        }
                        match ed25519_signature_matches_owner(
                            tx,
                            &owner_pubkey,
                            &tx.witness.signatures,
                        ) {
                            Ok(true) => {}
                            Ok(false) => return Err(ComputeError::SignatureOwnerMismatch),
                            Err(err) => return Err(err),
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

fn secp_signature_matches_owner(
    tx: &ComputeTx,
    owner: Address,
    signatures: &[TxSignature],
) -> ComputeResult<bool> {
    let message = tx.signing_preimage();
    let mut had_recover_error = false;

    for sig in signatures {
        if sig.scheme != SignatureScheme::Secp256k1 {
            continue;
        }
        let parsed = crate::crypto::Signature::from_bytes(&sig.bytes)
            .map_err(|_| ComputeError::InvalidSignature)?;
        match parsed.recover(&message) {
            Ok(pubkey) => {
                if Address::from_public_key(&pubkey) == owner {
                    return Ok(true);
                }
            }
            Err(_) => {
                had_recover_error = true;
            }
        }
    }

    if had_recover_error {
        Err(ComputeError::InvalidSignature)
    } else {
        Ok(false)
    }
}

fn ed25519_signature_matches_owner(
    tx: &ComputeTx,
    owner_pubkey: &[u8; 32],
    signatures: &[TxSignature],
) -> ComputeResult<bool> {
    let message = tx.signing_preimage();
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(owner_pubkey)
        .map_err(|_| ComputeError::OwnershipCheckFailed)?;

    for sig in signatures {
        if sig.scheme != SignatureScheme::Ed25519 {
            continue;
        }
        let Some(pk_bytes) = &sig.public_key else {
            return Err(ComputeError::InvalidSignature);
        };
        if pk_bytes.len() != 32 {
            return Err(ComputeError::InvalidSignature);
        }
        if pk_bytes.as_slice() != owner_pubkey {
            continue;
        }
        let sig_obj =
            Ed25519Signature::from_slice(&sig.bytes).map_err(|_| ComputeError::InvalidSignature)?;
        if verifying_key.verify(&message, &sig_obj).is_ok() {
            return Ok(true);
        }
    }

    Ok(false)
}

/// No-op resource policy.
#[derive(Debug, Default, Clone)]
pub struct NoopResourcePolicy;

impl ResourcePolicy for NoopResourcePolicy {
    fn check_resources(
        &self,
        _tx: &ComputeTx,
        _inputs: &[ObjectOutput],
        _reads: &[ObjectOutput],
    ) -> ComputeResult<()> {
        Ok(())
    }
}
