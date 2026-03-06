//! UTXO Compute v1.1 core module.
//!
//! This module introduces an object-centric execution path that can coexist
//! with the legacy account/EVM path during migration.

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
pub use object::{ObjectKind, ObjectOutput, Ownership};
pub use policy::{
    AuthorizationPolicy, DefaultAuthorizationPolicy, NoopResourcePolicy, ResourcePolicy,
};
pub use primitives::{DomainId, ObjectId, ObjectPointer, OutputId, ResourceId, TxId, Version};
pub use tx::{
    Command, ComputeTx, ObjectReadRef, OutputProposal, SignatureScheme, TxSignature, TxWitness,
};
