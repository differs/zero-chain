//! UTXO Compute v1.1 core module.
//!
//! This module is the canonical L1 execution path for ZeroChain compute.
//! L1 consensus and state transitions are defined here.

pub mod agent;
pub mod domain;
pub mod error;
pub mod execution;
pub mod object;
pub mod policy;
pub mod primitives;
pub mod tx;

pub use agent::{AgentScheduler, AgentSpec, AgentTask, InMemoryAgentScheduler};
pub use domain::{DomainConfig, DomainRegistry, InMemoryDomainRegistry};
pub use error::ComputeError;
pub use execution::{
    BasicTxExecutor, BasicTxValidator, InMemoryObjectStore, ObjectStore, ValidationReport,
};
pub use object::{
    AssetId, ObjectKind, ObjectOutput, Ownership, ResourceMap, ResourceValue, Script,
};
pub use policy::{
    AuthorizationPolicy, DefaultAuthorizationPolicy, NoopResourcePolicy, ResourcePolicy,
};
pub use primitives::{DomainId, ObjectId, ObjectPointer, OutputId, ResourceId, TxId, Version};
pub use tx::{
    Command, ComputeTx, Metadata, ObjectReadRef, OutputProposal, SignatureScheme, TxSignature,
    TxWitness,
};
