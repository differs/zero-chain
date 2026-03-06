//! Transaction model for UTXO Compute v1.1.

use serde::{Deserialize, Serialize};

use crate::crypto::{keccak256, Hash, Signature};

use super::{
    object::{ObjectKind, Ownership},
    primitives::{DomainId, ObjectId, OutputId, TxId, Version},
};

/// Read-set reference with expected version hash binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectReadRef {
    /// Referenced output id.
    pub output_id: OutputId,
    /// Domain where referenced output resides.
    pub domain_id: DomainId,
    /// Expected object version for optimistic read validation.
    pub expected_version: Version,
}

/// Proposed output to be materialized on successful execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputProposal {
    /// Physical output id to create.
    pub output_id: OutputId,
    /// Logical object id.
    pub object_id: ObjectId,
    /// Domain of the new output.
    pub domain_id: DomainId,
    /// Kind for the new output.
    pub kind: ObjectKind,
    /// Ownership model.
    pub owner: Ownership,
    /// Optional predecessor output id (required for update semantics).
    pub predecessor: Option<OutputId>,
    /// Target version.
    pub version: Version,
    /// Deterministic state blob.
    pub state: Vec<u8>,
    /// Optional executable logic payload.
    pub logic: Option<Vec<u8>>,
}

/// Transaction command type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Transfer ownership/value among object outputs.
    Transfer,
    /// Invoke program/logic against object state.
    Invoke,
    /// Mint new logical object and initial version.
    Mint,
    /// Burn existing object output.
    Burn,
    /// Create cross-domain anchor commitment.
    Anchor,
    /// Reveal ticket data for anchor finalization.
    Reveal,
    /// Execute scheduled agent step.
    AgentTick,
}

/// Witness/signature envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxWitness {
    /// Signatures proving authorization.
    pub signatures: Vec<Signature>,
    /// Optional minimal signatures required for authorization.
    pub threshold: Option<u16>,
}

/// UTXO Compute transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeTx {
    /// Deterministic tx id.
    pub tx_id: TxId,
    /// Target domain for execution.
    pub domain_id: DomainId,
    /// Command to execute.
    pub command: Command,
    /// Inputs consumed by this transaction.
    pub input_set: Vec<OutputId>,
    /// Explicit read set.
    pub read_set: Vec<ObjectReadRef>,
    /// Output proposals to be committed atomically.
    pub output_proposals: Vec<OutputProposal>,
    /// User payload / ABI-encoded command args.
    pub payload: Vec<u8>,
    /// Optional absolute expiration timestamp.
    pub deadline_unix_secs: Option<u64>,
    /// Optional chain identifier used for anti-replay across chains.
    pub chain_id: Option<u64>,
    /// Optional network identifier used for anti-replay across environments.
    pub network_id: Option<u32>,
    /// Authorization witness.
    pub witness: TxWitness,
}

impl ComputeTx {
    /// Checks minimal structural validity.
    pub fn basic_sanity_check(&self) -> bool {
        let needs_inputs = matches!(
            self.command,
            Command::Transfer | Command::Invoke | Command::Burn
        );
        let needs_outputs = !matches!(self.command, Command::Burn);

        if needs_inputs && self.input_set.is_empty() {
            return false;
        }

        if needs_outputs && self.output_proposals.is_empty() {
            return false;
        }

        true
    }

    /// Deterministic signing preimage for authorization witnesses.
    ///
    /// This payload intentionally excludes witness signatures to avoid circular
    /// dependency and includes all semantically relevant fields to prevent
    /// cross-context replay and partial-tx substitution.
    pub fn signing_preimage(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(512);
        out.extend_from_slice(b"ZEROCHAIN-COMPUTE-SIGNING-V1");

        out.extend_from_slice(&self.domain_id.0.to_be_bytes());
        out.push(command_tag(self.command));

        encode_output_ids(&mut out, &self.input_set);
        encode_read_set(&mut out, &self.read_set);
        encode_output_proposals(&mut out, &self.output_proposals);

        encode_bytes(&mut out, &self.payload);

        match self.deadline_unix_secs {
            Some(deadline) => {
                out.push(1);
                out.extend_from_slice(&deadline.to_be_bytes());
            }
            None => out.push(0),
        }

        match self.chain_id {
            Some(chain_id) => {
                out.push(1);
                out.extend_from_slice(&chain_id.to_be_bytes());
            }
            None => out.push(0),
        }

        match self.network_id {
            Some(network_id) => {
                out.push(1);
                out.extend_from_slice(&network_id.to_be_bytes());
            }
            None => out.push(0),
        }

        out.extend_from_slice(&(self.witness.threshold.unwrap_or(1)).to_be_bytes());
        out
    }

