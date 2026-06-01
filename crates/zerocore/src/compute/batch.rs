//! Batched compute planning and execution.

use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use super::{
    domain::DomainRegistry,
    error::{ComputeError, ComputeResult},
    execution::{BasicTxExecutor, BasicTxValidator, ObjectStore, ValidationReport},
    policy::{AuthorizationPolicy, ResourcePolicy},
    primitives::{DomainId, ObjectId, OutputId, TxId},
    scheduler::{ComputeScheduleError, ComputeScheduler, PendingComputeTx},
    tx::ComputeTx,
};

const MAX_COMPLETED_OUTCOMES: usize = 50_000;

/// Failure fallback mode for batch execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeFallbackMode {
    /// Never fallback to serial execution.
    Disabled,
    /// Fallback to serial execution when batch admission or outcome lookup fails.
    SerialOnFailure,
}

impl Default for ComputeFallbackMode {
    fn default() -> Self {
        Self::SerialOnFailure
    }
}

impl ComputeFallbackMode {
    /// Builds the runtime policy object for this mode.
    pub fn build_policy(self) -> Arc<dyn ComputeFallbackPolicy> {
        match self {
            Self::Disabled => Arc::new(DisabledComputeFallbackPolicy),
            Self::SerialOnFailure => Arc::new(SerialComputeFallbackPolicy),
        }
    }
}

/// Fallback disposition for a compute execution failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputeFallbackDisposition {
    /// Reject the tx or batch.
    Reject,
    /// Execute the affected txs serially.
    RunSerial,
}

/// Fallback policy hook.
pub trait ComputeFallbackPolicy: Send + Sync {
    /// Fallback choice when scheduler admission fails.
    fn on_schedule_reject(
        &self,
        tx: &ComputeTx,
        err: &ComputeScheduleError,
    ) -> ComputeFallbackDisposition;
    /// Fallback choice when batch planning fails.
    fn on_plan_error(
        &self,
        pending: &[PendingComputeTx],
        err: &ComputeError,
    ) -> ComputeFallbackDisposition;
    /// Fallback choice when a submitted tx cannot be resolved in the outcome cache.
    fn on_missing_outcome(&self, tx: &ComputeTx) -> ComputeFallbackDisposition;
}

/// Strict fallback policy.
#[derive(Clone, Copy, Debug, Default)]
pub struct DisabledComputeFallbackPolicy;

impl ComputeFallbackPolicy for DisabledComputeFallbackPolicy {
    fn on_schedule_reject(
        &self,
        _tx: &ComputeTx,
        _err: &ComputeScheduleError,
    ) -> ComputeFallbackDisposition {
        ComputeFallbackDisposition::Reject
    }

    fn on_plan_error(
        &self,
        _pending: &[PendingComputeTx],
        _err: &ComputeError,
    ) -> ComputeFallbackDisposition {
        ComputeFallbackDisposition::Reject
    }

    fn on_missing_outcome(&self, _tx: &ComputeTx) -> ComputeFallbackDisposition {
        ComputeFallbackDisposition::Reject
    }
}

/// Serial fallback policy used by the default runtime mode.
#[derive(Clone, Copy, Debug, Default)]
pub struct SerialComputeFallbackPolicy;

impl ComputeFallbackPolicy for SerialComputeFallbackPolicy {
    fn on_schedule_reject(
        &self,
        _tx: &ComputeTx,
        _err: &ComputeScheduleError,
    ) -> ComputeFallbackDisposition {
        ComputeFallbackDisposition::RunSerial
    }

    fn on_plan_error(
        &self,
        _pending: &[PendingComputeTx],
        _err: &ComputeError,
    ) -> ComputeFallbackDisposition {
        ComputeFallbackDisposition::RunSerial
    }

    fn on_missing_outcome(&self, _tx: &ComputeTx) -> ComputeFallbackDisposition {
        ComputeFallbackDisposition::RunSerial
    }
}

