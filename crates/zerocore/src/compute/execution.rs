//! Transaction validation/execution skeleton for UTXO Compute v1.1.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use super::{
    domain::DomainRegistry,
    error::{ComputeError, ComputeResult},
    object::ObjectOutput,
    policy::{AuthorizationPolicy, ResourcePolicy},
    primitives::{ObjectId, OutputId},
    tx::{ComputeTx, OutputProposal},
};

/// Object storage abstraction.
pub trait ObjectStore: Send + Sync {
    /// Fetches one output by id.
    fn get_output(&self, output_id: OutputId) -> Option<ObjectOutput>;
    /// Fetches latest output for a logical object id.
    fn get_latest_output_by_object(&self, object_id: ObjectId) -> Option<ObjectOutput>;
    /// Inserts output if absent.
    fn insert_output(&self, output: ObjectOutput) -> ComputeResult<()>;
    /// Marks output as spent.
    fn mark_spent(&self, output_id: OutputId) -> ComputeResult<()>;
}

/// In-memory object store.
#[derive(Default)]
pub struct InMemoryObjectStore {
    outputs: RwLock<HashMap<OutputId, ObjectOutput>>,
    latest_by_object: RwLock<HashMap<ObjectId, OutputId>>,
}

impl InMemoryObjectStore {
    /// Creates an empty object store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ObjectStore for InMemoryObjectStore {
    fn get_output(&self, output_id: OutputId) -> Option<ObjectOutput> {
        self.outputs.read().get(&output_id).cloned()
    }

    fn get_latest_output_by_object(&self, object_id: ObjectId) -> Option<ObjectOutput> {
        let latest = self.latest_by_object.read().get(&object_id).copied()?;
        self.outputs.read().get(&latest).cloned()
    }

    fn insert_output(&self, output: ObjectOutput) -> ComputeResult<()> {
        let mut guard = self.outputs.write();
        if guard.contains_key(&output.output_id) {
            return Err(ComputeError::DuplicateOutputId);
        }
        {
            let mut latest = self.latest_by_object.write();
            let should_update = match latest.get(&output.object_id).copied() {
                Some(prev_id) => guard
                    .get(&prev_id)
                    .map(|prev| output.version >= prev.version)
                    .unwrap_or(true),
                None => true,
            };
            if should_update {
                latest.insert(output.object_id, output.output_id);
            }
        }
        guard.insert(output.output_id, output);
        Ok(())
    }

    fn mark_spent(&self, output_id: OutputId) -> ComputeResult<()> {
        let mut guard = self.outputs.write();
        let Some(existing) = guard.get_mut(&output_id) else {
            return Err(ComputeError::ObjectNotFound(output_id.0));
        };

        if existing.spent {
            return Err(ComputeError::InvalidTransaction(
                "double spend detected".to_string(),
            ));
        }

        existing.spent = true;
        Ok(())
    }
}

impl<S: ObjectStore + ?Sized> ObjectStore for Arc<S> {
    fn get_output(&self, output_id: OutputId) -> Option<ObjectOutput> {
        self.as_ref().get_output(output_id)
    }

    fn get_latest_output_by_object(&self, object_id: ObjectId) -> Option<ObjectOutput> {
        self.as_ref().get_latest_output_by_object(object_id)
    }

    fn insert_output(&self, output: ObjectOutput) -> ComputeResult<()> {
        self.as_ref().insert_output(output)
    }

    fn mark_spent(&self, output_id: OutputId) -> ComputeResult<()> {
        self.as_ref().mark_spent(output_id)
    }
}

/// Validation result details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationReport {
    /// Resolved input objects.
    pub inputs: Vec<ObjectOutput>,
    /// Resolved read-set objects.
    pub reads: Vec<ObjectOutput>,
}

/// Stateless validator over object store and policies.
pub struct BasicTxValidator<
    'a,
    S: ObjectStore,
    A: AuthorizationPolicy,
    R: ResourcePolicy,
    D: DomainRegistry,
> {
    /// Object store backend.
    pub store: &'a S,
    /// Authorization policy.
    pub authorization: &'a A,
    /// Resource policy.
    pub resources: &'a R,
    /// Domain registry.
    pub domains: &'a D,
}