    /// Keccak256 digest used for signature recovery/verification.
    pub fn signing_digest(&self) -> [u8; 32] {
        keccak256(&self.signing_preimage())
    }

    /// Canonical transaction id expected from the current transaction body.
    pub fn expected_tx_id(&self) -> TxId {
        TxId(Hash::from_bytes(self.signing_digest()))
    }

    /// Returns true when `tx_id` matches canonical hash of signed body.
    pub fn has_consistent_tx_id(&self) -> bool {
        self.tx_id == self.expected_tx_id()
    }

    /// Mutates transaction id to the canonical expected value.
    pub fn assign_expected_tx_id(&mut self) {
        self.tx_id = self.expected_tx_id();
    }

    /// Returns a copy with canonical transaction id assigned.
    pub fn with_expected_tx_id(mut self) -> Self {
        self.assign_expected_tx_id();
        self
    }
}

fn command_tag(command: Command) -> u8 {
    match command {
        Command::Transfer => 1,
        Command::Invoke => 2,
        Command::Mint => 3,
        Command::Burn => 4,
        Command::Anchor => 5,
        Command::Reveal => 6,
        Command::AgentTick => 7,
    }
}

fn object_kind_tag(kind: ObjectKind) -> u8 {
    match kind {
        ObjectKind::Asset => 1,
        ObjectKind::Code => 2,
        ObjectKind::State => 3,
        ObjectKind::Capability => 4,
        ObjectKind::Agent => 5,
        ObjectKind::Anchor => 6,
        ObjectKind::Ticket => 7,
    }
}

fn encode_len(out: &mut Vec<u8>, len: usize) {
    out.extend_from_slice(&(len as u32).to_be_bytes());
}

fn encode_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    encode_len(out, bytes.len());
    out.extend_from_slice(bytes);
}

fn encode_output_ids(out: &mut Vec<u8>, ids: &[OutputId]) {
    encode_len(out, ids.len());
    for id in ids {
        out.extend_from_slice(id.0.as_bytes());
    }
}

fn encode_read_set(out: &mut Vec<u8>, reads: &[ObjectReadRef]) {
    encode_len(out, reads.len());
    for rr in reads {
        out.extend_from_slice(rr.output_id.0.as_bytes());
        out.extend_from_slice(&rr.domain_id.0.to_be_bytes());
        out.extend_from_slice(&rr.expected_version.0.to_be_bytes());
    }
}

fn encode_output_proposals(out: &mut Vec<u8>, proposals: &[OutputProposal]) {
    encode_len(out, proposals.len());
    for p in proposals {
        out.extend_from_slice(p.output_id.0.as_bytes());
        out.extend_from_slice(p.object_id.0.as_bytes());
        out.extend_from_slice(&p.domain_id.0.to_be_bytes());
        out.push(object_kind_tag(p.kind));
        encode_ownership(out, &p.owner);

        match p.predecessor {
            Some(pred) => {
                out.push(1);
                out.extend_from_slice(pred.0.as_bytes());
            }
            None => out.push(0),
        }

        out.extend_from_slice(&p.version.0.to_be_bytes());
        encode_bytes(out, &p.state);

        match &p.logic {
            Some(logic) => {
                out.push(1);
                encode_bytes(out, logic);
            }
            None => out.push(0),
        }
    }
}

fn encode_ownership(out: &mut Vec<u8>, owner: &Ownership) {
    match owner {
        Ownership::Address(addr) => {
            out.push(1);
            out.extend_from_slice(addr.as_bytes());
        }
        Ownership::Program(addr) => {
            out.push(2);
            out.extend_from_slice(addr.as_bytes());
        }
        Ownership::Shared => {
            out.push(3);
        }
    }
}