/// Access-set summary used for conflict detection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComputeAccessSet {
    /// Domain executed by this tx.
    pub domain_id: DomainId,
    /// Outputs read by the tx.
    pub read_output_ids: BTreeSet<OutputId>,
    /// Logical objects read by the tx.
    pub read_object_ids: BTreeSet<ObjectId>,
    /// Outputs written or consumed by the tx.
    pub write_output_ids: BTreeSet<OutputId>,
    /// Logical objects written or consumed by the tx.
    pub write_object_ids: BTreeSet<ObjectId>,
}

impl Default for ComputeAccessSet {
    fn default() -> Self {
        Self {
            domain_id: DomainId(0),
            read_output_ids: BTreeSet::new(),
            read_object_ids: BTreeSet::new(),
            write_output_ids: BTreeSet::new(),
            write_object_ids: BTreeSet::new(),
        }
    }
}

/// Planned transaction with its resolved access set.
#[derive(Clone, Debug)]
pub struct PlannedComputeTx {
    /// Pending tx wrapper.
    pub pending: PendingComputeTx,
    /// Resolved access set.
    pub access: ComputeAccessSet,
}

/// Batch execution group.
#[derive(Clone, Debug)]
pub struct ComputeBatchGroup {
    /// Domain id shared by the group.
    pub domain_id: DomainId,
    /// Planned txs in execution order.
    pub txs: Vec<PlannedComputeTx>,
}

/// Batch plan emitted by the planner.
#[derive(Clone, Debug, Default)]
pub struct ComputeBatchPlan {
    /// Batchable groups.
    pub groups: Vec<ComputeBatchGroup>,
    /// Transactions that were not batchable and should be run one-by-one.
    pub fallback: Vec<PendingComputeTx>,
}

/// Per-tx execution outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComputeBatchOutcome {
    /// Transaction id.
    pub tx_id: TxId,
    /// Whether execution committed successfully.
    pub accepted: bool,
    /// Validation / execution report on success.
    pub report: Option<ValidationReport>,
    /// Error on failure.
    pub error: Option<ComputeError>,
}

/// Conflict policy used by the batch planner.
pub trait ComputeConflictPolicy: Send + Sync {
    /// Builds a resolved access set from the tx and current store state.
    fn access_set(
        &self,
        tx: &ComputeTx,
        store: &dyn ObjectStore,
    ) -> ComputeResult<ComputeAccessSet>;

    /// Returns true when two txs cannot be placed in the same batch group.
    fn conflicts(&self, left: &ComputeAccessSet, right: &ComputeAccessSet) -> bool;

    /// Whether this tx is eligible for batched execution.
    fn can_batch(&self, _tx: &ComputeTx) -> bool {
        true
    }
}

/// Default conservative conflict policy.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultComputeConflictPolicy;

impl DefaultComputeConflictPolicy {
    fn intersects<T: Ord>(left: &BTreeSet<T>, right: &BTreeSet<T>) -> bool {
        left.iter().any(|item| right.contains(item))
    }
}

impl ComputeConflictPolicy for DefaultComputeConflictPolicy {
    fn access_set(
        &self,
        tx: &ComputeTx,
        store: &dyn ObjectStore,
    ) -> ComputeResult<ComputeAccessSet> {
        let mut access = ComputeAccessSet {
            domain_id: tx.domain_id,
            ..Default::default()
        };

        for output_id in &tx.input_set {
            let Some(output) = store.get_output(*output_id) else {
                return Err(ComputeError::ObjectNotFound(output_id.0));
            };
            access.write_output_ids.insert(*output_id);
            access.write_object_ids.insert(output.object_id);
        }

        for read_ref in &tx.read_set {
            let Some(output) = store.get_output(read_ref.output_id) else {
                return Err(ComputeError::ObjectNotFound(read_ref.output_id.0));
            };
            access.read_output_ids.insert(read_ref.output_id);
            access.read_object_ids.insert(output.object_id);
        }

        for proposal in &tx.output_proposals {
            access.write_output_ids.insert(proposal.output_id);
            access.write_object_ids.insert(proposal.object_id);

            if let Some(predecessor) = proposal.predecessor {
                let Some(output) = store.get_output(predecessor) else {
                    return Err(ComputeError::ObjectNotFound(predecessor.0));
                };
                access.write_output_ids.insert(predecessor);
                access.write_object_ids.insert(output.object_id);
            }
        }

        Ok(access)
    }

