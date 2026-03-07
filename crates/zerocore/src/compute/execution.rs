//! Transaction validation/execution skeleton for UTXO Compute v1.1.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::RwLock;

use super::{
    domain::DomainRegistry,
    error::{ComputeError, ComputeResult},
    object::{ObjectKind, ObjectOutput},
    policy::{AuthorizationPolicy, ResourcePolicy},
    primitives::{ObjectId, OutputId},
    tx::{Command, ComputeTx, OutputProposal},
};

const FLAG_FROZEN: u32 = 0x04;
const FLAG_AGENT: u32 = 0x08;
const FLAG_CHANNEL: u32 = 0x10;
const KNOWN_FLAGS_MASK: u32 = 0x1f;

const MAX_METADATA_ENTRIES: usize = 64;
const MAX_METADATA_KEY_BYTES: usize = 128;
const MAX_METADATA_VALUE_BYTES: usize = 4096;

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
        validate_tx_envelope(tx)?;

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
            validate_existing_output_lifecycle(&output, tx, "input")?;
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
            validate_existing_output_lifecycle(&output, tx, "read")?;
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
            validate_proposal_lifecycle(proposal, tx)?;

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
                    if proposal.created_at < parent.created_at {
                        return Err(ComputeError::InvalidTransaction(
                            "proposal created_at must be >= predecessor created_at".to_string(),
                        ));
                    }
                    if (parent.flags & FLAG_FROZEN) != 0 {
                        return Err(ComputeError::InvalidTransaction(
                            "frozen predecessor cannot be spent".to_string(),
                        ));
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

fn validate_tx_envelope(tx: &ComputeTx) -> ComputeResult<()> {
    if tx.nonce == Some(0) {
        return Err(ComputeError::InvalidTransaction(
            "nonce must be > 0 when present".to_string(),
        ));
    }

    if !matches!(tx.command, Command::Mint)
        && (tx.chain_id.is_some() || tx.network_id.is_some())
        && tx.nonce.is_none()
    {
        return Err(ComputeError::InvalidTransaction(
            "nonce is required for non-mint tx when replay fields are set".to_string(),
        ));
    }

    if matches!(tx.command, Command::Mint) && tx.fee != 0 {
        return Err(ComputeError::InvalidTransaction(
            "mint command cannot carry non-zero fee".to_string(),
        ));
    }

    validate_metadata(&tx.metadata)?;
    Ok(())
}

fn validate_metadata(metadata: &[(String, Vec<u8>)]) -> ComputeResult<()> {
    if metadata.len() > MAX_METADATA_ENTRIES {
        return Err(ComputeError::InvalidTransaction(format!(
            "metadata entries exceed limit: {} > {}",
            metadata.len(),
            MAX_METADATA_ENTRIES
        )));
    }

    let mut keys = HashSet::with_capacity(metadata.len());
    for (key, value) in metadata {
        if key.trim().is_empty() {
            return Err(ComputeError::InvalidTransaction(
                "metadata key must not be empty".to_string(),
            ));
        }
        if key.len() > MAX_METADATA_KEY_BYTES {
            return Err(ComputeError::InvalidTransaction(format!(
                "metadata key too long: {} > {}",
                key.len(),
                MAX_METADATA_KEY_BYTES
            )));
        }
        if value.len() > MAX_METADATA_VALUE_BYTES {
            return Err(ComputeError::InvalidTransaction(format!(
                "metadata value too large: {} > {}",
                value.len(),
                MAX_METADATA_VALUE_BYTES
            )));
        }
        if !keys.insert(key.clone()) {
            return Err(ComputeError::InvalidTransaction(format!(
                "duplicate metadata key: {key}"
            )));
        }
    }

    Ok(())
}

fn validate_existing_output_lifecycle(
    output: &ObjectOutput,
    tx: &ComputeTx,
    source: &str,
) -> ComputeResult<()> {
    validate_common_flags(output.kind, output.flags, source)?;
    if let Some(rent) = output.rent_reserve {
        if rent == 0 {
            return Err(ComputeError::InvalidTransaction(format!(
                "{source} rent_reserve must be > 0 when present"
            )));
        }
    }
    if let Some(ttl) = output.ttl {
        if ttl <= output.created_at {
            return Err(ComputeError::InvalidTransaction(format!(
                "{source} ttl must be > created_at"
            )));
        }
        let exec_ref = tx.deadline_unix_secs.ok_or_else(|| {
            ComputeError::InvalidTransaction(
                "deadline_unix_secs is required when ttl-protected objects are touched".to_string(),
            )
        })?;
        if ttl < exec_ref {
            return Err(ComputeError::InvalidTransaction(format!(
                "{source} object expired at {ttl}, tx reference time is {exec_ref}"
            )));
        }
    }
    Ok(())
}

fn validate_proposal_lifecycle(proposal: &OutputProposal, tx: &ComputeTx) -> ComputeResult<()> {
    validate_common_flags(proposal.kind, proposal.flags, "proposal")?;
    if let Some(rent) = proposal.rent_reserve {
        if rent == 0 {
            return Err(ComputeError::InvalidTransaction(
                "proposal rent_reserve must be > 0 when present".to_string(),
            ));
        }
    }
    if let Some(ttl) = proposal.ttl {
        if ttl <= proposal.created_at {
            return Err(ComputeError::InvalidTransaction(
                "proposal ttl must be > created_at".to_string(),
            ));
        }
        let exec_ref = tx.deadline_unix_secs.ok_or_else(|| {
            ComputeError::InvalidTransaction(
                "deadline_unix_secs is required when proposal ttl is set".to_string(),
            )
        })?;
        if ttl < exec_ref {
            return Err(ComputeError::InvalidTransaction(
                "proposal ttl is already expired relative to tx reference time".to_string(),
            ));
        }
        if proposal.rent_reserve.unwrap_or(0) == 0 {
            return Err(ComputeError::InvalidTransaction(
                "proposal with ttl must include non-zero rent_reserve".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_common_flags(kind: ObjectKind, flags: u32, source: &str) -> ComputeResult<()> {
    if (flags & !KNOWN_FLAGS_MASK) != 0 {
        return Err(ComputeError::InvalidTransaction(format!(
            "{source} contains unknown flag bits: 0x{flags:08x}"
        )));
    }
    if (flags & FLAG_AGENT) != 0 && kind != ObjectKind::Agent {
        return Err(ComputeError::InvalidTransaction(format!(
            "{source} with agent flag must be ObjectKind::Agent"
        )));
    }
    if (flags & FLAG_CHANNEL) != 0 && kind != ObjectKind::Ticket {
        return Err(ComputeError::InvalidTransaction(format!(
            "{source} with channel flag must be ObjectKind::Ticket"
        )));
    }
    Ok(())
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
        object::{ObjectKind, Ownership, ResourceValue, Script},
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
            nonce: Some(100),
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
            nonce: Some(101),
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

    #[test]
    fn validate_rejects_duplicate_metadata_keys() {
        let store = InMemoryObjectStore::new();
        let domains = InMemoryDomainRegistry::new();
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        let tx = ComputeTx {
            tx_id: TxId(Hash::from_bytes([111; 32])),
            domain_id: DomainId(0),
            command: Command::Mint,
            input_set: vec![],
            read_set: vec![],
            output_proposals: vec![OutputProposal {
                output_id: OutputId(Hash::from_bytes([112; 32])),
                object_id: ObjectId(Hash::from_bytes([113; 32])),
                domain_id: DomainId(0),
                kind: ObjectKind::State,
                owner: Ownership::Shared,
                predecessor: None,
                version: Version(1),
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
            }],
            fee: 0,
            nonce: None,
            metadata: vec![
                ("proof".to_string(), vec![1]),
                ("proof".to_string(), vec![2]),
            ],
            payload: vec![],
            deadline_unix_secs: None,
            chain_id: Some(10086),
            network_id: Some(1),
            witness: TxWitness {
                signatures: vec![TxSignature::secp256k1(crate::crypto::Signature::new(
                    [1; 32], [2; 32], 27,
                ))],
                threshold: Some(1),
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
            .expect_err("duplicate metadata keys must fail");
        assert!(matches!(err, ComputeError::InvalidTransaction(_)));
    }

    #[test]
    fn validate_rejects_transfer_without_nonce_with_replay_fields() {
        let store = InMemoryObjectStore::new();
        let input = build_output(DomainId(0), 120);
        store.insert_output(input.clone()).expect("insert input");

        let domains = InMemoryDomainRegistry::new();
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        let tx = ComputeTx {
            tx_id: TxId(Hash::from_bytes([121; 32])),
            domain_id: DomainId(0),
            command: Command::Transfer,
            input_set: vec![input.output_id],
            read_set: vec![],
            output_proposals: vec![OutputProposal {
                output_id: OutputId(Hash::from_bytes([122; 32])),
                object_id: input.object_id,
                domain_id: DomainId(0),
                kind: ObjectKind::Asset,
                owner: Ownership::Shared,
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
            chain_id: Some(10086),
            network_id: Some(1),
            witness: TxWitness {
                signatures: vec![TxSignature::secp256k1(crate::crypto::Signature::new(
                    [1; 32], [2; 32], 27,
                ))],
                threshold: Some(1),
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
            .expect_err("nonce must be required in replay-protected transfer");
        assert!(matches!(err, ComputeError::InvalidTransaction(_)));
    }

    #[test]
    fn validate_rejects_ttl_proposal_without_deadline() {
        let store = InMemoryObjectStore::new();
        let domains = InMemoryDomainRegistry::new();
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        let tx = ComputeTx {
            tx_id: TxId(Hash::from_bytes([131; 32])),
            domain_id: DomainId(0),
            command: Command::Mint,
            input_set: vec![],
            read_set: vec![],
            output_proposals: vec![OutputProposal {
                output_id: OutputId(Hash::from_bytes([132; 32])),
                object_id: ObjectId(Hash::from_bytes([133; 32])),
                domain_id: DomainId(0),
                kind: ObjectKind::State,
                owner: Ownership::Shared,
                predecessor: None,
                version: Version(1),
                state: vec![2],
                state_root: None,
                resources: vec![],
                lock: Script::default(),
                logic: None,
                created_at: 10,
                ttl: Some(20),
                rent_reserve: Some(1),
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
                threshold: Some(1),
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
            .expect_err("ttl proposal without deadline must fail");
        assert!(matches!(err, ComputeError::InvalidTransaction(_)));
    }

    #[test]
    fn validate_rejects_resource_inflation() {
        let store = InMemoryObjectStore::new();
        let mut input = build_output(DomainId(0), 140);
        input.resources = vec![(Hash::zero(), ResourceValue::Amount(10))];
        store.insert_output(input.clone()).expect("insert input");

        let domains = InMemoryDomainRegistry::new();
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        let tx = ComputeTx {
            tx_id: TxId(Hash::from_bytes([141; 32])),
            domain_id: DomainId(0),
            command: Command::Transfer,
            input_set: vec![input.output_id],
            read_set: vec![],
            output_proposals: vec![OutputProposal {
                output_id: OutputId(Hash::from_bytes([142; 32])),
                object_id: input.object_id,
                domain_id: DomainId(0),
                kind: ObjectKind::Asset,
                owner: Ownership::Shared,
                predecessor: Some(input.output_id),
                version: Version(2),
                state: vec![2],
                state_root: None,
                resources: vec![(Hash::zero(), ResourceValue::Amount(11))],
                lock: Script::default(),
                logic: None,
                created_at: 0,
                ttl: None,
                rent_reserve: None,
                flags: 0,
                extensions: vec![],
            }],
            fee: 0,
            nonce: Some(7),
            metadata: vec![],
            payload: vec![],
            deadline_unix_secs: None,
            chain_id: None,
            network_id: None,
            witness: TxWitness {
                signatures: vec![TxSignature::secp256k1(crate::crypto::Signature::new(
                    [1; 32], [2; 32], 27,
                ))],
                threshold: Some(1),
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
            .expect_err("resource inflation must fail");
        assert!(matches!(err, ComputeError::ResourcePolicyViolation));
    }

    #[test]
    fn validate_rejects_lock_script_when_witness_scheme_missing() {
        let store = InMemoryObjectStore::new();
        let mut input = build_output(DomainId(0), 150);
        input.lock = Script {
            vm: 1,
            code: b"REQUIRE_ED25519".to_vec(),
        };
        store.insert_output(input.clone()).expect("insert input");

        let domains = InMemoryDomainRegistry::new();
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        let tx = ComputeTx {
            tx_id: TxId(Hash::from_bytes([151; 32])),
            domain_id: DomainId(0),
            command: Command::Transfer,
            input_set: vec![input.output_id],
            read_set: vec![],
            output_proposals: vec![OutputProposal {
                output_id: OutputId(Hash::from_bytes([152; 32])),
                object_id: input.object_id,
                domain_id: DomainId(0),
                kind: ObjectKind::Asset,
                owner: Ownership::Shared,
                predecessor: Some(input.output_id),
                version: Version(2),
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
            }],
            fee: 0,
            nonce: Some(9),
            metadata: vec![],
            payload: vec![],
            deadline_unix_secs: None,
            chain_id: None,
            network_id: None,
            witness: TxWitness {
                signatures: vec![TxSignature::secp256k1(crate::crypto::Signature::new(
                    [1; 32], [2; 32], 27,
                ))],
                threshold: Some(1),
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
            .expect_err("missing required witness scheme must fail");
        assert!(matches!(err, ComputeError::AuthorizationDenied));
    }
}
