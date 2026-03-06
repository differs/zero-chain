//! Object and output definitions for UTXO Compute v1.1.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::crypto::Address;

use super::primitives::{DomainId, ObjectId, OutputId, Version};

/// Object ownership model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ownership {
    /// Single externally controlled owner.
    Address(Address),
    /// Program/contract-controlled object.
    Program(Address),
    /// Shared object with no single owner authority.
    Shared,
}

/// Supported object categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectKind {
    /// Fungible value-bearing object.
    Asset,
    /// Contract/program code object.
    Code,
    /// Pure state/config object.
    State,
    /// Capability/tokenized permission object.
    Capability,
    /// Agent spec/state object.
    Agent,
    /// Cross-domain bridge object.
    Anchor,
    /// Commit-reveal proof object.
    Ticket,
}

/// Immutable versioned object output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectOutput {
    /// Physical immutable output identity.
    pub output_id: OutputId,
    /// Stable logical identity for all versions.
    pub object_id: ObjectId,
    /// Object version.
    pub version: Version,
    /// Owning domain.
    pub domain_id: DomainId,
    /// Category.
    pub kind: ObjectKind,
    /// Owner semantics.
    pub owner: Ownership,
    /// Optional predecessor output for lineage.
    pub predecessor: Option<OutputId>,
    /// Deterministic state payload.
    pub state: Vec<u8>,
    /// Optional executable payload (e.g. WASM blob id or code bytes).
    pub logic: Option<Vec<u8>>,
    /// Resource accounting tags/values.
    pub resources: BTreeMap<String, u128>,
    /// Whether output has been consumed.
    pub spent: bool,
}

impl ObjectOutput {
    /// Returns true if output is still spendable.
    pub fn is_live(&self) -> bool {
        !self.spent
    }
}