    fn conflicts(&self, left: &ComputeAccessSet, right: &ComputeAccessSet) -> bool {
        Self::intersects(&left.write_output_ids, &right.write_output_ids)
            || Self::intersects(&left.write_output_ids, &right.read_output_ids)
            || Self::intersects(&left.read_output_ids, &right.write_output_ids)
            || Self::intersects(&left.write_object_ids, &right.write_object_ids)
            || Self::intersects(&left.write_object_ids, &right.read_object_ids)
            || Self::intersects(&left.read_object_ids, &right.write_object_ids)
    }
}

/// Planner turns a pending queue into executable groups.
pub trait ComputeBatchPlanner: Send + Sync {
    /// Plans a batch from pending transactions.
    fn plan(
        &self,
        pending: &[PendingComputeTx],
        store: &dyn ObjectStore,
    ) -> ComputeResult<ComputeBatchPlan>;
}

/// Default greedy planner.
pub struct DefaultComputeBatchPlanner<P> {
    conflict_policy: P,
}

impl<P> DefaultComputeBatchPlanner<P> {
    /// Creates a new planner.
    pub fn new(conflict_policy: P) -> Self {
        Self { conflict_policy }
    }
}

impl<P: ComputeConflictPolicy> ComputeBatchPlanner for DefaultComputeBatchPlanner<P> {
    fn plan(
        &self,
        pending: &[PendingComputeTx],
        store: &dyn ObjectStore,
    ) -> ComputeResult<ComputeBatchPlan> {
        let mut groups: Vec<ComputeBatchGroup> = Vec::new();
        let mut fallback: Vec<PendingComputeTx> = Vec::new();

        for item in pending.iter().cloned() {
            if !self.conflict_policy.can_batch(&item.tx) {
                fallback.push(item);
                continue;
            }

            let access = match self.conflict_policy.access_set(&item.tx, store) {
                Ok(access) => access,
                Err(_) => {
                    fallback.push(item);
                    continue;
                }
            };

            let planned = PlannedComputeTx {
                pending: item,
                access,
            };

            let mut placed = false;
            for group in &mut groups {
                if group.domain_id != planned.pending.domain_id {
                    continue;
                }
                if group.txs.iter().all(|existing| {
                    !self
                        .conflict_policy
                        .conflicts(&existing.access, &planned.access)
                }) {
                    group.txs.push(planned.clone());
                    placed = true;
                    break;
                }
            }

            if !placed {
                groups.push(ComputeBatchGroup {
                    domain_id: planned.pending.domain_id,
                    txs: vec![planned],
                });
            }
        }

        Ok(ComputeBatchPlan { groups, fallback })
    }
}

/// Batch runner interface.
pub trait ComputeBatchRunner: Send + Sync {
    /// Executes a whole plan.
    fn run_plan(&self, plan: ComputeBatchPlan) -> Vec<ComputeBatchOutcome>;
    /// Executes one group.
    fn run_group(&self, group: ComputeBatchGroup) -> Vec<ComputeBatchOutcome>;
    /// Executes one tx serially as a fallback path.
    fn run_serial(&self, pending: PendingComputeTx) -> ComputeBatchOutcome;
}

/// Parallel validator and serial committer.
pub struct ParallelComputeBatchRunner {
    store: Arc<dyn ObjectStore>,
    authorization: Arc<dyn AuthorizationPolicy>,
    resources: Arc<dyn ResourcePolicy>,
    domains: Arc<dyn DomainRegistry>,
}

impl ParallelComputeBatchRunner {
    /// Creates a runner over shared compute backends.
    pub fn new(
        store: Arc<dyn ObjectStore>,
        authorization: Arc<dyn AuthorizationPolicy>,
        resources: Arc<dyn ResourcePolicy>,
        domains: Arc<dyn DomainRegistry>,
    ) -> Self {
        Self {
            store,
            authorization,
            resources,
            domains,
        }
    }