impl<'a, S: ObjectStore, A: AuthorizationPolicy, R: ResourcePolicy, D: DomainRegistry>
    BasicTxValidator<'a, S, A, R, D>
{
    /// Validates one transaction and returns resolved context.
    pub fn validate(&self, tx: &ComputeTx) -> ComputeResult<ValidationReport> {
        if !tx.basic_sanity_check() {
            return Err(ComputeError::InvalidTransaction(
                "transaction failed basic sanity check".to_string(),
            ));
        }

        let Some(domain) = self.domains.get_domain(tx.domain_id) else {
            return Err(ComputeError::DomainNotRegistered(tx.domain_id.0));
        };
        if !domain.public {
            return Err(ComputeError::DomainNotPublic(tx.domain_id.0));
        }

        let mut inputs = Vec::with_capacity(tx.input_set.len());
        for id in &tx.input_set {
            let Some(output) = self.store.get_output(*id) else {
                return Err(ComputeError::ObjectNotFound(id.0));
            };
            if output.spent {
                return Err(ComputeError::InvalidTransaction(
                    "input already spent".to_string(),
                ));
            }
            if output.domain_id != tx.domain_id {
                return Err(ComputeError::DomainMismatch {
                    expected: tx.domain_id.0,
                    actual: output.domain_id.0,
                });
            }
            inputs.push(output);
        }

        let mut reads = Vec::with_capacity(tx.read_set.len());
        for rr in &tx.read_set {
            let Some(output) = self.store.get_output(rr.output_id) else {
                return Err(ComputeError::ObjectNotFound(rr.output_id.0));
            };
            if output.domain_id != rr.domain_id {
                return Err(ComputeError::DomainMismatch {
                    expected: rr.domain_id.0,
                    actual: output.domain_id.0,
                });
            }
            if output.version != rr.expected_version {
                return Err(ComputeError::ReadVersionMismatch {
                    expected: rr.expected_version.0,
                    actual: output.version.0,
                });
            }
            reads.push(output);
        }

        self.validate_output_proposals(tx, &inputs)?;

        self.authorization.authorize(tx, &inputs, &reads)?;
        self.resources.check_resources(tx, &inputs, &reads)?;

        Ok(ValidationReport { inputs, reads })
    }

    fn validate_output_proposals(
        &self,
        tx: &ComputeTx,
        inputs: &[ObjectOutput],
    ) -> ComputeResult<()> {
        for proposal in &tx.output_proposals {
            if proposal.domain_id != tx.domain_id {
                return Err(ComputeError::DomainMismatch {
                    expected: tx.domain_id.0,
                    actual: proposal.domain_id.0,
                });
            }

            match proposal.predecessor {
                Some(pred_id) => {
                    let Some(parent) = inputs.iter().find(|o| o.output_id == pred_id) else {
                        return Err(ComputeError::InvalidPredecessor);
                    };
                    if parent.object_id != proposal.object_id {
                        return Err(ComputeError::InvalidPredecessor);
                    }
                    if proposal.version.0 != parent.version.0.saturating_add(1) {
                        return Err(ComputeError::InvalidVersionProgression);
                    }
                }
                None => {
                    if proposal.version.0 != 1 {
                        return Err(ComputeError::InvalidVersionProgression);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Minimal executor that only consumes inputs after validation.
pub struct BasicTxExecutor<
    S: ObjectStore,
    A: AuthorizationPolicy,
    R: ResourcePolicy,
    D: DomainRegistry,
> {
    /// Object store backend.
    pub store: S,
    /// Authorization policy.
    pub authorization: A,
    /// Resource policy.
    pub resources: R,
    /// Domain registry.
    pub domains: D,
}

impl<S: ObjectStore, A: AuthorizationPolicy, R: ResourcePolicy, D: DomainRegistry>
    BasicTxExecutor<S, A, R, D>
{
    /// Creates an executor.
    pub fn new(store: S, authorization: A, resources: R, domains: D) -> Self {
        Self {
            store,
            authorization,
            resources,
            domains,
        }
    }

    /// Executes by validating and marking all inputs spent.
    pub fn execute(&self, tx: &ComputeTx) -> ComputeResult<ValidationReport> {
        let validator = BasicTxValidator {
            store: &self.store,
            authorization: &self.authorization,
            resources: &self.resources,
            domains: &self.domains,
        };

        let report = validator.validate(tx)?;
        for id in &tx.input_set {
            self.store.mark_spent(*id)?;
        }

        for proposal in &tx.output_proposals {
            self.store.insert_output(object_from_proposal(proposal))?;
        }

        Ok(report)
    }
}

fn object_from_proposal(proposal: &OutputProposal) -> ObjectOutput {
    ObjectOutput {
        output_id: proposal.output_id,
        object_id: proposal.object_id,
        version: proposal.version,
        domain_id: proposal.domain_id,
        kind: proposal.kind,
        owner: proposal.owner.clone(),
        predecessor: proposal.predecessor,
        state: proposal.state.clone(),
        state_root: proposal.state_root,
        resources: proposal.resources.clone(),
        lock: proposal.lock.clone(),
        logic: proposal.logic.clone(),
        created_at: proposal.created_at,
        ttl: proposal.ttl,
        rent_reserve: proposal.rent_reserve,
        flags: proposal.flags,
        extensions: proposal.extensions.clone(),
        spent: false,
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::Signer as _;

    use crate::crypto::{Address, Hash, PrivateKey};

    use super::*;
    use crate::compute::{
        domain::{DomainConfig, InMemoryDomainRegistry},
        object::{ObjectKind, Ownership, Script},
        policy::{DefaultAuthorizationPolicy, NoopResourcePolicy},
        primitives::{DomainId, ObjectId, OutputId, TxId, Version},
        tx::{Command, ObjectReadRef, OutputProposal, TxSignature, TxWitness},
    };

    fn build_output(domain: DomainId, seed: u8) -> ObjectOutput {
        ObjectOutput {
            output_id: OutputId(Hash::from_bytes([seed; 32])),
            object_id: ObjectId(Hash::from_bytes([seed.wrapping_add(1); 32])),
            version: Version(1),
            domain_id: domain,
            kind: ObjectKind::Asset,
            owner: Ownership::Shared,
            predecessor: None,
            state: vec![seed],
            state_root: None,
            resources: vec![],
            lock: Script::default(),
            logic: None,
            created_at: 0,
            ttl: None,
            rent_reserve: None,
            flags: 0,
            extensions: vec![],
            spent: false,
        }
    }

    #[test]
    fn execute_marks_inputs_spent() {
        let store = InMemoryObjectStore::new();
        let input = build_output(DomainId(0), 7);
        store.insert_output(input.clone()).unwrap();

        let domains = InMemoryDomainRegistry::new();
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        let tx = ComputeTx {
            tx_id: TxId(Hash::from_bytes([9; 32])),
            domain_id: DomainId(0),
            command: Command::Transfer,
            input_set: vec![input.output_id],
            read_set: vec![],
            output_proposals: vec![OutputProposal {
                output_id: OutputId(Hash::from_bytes([10; 32])),
                object_id: input.object_id,
                domain_id: DomainId(0),
                kind: ObjectKind::Asset,
                owner: Ownership::Shared,
                predecessor: Some(input.output_id),
                version: Version(2),
                state: vec![42],
                state_root: None,
                resources: vec![],
                lock: Script::default(),
                logic: None,
                created_at: 0,
                ttl: None,
                rent_reserve: None,
                flags: 0,
                extensions: vec![],
            }],
            fee: 0,
            nonce: None,
            metadata: vec![],
            payload: vec![],
            deadline_unix_secs: None,
            chain_id: None,
            network_id: None,
            witness: TxWitness {
                signatures: vec![TxSignature::secp256k1(crate::crypto::Signature::new(
                    [1; 32], [2; 32], 27,
                ))],
                threshold: None,
            },
        }
        .with_expected_tx_id();

        let executor = BasicTxExecutor::new(
            store,
            DefaultAuthorizationPolicy,
            NoopResourcePolicy,
            domains,
        );
        let report = executor.execute(&tx).unwrap();
        assert_eq!(report.inputs.len(), 1);
        assert!(
            executor
                .store
                .get_output(input.output_id)
                .expect("input must exist")
                .spent
        );
        assert!(executor
            .store
            .get_output(OutputId(Hash::from_bytes([10; 32])))
            .is_some());
    }

    #[test]
    fn validate_rejects_read_version_mismatch() {
        let store = InMemoryObjectStore::new();
        let input = build_output(DomainId(0), 11);
        let read = build_output(DomainId(0), 12);
        store.insert_output(input.clone()).unwrap();
        store.insert_output(read.clone()).unwrap();

        let domains = InMemoryDomainRegistry::new();
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        let tx = ComputeTx {
            tx_id: TxId(Hash::from_bytes([91; 32])),
            domain_id: DomainId(0),
            command: Command::Invoke,
            input_set: vec![input.output_id],
            read_set: vec![ObjectReadRef {
                output_id: read.output_id,
                domain_id: DomainId(0),
                expected_version: Version(99),
            }],
            output_proposals: vec![OutputProposal {
                output_id: OutputId(Hash::from_bytes([92; 32])),
                object_id: input.object_id,
                domain_id: DomainId(0),
                kind: ObjectKind::State,
                owner: Ownership::Shared,
                predecessor: Some(input.output_id),
                version: Version(2),
                state: vec![1, 2, 3],
                state_root: None,
                resources: vec![],
                lock: Script::default(),
                logic: None,
                created_at: 0,
                ttl: None,
                rent_reserve: None,
                flags: 0,
                extensions: vec![],
            }],
            fee: 0,
            nonce: None,
            metadata: vec![],
            payload: vec![],
            deadline_unix_secs: None,
            chain_id: None,
            network_id: None,
            witness: TxWitness {
                signatures: vec![TxSignature::secp256k1(crate::crypto::Signature::new(
                    [1; 32], [2; 32], 27,
                ))],
                threshold: None,
            },
        }
        .with_expected_tx_id();

        let validator = BasicTxValidator {
            store: &store,
            authorization: &DefaultAuthorizationPolicy,
            resources: &NoopResourcePolicy,
            domains: &domains,
        };

        let err = validator
            .validate(&tx)
            .expect_err("must reject version mismatch");
        assert!(matches!(err, ComputeError::ReadVersionMismatch { .. }));
    }

    #[test]
    fn validate_rejects_unregistered_domain() {
        let store = InMemoryObjectStore::new();
        let input = build_output(DomainId(7), 21);
        store.insert_output(input.clone()).unwrap();

        let domains = InMemoryDomainRegistry::new();

        let tx = ComputeTx {
            tx_id: TxId(Hash::from_bytes([88; 32])),
            domain_id: DomainId(7),
            command: Command::Transfer,
            input_set: vec![input.output_id],
            read_set: vec![],
            output_proposals: vec![OutputProposal {
                output_id: OutputId(Hash::from_bytes([89; 32])),
                object_id: input.object_id,
                domain_id: DomainId(7),
                kind: ObjectKind::Asset,
                owner: Ownership::Shared,
                predecessor: Some(input.output_id),
                version: Version(2),
                state: vec![9],
                state_root: None,
                resources: vec![],
                lock: Script::default(),
                logic: None,
                created_at: 0,
                ttl: None,
                rent_reserve: None,
                flags: 0,
                extensions: vec![],
            }],
            fee: 0,
            nonce: None,
            metadata: vec![],
            payload: vec![],
            deadline_unix_secs: None,
            chain_id: None,
            network_id: None,
            witness: TxWitness {
                signatures: vec![TxSignature::secp256k1(crate::crypto::Signature::new(
                    [1; 32], [2; 32], 27,
                ))],
                threshold: None,
            },
        }
        .with_expected_tx_id();

        let validator = BasicTxValidator {
            store: &store,
            authorization: &DefaultAuthorizationPolicy,
            resources: &NoopResourcePolicy,
            domains: &domains,
        };
        let err = validator
            .validate(&tx)
            .expect_err("must reject unregistered domain");
        assert!(matches!(err, ComputeError::DomainNotRegistered(7)));
    }

    #[test]
    fn mint_allows_version_one_without_predecessor() {
        let store = InMemoryObjectStore::new();

        let domains = InMemoryDomainRegistry::new();
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        let tx = ComputeTx {
            tx_id: TxId(Hash::from_bytes([77; 32])),
            domain_id: DomainId(0),
            command: Command::Mint,
            input_set: vec![],
            read_set: vec![],
            output_proposals: vec![OutputProposal {
                output_id: OutputId(Hash::from_bytes([78; 32])),
                object_id: ObjectId(Hash::from_bytes([79; 32])),
                domain_id: DomainId(0),
                kind: ObjectKind::Asset,
                owner: Ownership::Shared,
                predecessor: None,
                version: Version(1),
                state: vec![0xAA],
                state_root: None,
                resources: vec![],
                lock: Script::default(),
                logic: None,
                created_at: 0,
                ttl: None,
                rent_reserve: None,
                flags: 0,
                extensions: vec![],
            }],
            fee: 0,
            nonce: None,
            metadata: vec![],
            payload: vec![],
            deadline_unix_secs: None,
            chain_id: None,
            network_id: None,
            witness: TxWitness {
                signatures: vec![TxSignature::secp256k1(crate::crypto::Signature::new(
                    [1; 32], [2; 32], 27,
                ))],
                threshold: None,
            },
        }
        .with_expected_tx_id();

        let executor = BasicTxExecutor::new(
            store,
            DefaultAuthorizationPolicy,
            NoopResourcePolicy,
            domains,
        );
        executor.execute(&tx).expect("mint should execute");
        assert!(executor
            .store
            .get_output(OutputId(Hash::from_bytes([78; 32])))
            .is_some());
    }

    #[test]
    fn validate_rejects_transfer_when_tx_id_mismatch_for_address_owner() {
        let store = InMemoryObjectStore::new();
        let owner_key = PrivateKey::from_bytes([7u8; 32]).expect("valid private key");
        let owner_addr = Address::from_public_key(&owner_key.public_key());

        let input = ObjectOutput {
            output_id: OutputId(Hash::from_bytes([31; 32])),
            object_id: ObjectId(Hash::from_bytes([32; 32])),
            version: Version(1),
            domain_id: DomainId(0),
            kind: ObjectKind::Asset,
            owner: Ownership::Address(owner_addr),
            predecessor: None,
            state: vec![1],
            state_root: None,
            resources: vec![],
            lock: Script::default(),
            logic: None,
            created_at: 0,
            ttl: None,
            rent_reserve: None,
            flags: 0,
            extensions: vec![],
            spent: false,
        };
        store.insert_output(input.clone()).expect("insert input");

        let domains = InMemoryDomainRegistry::new();
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        let mut tx = ComputeTx {
            tx_id: TxId(Hash::from_bytes([41; 32])),
            domain_id: DomainId(0),
            command: Command::Transfer,
            input_set: vec![input.output_id],
            read_set: vec![],
            output_proposals: vec![OutputProposal {
                output_id: OutputId(Hash::from_bytes([42; 32])),
                object_id: input.object_id,
                domain_id: DomainId(0),
                kind: ObjectKind::Asset,
                owner: Ownership::Address(owner_addr),
                predecessor: Some(input.output_id),
                version: Version(2),
                state: vec![2],
                state_root: None,
                resources: vec![],
                lock: Script::default(),
                logic: None,
                created_at: 0,
                ttl: None,
                rent_reserve: None,
                flags: 0,
                extensions: vec![],
            }],
            fee: 0,
            nonce: None,
            metadata: vec![],
            payload: vec![],
            deadline_unix_secs: None,
            chain_id: None,
            network_id: None,
            witness: TxWitness {
                signatures: vec![],
                threshold: Some(1),
            },
        };

        let sig = owner_key.sign(&tx.signing_preimage());
        tx.witness.signatures = vec![TxSignature::secp256k1(sig)];
        tx.tx_id = TxId(Hash::from_bytes([99; 32]));

        let validator = BasicTxValidator {
            store: &store,
            authorization: &DefaultAuthorizationPolicy,
            resources: &NoopResourcePolicy,
            domains: &domains,
        };

        let err = validator
            .validate(&tx)
            .expect_err("tx_id mismatch should be rejected");
        assert!(matches!(err, ComputeError::TxIdMismatch));
    }

    #[test]
    fn validate_accepts_transfer_when_owner_signature_and_tx_id_match() {
        let store = InMemoryObjectStore::new();
        let owner_key = PrivateKey::from_bytes([8u8; 32]).expect("valid private key");
        let owner_addr = Address::from_public_key(&owner_key.public_key());

        let input = ObjectOutput {
            output_id: OutputId(Hash::from_bytes([51; 32])),
            object_id: ObjectId(Hash::from_bytes([52; 32])),
            version: Version(1),
            domain_id: DomainId(0),
            kind: ObjectKind::Asset,
            owner: Ownership::Address(owner_addr),
            predecessor: None,
            state: vec![1],
            state_root: None,
            resources: vec![],
            lock: Script::default(),
            logic: None,
            created_at: 0,
            ttl: None,
            rent_reserve: None,
            flags: 0,
            extensions: vec![],
            spent: false,
        };
        store.insert_output(input.clone()).expect("insert input");

        let domains = InMemoryDomainRegistry::new();
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        let mut tx = ComputeTx {
            tx_id: TxId(Hash::from_bytes([61; 32])),
            domain_id: DomainId(0),
            command: Command::Transfer,
            input_set: vec![input.output_id],
            read_set: vec![],
            output_proposals: vec![OutputProposal {
                output_id: OutputId(Hash::from_bytes([62; 32])),
                object_id: input.object_id,
                domain_id: DomainId(0),
                kind: ObjectKind::Asset,
                owner: Ownership::Address(owner_addr),
                predecessor: Some(input.output_id),
                version: Version(2),
                state: vec![2],
                state_root: None,
                resources: vec![],
                lock: Script::default(),
                logic: None,
                created_at: 0,
                ttl: None,
                rent_reserve: None,
                flags: 0,
                extensions: vec![],
            }],
            fee: 0,
            nonce: None,
            metadata: vec![],
            payload: vec![1, 2, 3],
            deadline_unix_secs: Some(1_900_000_000),
            chain_id: Some(10086),
            network_id: Some(1),
            witness: TxWitness {
                signatures: vec![],
                threshold: Some(1),
            },
        };

        let sig = owner_key.sign(&tx.signing_preimage());
        tx.witness.signatures = vec![TxSignature::secp256k1(sig)];
        tx.assign_expected_tx_id();

        let validator = BasicTxValidator {
            store: &store,
            authorization: &DefaultAuthorizationPolicy,
            resources: &NoopResourcePolicy,
            domains: &domains,
        };

        let report = validator.validate(&tx).expect("valid transfer should pass");
        assert_eq!(report.inputs.len(), 1);
    }

    #[test]
    fn validate_accepts_transfer_when_native_ed25519_signature_matches() {
        let store = InMemoryObjectStore::new();

        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let verify_key = signing_key.verifying_key();
        let owner_pub = verify_key.to_bytes();

        let input = ObjectOutput {
            output_id: OutputId(Hash::from_bytes([71; 32])),
            object_id: ObjectId(Hash::from_bytes([72; 32])),
            version: Version(1),
            domain_id: DomainId(0),
            kind: ObjectKind::Asset,
            owner: Ownership::NativeEd25519(owner_pub),
            predecessor: None,
            state: vec![1],
            state_root: None,
            resources: vec![],
            lock: Script::default(),
            logic: None,
            created_at: 0,
            ttl: None,
            rent_reserve: None,
            flags: 0,
            extensions: vec![],
            spent: false,
        };
        store.insert_output(input.clone()).expect("insert input");

        let domains = InMemoryDomainRegistry::new();
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        let mut tx = ComputeTx {
            tx_id: TxId(Hash::from_bytes([81; 32])),
            domain_id: DomainId(0),
            command: Command::Transfer,
            input_set: vec![input.output_id],
            read_set: vec![],
            output_proposals: vec![OutputProposal {
                output_id: OutputId(Hash::from_bytes([82; 32])),
                object_id: input.object_id,
                domain_id: DomainId(0),
                kind: ObjectKind::Asset,
                owner: Ownership::NativeEd25519(owner_pub),
                predecessor: Some(input.output_id),
                version: Version(2),
                state: vec![2],
                state_root: None,
                resources: vec![],
                lock: Script::default(),
                logic: None,
                created_at: 0,
                ttl: None,
                rent_reserve: None,
                flags: 0,
                extensions: vec![],
            }],
            fee: 0,
            nonce: None,
            metadata: vec![],
            payload: vec![7, 7, 7],
            deadline_unix_secs: Some(1_900_000_001),
            chain_id: Some(10086),
            network_id: Some(1),
            witness: TxWitness {
                signatures: vec![],
                threshold: Some(1),
            },
        };

        let sig = signing_key.sign(&tx.signing_preimage()).to_bytes();
        tx.witness.signatures = vec![TxSignature::ed25519(sig, owner_pub)];
        tx.assign_expected_tx_id();

        let validator = BasicTxValidator {
            store: &store,
            authorization: &DefaultAuthorizationPolicy,
            resources: &NoopResourcePolicy,
            domains: &domains,
        };

        let report = validator
            .validate(&tx)
            .expect("valid native transfer should pass");
        assert_eq!(report.inputs.len(), 1);
    }
}