    fn run_single(&self, tx: &ComputeTx) -> ComputeBatchOutcome {
        let executor = BasicTxExecutor::new(
            self.store.clone(),
            self.authorization.clone(),
            self.resources.clone(),
            self.domains.clone(),
        );

        match executor.execute(tx) {
            Ok(report) => ComputeBatchOutcome {
                tx_id: tx.tx_id,
                accepted: true,
                report: Some(report),
                error: None,
            },
            Err(err) => ComputeBatchOutcome {
                tx_id: tx.tx_id,
                accepted: false,
                report: None,
                error: Some(err),
            },
        }
    }
}

impl ComputeBatchRunner for ParallelComputeBatchRunner {
    fn run_plan(&self, plan: ComputeBatchPlan) -> Vec<ComputeBatchOutcome> {
        let mut outcomes = Vec::new();
        for group in plan.groups {
            outcomes.extend(self.run_group(group));
        }
        for pending in plan.fallback {
            outcomes.push(self.run_serial(pending));
        }
        outcomes
    }

    fn run_group(&self, group: ComputeBatchGroup) -> Vec<ComputeBatchOutcome> {
        if group.txs.is_empty() {
            return Vec::new();
        }

        if group.txs.len() == 1 {
            return vec![self.run_serial(group.txs[0].pending.clone())];
        }

        let group_len = group.txs.len();
        let results = Arc::new(parking_lot::Mutex::new(vec![None; group_len]));
        thread::scope(|scope| {
            for (idx, planned) in group.txs.into_iter().enumerate() {
                let results = Arc::clone(&results);
                let store = self.store.clone();
                let authorization = self.authorization.clone();
                let resources = self.resources.clone();
                let domains = self.domains.clone();

                scope.spawn(move || {
                    let validator = BasicTxValidator {
                        store: &store,
                        authorization: &authorization,
                        resources: &resources,
                        domains: &domains,
                    };

                    let outcome = match validator.validate(&planned.pending.tx) {
                        Ok(report) => {
                            let executor =
                                BasicTxExecutor::new(store, authorization, resources, domains);
                            match executor.commit_prevalidated(&planned.pending.tx, report) {
                                Ok(report) => ComputeBatchOutcome {
                                    tx_id: planned.pending.tx_id,
                                    accepted: true,
                                    report: Some(report),
                                    error: None,
                                },
                                Err(err) => ComputeBatchOutcome {
                                    tx_id: planned.pending.tx_id,
                                    accepted: false,
                                    report: None,
                                    error: Some(err),
                                },
                            }
                        }
                        Err(err) => ComputeBatchOutcome {
                            tx_id: planned.pending.tx_id,
                            accepted: false,
                            report: None,
                            error: Some(err),
                        },
                    };

                    results.lock()[idx] = Some(outcome);
                });
            }
        });

        let mut guard = results.lock();
        let mut outcomes = Vec::with_capacity(group_len);
        for outcome in guard.iter_mut() {
            outcomes.push(outcome.take().expect("batch outcome missing"));
        }
        outcomes
    }

    fn run_serial(&self, pending: PendingComputeTx) -> ComputeBatchOutcome {
        self.run_single(&pending.tx)
    }
}

/// Execution service combining scheduler, planner and runner.
pub struct ComputeExecutionService {
    store: Arc<dyn ObjectStore>,
    scheduler: Arc<dyn ComputeScheduler>,
    planner: Arc<dyn ComputeBatchPlanner>,
    runner: Arc<dyn ComputeBatchRunner>,
    fallback_policy: Arc<dyn ComputeFallbackPolicy>,
    completed: RwLock<HashMap<TxId, ComputeBatchOutcome>>,
    completed_order: RwLock<VecDeque<TxId>>,
}

impl ComputeExecutionService {
    /// Creates a service.
    pub fn new(
        store: Arc<dyn ObjectStore>,
        scheduler: Arc<dyn ComputeScheduler>,
        planner: Arc<dyn ComputeBatchPlanner>,
        runner: Arc<dyn ComputeBatchRunner>,
        fallback_policy: Arc<dyn ComputeFallbackPolicy>,
    ) -> Self {
        Self {
            store,
            scheduler,
            planner,
            runner,
            fallback_policy,
            completed: RwLock::new(HashMap::new()),
            completed_order: RwLock::new(VecDeque::new()),
        }
    }

    /// Enqueues a tx.
    pub fn submit(
        &self,
        tx: ComputeTx,
    ) -> Result<super::scheduler::ComputeScheduleTicket, super::scheduler::ComputeScheduleError>
    {
        self.scheduler.submit(tx)
    }

    /// Flushes ready batches through planner and runner.
    pub fn flush_ready(&self) -> ComputeResult<Vec<ComputeBatchOutcome>> {
        let pending = self.scheduler.drain_ready();
        if pending.is_empty() {
            return Ok(Vec::new());
        }

        let mut outcomes = Vec::new();
        let batch_size = self.scheduler.config().max_batch_size.max(1);

        for chunk in pending.chunks(batch_size) {
            let chunk_outcomes = match self.planner.plan(chunk, self.store.as_ref()) {
                Ok(plan) => self.runner.run_plan(plan),
                Err(err) => match self.fallback_policy.on_plan_error(chunk, &err) {
                    ComputeFallbackDisposition::Reject => return Err(err),
                    ComputeFallbackDisposition::RunSerial => chunk
                        .iter()
                        .cloned()
                        .map(|pending| self.runner.run_serial(pending))
                        .collect(),
                },
            };
            self.record_outcomes(&chunk_outcomes);
            outcomes.extend(chunk_outcomes);
        }

        Ok(outcomes)
    }

    /// Submit one tx, wait for the batch window, then return its outcome.
    pub async fn submit_and_run(&self, tx: ComputeTx) -> ComputeResult<ComputeBatchOutcome> {
        let tx_id = tx.tx_id;
        let lane_key = self.scheduler.config().lane_strategy.lane_key(&tx);
        let pending = PendingComputeTx::new(tx.clone(), lane_key, Instant::now());
        let ticket = match self.submit(tx.clone()) {
            Ok(ticket) => ticket,
            Err(err) => match self.fallback_policy.on_schedule_reject(&tx, &err) {
                ComputeFallbackDisposition::Reject => {
                    return Err(ComputeError::InvalidOperation(err.to_string()));
                }
                ComputeFallbackDisposition::RunSerial => {
                    let outcome = self.runner.run_serial(pending);
                    self.record_outcomes(std::slice::from_ref(&outcome));
                    return Ok(outcome);
                }
            },
        };

        let wait_ms = self.scheduler.config().batch_window_ms;
        if wait_ms > 0 {
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
        }

        let _ = self.flush_ready()?;
        if let Some(outcome) = self.completed.read().get(&tx_id).cloned() {
            return Ok(outcome);
        }

        match self.fallback_policy.on_missing_outcome(&tx) {
            ComputeFallbackDisposition::Reject => Err(ComputeError::InvalidOperation(format!(
                "compute outcome missing after submit for {}",
                hex::encode(ticket.tx_id.0.as_bytes())
            ))),
            ComputeFallbackDisposition::RunSerial => {
                let outcome = self.runner.run_serial(pending);
                self.record_outcomes(std::slice::from_ref(&outcome));
                Ok(outcome)
            }
        }
    }

    fn record_outcomes(&self, outcomes: &[ComputeBatchOutcome]) {
        let mut map = self.completed.write();
        let mut order = self.completed_order.write();
        for outcome in outcomes {
            order.retain(|tx_id| tx_id != &outcome.tx_id);
            order.push_back(outcome.tx_id);
            map.insert(outcome.tx_id, outcome.clone());
        }
        while order.len() > MAX_COMPLETED_OUTCOMES {
            if let Some(tx_id) = order.pop_front() {
                map.remove(&tx_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute::{
        domain::{DomainConfig, InMemoryDomainRegistry},
        object::{ObjectKind, Ownership, Script},
        primitives::{ObjectId, OutputId, Version},
        tx::{Command, OutputProposal, TxSignature, TxWitness},
        InMemoryObjectStore,
    };

    fn build_output(
        domain_id: DomainId,
        object_seed: u8,
        output_seed: u8,
    ) -> crate::compute::ObjectOutput {
        crate::compute::ObjectOutput {
            output_id: OutputId(crate::crypto::Hash::from_bytes([output_seed; 32])),
            object_id: ObjectId(crate::crypto::Hash::from_bytes([object_seed; 32])),
            version: Version(1),
            domain_id,
            kind: ObjectKind::Asset,
            owner: Ownership::Shared,
            predecessor: None,
            state: vec![output_seed],
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

    fn build_tx(
        tx_seed: u8,
        domain_id: DomainId,
        input: OutputId,
        input_object: ObjectId,
        output_seed: u8,
    ) -> ComputeTx {
        ComputeTx {
            tx_id: crate::TxId(crate::crypto::Hash::from_bytes([tx_seed; 32])),
            domain_id,
            command: Command::Transfer,
            input_set: vec![input],
            read_set: vec![],
            output_proposals: vec![OutputProposal {
                output_id: OutputId(crate::crypto::Hash::from_bytes([output_seed; 32])),
                object_id: input_object,
                domain_id,
                kind: ObjectKind::Asset,
                owner: Ownership::Shared,
                predecessor: Some(input),
                version: Version(2),
                state: vec![output_seed],
                state_root: None,
                resources: vec![],
                lock: Script::default(),
                logic: None,
                created_at: 1,
                ttl: None,
                rent_reserve: None,
                flags: 0,
                extensions: vec![],
            }],
            fee: 0,
            nonce: Some(1),
            metadata: vec![],
            payload: vec![],
            deadline_unix_secs: None,
            chain_id: None,
            network_id: None,
            witness: TxWitness {
                signatures: vec![TxSignature::ed25519([1; 64], [2; 32])],
                threshold: None,
            },
        }
    }

    fn build_store() -> (
        Arc<InMemoryObjectStore>,
        Arc<InMemoryDomainRegistry>,
        ComputeAccessSet,
    ) {
        let store = Arc::new(InMemoryObjectStore::new());
        let domains = Arc::new(InMemoryDomainRegistry::new());
        domains.upsert_domain(DomainConfig {
            domain_id: DomainId(0),
            name: "main".to_string(),
            vm: "wasm".to_string(),
            public: true,
        });

        let out = build_output(DomainId(0), 9, 1);
        store.insert_output(out.clone()).unwrap();

        let mut access = ComputeAccessSet::default();
        access.domain_id = DomainId(0);
        access.write_output_ids.insert(out.output_id);
        access.write_object_ids.insert(out.object_id);

        (store, domains, access)
    }

    #[test]
    fn conflict_policy_flags_same_object_as_conflict() {
        let (store, _domains, _access) = build_store();
        let policy = DefaultComputeConflictPolicy;
        let tx_a = build_tx(
            1,
            DomainId(0),
            OutputId(crate::crypto::Hash::from_bytes([1; 32])),
            ObjectId(crate::crypto::Hash::from_bytes([9; 32])),
            2,
        );
        let tx_b = build_tx(
            2,
            DomainId(0),
            OutputId(crate::crypto::Hash::from_bytes([1; 32])),
            ObjectId(crate::crypto::Hash::from_bytes([9; 32])),
            3,
        );

        let access_a = policy.access_set(&tx_a, store.as_ref()).unwrap();
        let access_b = policy.access_set(&tx_b, store.as_ref()).unwrap();
        assert!(policy.conflicts(&access_a, &access_b));
    }

    #[test]
    fn fallback_policy_distinguishes_reject_and_serial() {
        let tx = build_tx(
            3,
            DomainId(0),
            OutputId(crate::crypto::Hash::from_bytes([4; 32])),
            ObjectId(crate::crypto::Hash::from_bytes([5; 32])),
            6,
        );
        let err = ComputeScheduleError::QueueFull;

        assert_eq!(
            DisabledComputeFallbackPolicy.on_schedule_reject(&tx, &err),
            ComputeFallbackDisposition::Reject
        );
        assert_eq!(
            SerialComputeFallbackPolicy.on_schedule_reject(&tx, &err),
            ComputeFallbackDisposition::RunSerial
        );
    }
}
