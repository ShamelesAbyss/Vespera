//! In-process task leases for approved external workers.
//!
//! The host authenticates callers before passing their worker identity here.
//! Calls that approve, revoke, submit or cancel belong to the operator.

mod events;
pub mod http;
mod storage;

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use events::{EventFeeds, Transition};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use vespera_core::{
    AgentId, BrokerEventCursor, BrokerEventPage, Capability, ExternalAttempt, ExternalTaskChange,
    Task, TaskId, TaskState,
};

const MAX_WORKERS: usize = 64;
const MAX_TASKS: usize = 256;
const MAX_TEXT: usize = 64 * 1024;
const MAX_CAPABILITIES: usize = 32;
const LEASE_TTL: Duration = Duration::from_secs(30);
const MAX_LIFETIME: Duration = Duration::from_secs(3600);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BrokerError {
    #[error("invalid external task input: {0}")]
    Invalid(&'static str),
    #[error("external task broker capacity reached")]
    Capacity,
    #[error("worker authority denied")]
    Unauthorized,
    #[error("worker already holds an active lease")]
    Busy,
    #[error("task or worker identity already exists")]
    Duplicate,
    #[error("task was not found")]
    Missing,
    #[error("lease is stale or does not belong to the worker")]
    StaleLease,
    #[error("result conflicts with the accepted result")]
    ResultConflict,
    #[error("event cursor belongs to a different feed")]
    ForeignCursor,
    #[error("event cursor is older than retained history")]
    ExpiredCursor,
}

/// Attempt fence. A caller must also present its authenticated worker identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fence {
    pub task_id: TaskId,
    pub attempt_id: Uuid,
    pub generation: u32,
}

#[derive(Clone, Serialize)]
pub struct Lease {
    pub task: Task,
    pub fence: Fence,
    pub expires_in_ms: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedResult {
    pub worker: AgentId,
    pub fence: Fence,
    pub content: String,
}

struct Worker {
    capabilities: BTreeSet<Capability>,
    enabled: bool,
}

struct ActiveLease {
    worker: AgentId,
    fence: Fence,
    expires: Instant,
}

enum Phase {
    Queued,
    Leased(ActiveLease),
    Completed(AcceptedResult),
    Failed,
    Cancelled,
}

struct Record {
    task: Task,
    eligible: BTreeSet<AgentId>,
    deadline: Instant,
    deadline_unix_ms: i64,
    max_attempts: u32,
    generation: u32,
    phase: Phase,
    dirty: bool,
}

impl Record {
    fn attempt(&self) -> Option<ExternalAttempt> {
        match &self.phase {
            Phase::Leased(active) => Some(ExternalAttempt {
                worker_id: active.worker,
                attempt_id: active.fence.attempt_id,
                generation: active.fence.generation,
            }),
            _ => None,
        }
    }

    fn transition(
        &mut self,
        change: ExternalTaskChange,
        attempt: Option<ExternalAttempt>,
    ) -> Transition {
        self.dirty = true;
        Transition {
            task_id: self.task.id,
            change,
            state: self.state(),
            attempt,
        }
    }

    fn retry_or_fail(&mut self, now: Instant) {
        self.phase = if now < self.deadline && self.generation < self.max_attempts {
            Phase::Queued
        } else {
            Phase::Failed
        };
    }

    fn expire(&mut self, now: Instant) -> Option<Transition> {
        let attempt = self.attempt();
        let change = match &self.phase {
            Phase::Queued if now >= self.deadline => {
                self.phase = Phase::Failed;
                ExternalTaskChange::DeadlineExceeded
            }
            Phase::Leased(active) if now >= active.expires => {
                self.retry_or_fail(now);
                ExternalTaskChange::LeaseExpired
            }
            _ => return None,
        };
        Some(self.transition(change, attempt))
    }

    fn state(&self) -> TaskState {
        match self.phase {
            Phase::Queued => TaskState::Queued,
            Phase::Leased(_) => TaskState::Running,
            Phase::Completed(_) => TaskState::Completed,
            Phase::Failed => TaskState::Failed,
            Phase::Cancelled => TaskState::Cancelled,
        }
    }
}

/// Serialized state for one host process. Terminal results occupy bounded slots
/// for the broker lifetime so retries can be reconciled by task and attempt.
#[derive(Default)]
pub struct LeaseBroker {
    workers: BTreeMap<AgentId, Worker>,
    tasks: Vec<Record>,
    events: EventFeeds,
}

impl LeaseBroker {
    /// Operator approval binds an identity to its capability allowlist.
    pub fn approve_worker(
        &mut self,
        id: AgentId,
        capabilities: BTreeSet<Capability>,
    ) -> Result<(), BrokerError> {
        id.validate()
            .map_err(|_| BrokerError::Invalid("worker ID"))?;
        if capabilities.len() > MAX_CAPABILITIES {
            return Err(BrokerError::Invalid("worker capabilities"));
        }
        if self.workers.contains_key(&id) {
            return Err(BrokerError::Duplicate);
        }
        if self.workers.len() >= MAX_WORKERS {
            return Err(BrokerError::Capacity);
        }
        self.workers.insert(
            id,
            Worker {
                capabilities,
                enabled: true,
            },
        );
        self.events.approve(id);
        Ok(())
    }

    /// Revoke future claims and immediately fence the worker's active attempt.
    pub fn revoke_worker(&mut self, id: AgentId) -> Result<(), BrokerError> {
        let worker = self.workers.get_mut(&id).ok_or(BrokerError::Unauthorized)?;
        worker.enabled = false;
        let now = Instant::now();
        self.reconcile(now);
        self.events.revoke(id);
        for record in &mut self.tasks {
            if matches!(&record.phase, Phase::Leased(active) if active.worker == id) {
                let attempt = record.attempt();
                record.retry_or_fail(now);
                self.events
                    .publish(record.transition(ExternalTaskChange::WorkerRevoked, attempt));
            }
        }
        Ok(())
    }

    /// Submit one queued task with an explicit worker allowlist and retry budget.
    pub fn submit(
        &mut self,
        task: Task,
        eligible: BTreeSet<AgentId>,
        lifetime: Duration,
        max_attempts: u32,
    ) -> Result<(), BrokerError> {
        task.validate().map_err(|_| BrokerError::Invalid("task"))?;
        if task.state != TaskState::Queued
            || task.prompt.len() > MAX_TEXT
            || task.required_capabilities.len() > MAX_CAPABILITIES
            || lifetime.is_zero()
            || lifetime > MAX_LIFETIME
            || !(1..=8).contains(&max_attempts)
        {
            return Err(BrokerError::Invalid("task bounds"));
        }
        if eligible.is_empty() || eligible.len() > MAX_WORKERS {
            return Err(BrokerError::Invalid("worker allowlist"));
        }
        for id in &eligible {
            self.worker(*id)?;
        }
        if self.tasks.iter().any(|record| record.task.id == task.id) {
            return Err(BrokerError::Duplicate);
        }
        if self.tasks.len() >= MAX_TASKS {
            return Err(BrokerError::Capacity);
        }
        let mut record = Record {
            task,
            eligible,
            deadline: Instant::now() + lifetime,
            deadline_unix_ms: storage::unix_ms()
                .and_then(|now| now.checked_add(lifetime.as_millis() as i64))
                .unwrap_or(0),
            max_attempts,
            generation: 0,
            phase: Phase::Queued,
            dirty: false,
        };
        self.events
            .publish(record.transition(ExternalTaskChange::Submitted, None));
        self.tasks.push(record);
        Ok(())
    }

    /// Claim the earliest submitted eligible task for this authenticated worker.
    pub fn claim(&mut self, worker: AgentId) -> Result<Option<Lease>, BrokerError> {
        self.claim_at(worker, Instant::now())
    }

    fn claim_at(&mut self, worker: AgentId, now: Instant) -> Result<Option<Lease>, BrokerError> {
        self.worker(worker)?;
        self.reconcile(now);
        let capabilities = &self
            .workers
            .get(&worker)
            .expect("approved worker")
            .capabilities;
        if self
            .tasks
            .iter()
            .any(|record| matches!(&record.phase, Phase::Leased(active) if active.worker == worker))
        {
            return Err(BrokerError::Busy);
        }
        let Some(record) = self.tasks.iter_mut().find(|record| {
            matches!(record.phase, Phase::Queued)
                && record.eligible.contains(&worker)
                && record.task.required_capabilities.is_subset(capabilities)
        }) else {
            return Ok(None);
        };
        record.generation += 1;
        let fence = Fence {
            task_id: record.task.id,
            attempt_id: Uuid::new_v4(),
            generation: record.generation,
        };
        let expires = (now + LEASE_TTL).min(record.deadline);
        record.phase = Phase::Leased(ActiveLease {
            worker,
            fence,
            expires,
        });
        self.events
            .publish(record.transition(ExternalTaskChange::Claimed, record.attempt()));
        let mut task = record.task.clone();
        task.state = TaskState::Running;
        Ok(Some(Lease {
            task,
            fence,
            expires_in_ms: remaining_ms(expires, now),
        }))
    }

    /// Renew a still-active lease without extending the task deadline.
    pub fn renew(&mut self, worker: AgentId, fence: Fence) -> Result<u64, BrokerError> {
        self.worker(worker)?;
        let now = Instant::now();
        self.reconcile(now);
        let record = self
            .record(fence.task_id)
            .map_err(|_| BrokerError::StaleLease)?;
        match &mut record.phase {
            Phase::Leased(active) if active.worker == worker && active.fence == fence => {
                active.expires = (now + LEASE_TTL).min(record.deadline);
                Ok(remaining_ms(active.expires, now))
            }
            _ => Err(BrokerError::StaleLease),
        }
    }

    /// Accept one result or return the existing result on an exact retry.
    pub fn complete(
        &mut self,
        worker: AgentId,
        fence: Fence,
        content: String,
    ) -> Result<AcceptedResult, BrokerError> {
        self.worker(worker)?;
        if content.len() > MAX_TEXT || content.trim().is_empty() {
            return Err(BrokerError::Invalid("result content"));
        }
        let now = Instant::now();
        self.reconcile(now);
        let record = self
            .record(fence.task_id)
            .map_err(|_| BrokerError::StaleLease)?;
        let attempt = record.attempt();
        match &record.phase {
            Phase::Completed(result) if result.worker == worker && result.fence == fence => {
                if result.content == content {
                    Ok(result.clone())
                } else {
                    Err(BrokerError::ResultConflict)
                }
            }
            Phase::Leased(active) if active.worker == worker && active.fence == fence => {
                let result = AcceptedResult {
                    worker,
                    fence,
                    content,
                };
                record.phase = Phase::Completed(result.clone());
                let event = record.transition(ExternalTaskChange::Completed, attempt);
                self.events.publish(event);
                Ok(result)
            }
            _ => Err(BrokerError::StaleLease),
        }
    }

    /// Operator cancellation returns the existing outcome if already terminal.
    pub fn cancel(&mut self, task: TaskId) -> Result<TaskState, BrokerError> {
        self.reconcile(Instant::now());
        let record = self.record(task)?;
        if matches!(record.phase, Phase::Queued | Phase::Leased(_)) {
            let attempt = record.attempt();
            record.phase = Phase::Cancelled;
            let event = record.transition(ExternalTaskChange::Cancelled, attempt);
            self.events.publish(event);
            return Ok(TaskState::Cancelled);
        }
        Ok(record.state())
    }

    /// Operator inspection reconciles elapsed expiry before returning the state.
    pub fn state(&mut self, task: TaskId) -> Result<TaskState, BrokerError> {
        self.reconcile(Instant::now());
        let record = self.record(task)?;
        Ok(record.state())
    }

    /// Operator access to an accepted result and its attempt provenance.
    pub fn result(&self, task: TaskId) -> Result<Option<&AcceptedResult>, BrokerError> {
        let record = self
            .tasks
            .iter()
            .find(|record| record.task.id == task)
            .ok_or(BrokerError::Missing)?;
        Ok(match &record.phase {
            Phase::Completed(result) => Some(result),
            _ => None,
        })
    }

    /// Read one authenticated worker's attempt history, or the operator feed.
    pub fn replay(
        &mut self,
        worker: Option<AgentId>,
        after: Option<BrokerEventCursor>,
        limit: u32,
    ) -> Result<BrokerEventPage, BrokerError> {
        if let Some(worker) = worker {
            self.worker(worker)?;
        }
        self.reconcile(Instant::now());
        self.events.replay(worker, after, limit)
    }

    fn replay_task_audit(
        &mut self,
        after: Option<BrokerEventCursor>,
        limit: u32,
    ) -> Result<BrokerEventPage, BrokerError> {
        self.reconcile(Instant::now());
        self.events.replay_task_audit(after, limit)
    }

    fn reconcile(&mut self, now: Instant) {
        for record in &mut self.tasks {
            if let Some(event) = record.expire(now) {
                self.events.publish(event);
            }
        }
    }

    fn worker(&self, id: AgentId) -> Result<(), BrokerError> {
        self.workers
            .get(&id)
            .filter(|worker| worker.enabled)
            .map(|_| ())
            .ok_or(BrokerError::Unauthorized)
    }

    fn record(&mut self, id: TaskId) -> Result<&mut Record, BrokerError> {
        self.tasks
            .iter_mut()
            .find(|record| record.task.id == id)
            .ok_or(BrokerError::Missing)
    }
}

fn remaining_ms(expires: Instant, now: Instant) -> u64 {
    expires.duration_since(now).as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use vespera_core::EventKind;

    fn task() -> Task {
        Task {
            id: TaskId::new(),
            prompt: "test input".to_owned(),
            required_capabilities: BTreeSet::from(["text-generation".parse().unwrap()]),
            state: TaskState::Queued,
        }
    }

    fn workers(broker: &mut LeaseBroker) -> (AgentId, AgentId) {
        let first = AgentId::new();
        let second = AgentId::new();
        for worker in [first, second] {
            broker
                .approve_worker(worker, task().required_capabilities)
                .unwrap();
        }
        (first, second)
    }

    #[test]
    fn lease_requires_approved_identity_capability_and_task_allowlist() {
        let mut broker = LeaseBroker::default();
        let (first, second) = workers(&mut broker);
        let unqualified = AgentId::new();
        broker.approve_worker(unqualified, BTreeSet::new()).unwrap();
        let first_task = task();
        let first_id = first_task.id;
        broker
            .submit(
                first_task,
                BTreeSet::from([first, unqualified]),
                Duration::from_secs(60),
                2,
            )
            .unwrap();
        assert!(matches!(
            broker.claim(AgentId::new()),
            Err(BrokerError::Unauthorized)
        ));
        assert!(broker.claim(second).unwrap().is_none());
        assert!(broker.claim(unqualified).unwrap().is_none());
        let lease = broker.claim(first).unwrap().unwrap();
        assert_eq!(lease.task.id, first_id);
        assert_eq!(lease.task.state, TaskState::Running);
        assert!(matches!(broker.claim(first), Err(BrokerError::Busy)));
        assert_eq!(
            broker.renew(second, lease.fence),
            Err(BrokerError::StaleLease)
        );
        assert!(matches!(
            broker.complete(second, lease.fence, "forged".into()),
            Err(BrokerError::StaleLease)
        ));
        assert!(matches!(
            broker.approve_worker(first, BTreeSet::new()),
            Err(BrokerError::Duplicate)
        ));
    }

    #[test]
    fn expiry_and_revocation_fence_old_attempts_and_enforce_retry_budget() {
        let mut broker = LeaseBroker::default();
        let (first, second) = workers(&mut broker);
        let task = task();
        let id = task.id;
        broker
            .submit(
                task,
                BTreeSet::from([first, second]),
                Duration::from_secs(120),
                2,
            )
            .unwrap();
        let before_expiry = Instant::now() - LEASE_TTL;
        let old = broker.claim_at(first, before_expiry).unwrap().unwrap();
        assert!(matches!(
            broker.complete(first, old.fence, "late".into()),
            Err(BrokerError::StaleLease)
        ));
        let new = broker.claim(second).unwrap().unwrap();
        assert_eq!(new.fence.generation, 2);
        let expired = broker.replay(Some(first), None, 8).unwrap();
        assert_eq!(expired.entries.len(), 2);
        assert!(
            matches!(expired.entries[1].event.kind, EventKind::ExternalTaskChanged {
            change: ExternalTaskChange::LeaseExpired, state: TaskState::Queued, attempt: Some(attempt), ..
        } if attempt.attempt_id == old.fence.attempt_id)
        );
        assert_eq!(
            broker.replay(Some(second), None, 8).unwrap().entries.len(),
            1
        );
        assert_ne!(old.fence.attempt_id, new.fence.attempt_id);
        assert_eq!(broker.renew(first, old.fence), Err(BrokerError::StaleLease));
        broker.revoke_worker(second).unwrap();
        assert!(matches!(
            broker.complete(second, new.fence, "revoked".into()),
            Err(BrokerError::Unauthorized)
        ));
        assert_eq!(broker.state(id), Ok(TaskState::Failed));
        assert!(matches!(
            broker.replay(Some(second), None, 8),
            Err(BrokerError::Unauthorized)
        ));
        let operator = broker.replay(None, None, 8).unwrap();
        assert!(matches!(
            operator.entries.last().unwrap().event.kind,
            EventKind::ExternalTaskChanged {
                change: ExternalTaskChange::WorkerRevoked,
                state: TaskState::Failed,
                ..
            }
        ));
        assert!(broker.claim(first).unwrap().is_none());
    }

    #[test]
    fn completion_is_idempotent_and_cancellation_has_one_terminal_winner() {
        let mut broker = LeaseBroker::default();
        let (worker, _) = workers(&mut broker);
        let first = task();
        let first_id = first.id;
        let second = task();
        let second_id = second.id;
        for task in [first, second] {
            broker
                .submit(task, BTreeSet::from([worker]), Duration::from_secs(60), 2)
                .unwrap();
        }
        let lease = broker.claim(worker).unwrap().unwrap();
        assert_eq!(lease.task.id, first_id);
        let accepted = broker
            .complete(worker, lease.fence, "result".into())
            .unwrap();
        let retry = broker
            .complete(worker, lease.fence, "result".into())
            .unwrap();
        assert!(accepted == retry);
        assert!(broker.result(first_id).unwrap() == Some(&accepted));
        assert!(matches!(
            broker.complete(worker, lease.fence, "conflict".into()),
            Err(BrokerError::ResultConflict)
        ));
        assert_eq!(broker.cancel(first_id), Ok(TaskState::Completed));
        assert_eq!(
            broker.renew(worker, lease.fence),
            Err(BrokerError::StaleLease)
        );
        let second = broker.claim(worker).unwrap().unwrap();
        assert_eq!(second.task.id, second_id);
        assert_eq!(broker.cancel(second_id), Ok(TaskState::Cancelled));
        assert_eq!(broker.cancel(second_id), Ok(TaskState::Cancelled));
        assert!(matches!(
            broker.complete(worker, second.fence, "late".into()),
            Err(BrokerError::StaleLease)
        ));
        let events = broker.replay(Some(worker), None, 8).unwrap();
        assert_eq!(events.entries.len(), 4);
        assert!(
            matches!(events.entries[3].event.kind, EventKind::ExternalTaskChanged {
            change: ExternalTaskChange::Cancelled, attempt: Some(attempt), ..
        } if attempt.attempt_id == second.fence.attempt_id)
        );
    }

    #[test]
    fn renewal_obeys_deadline_and_submission_respects_capacity() {
        let mut broker = LeaseBroker::default();
        let (worker, _) = workers(&mut broker);
        let cursor = broker.replay(None, None, 1).unwrap().next_cursor;
        let first = task();
        let lifetime = Duration::from_secs(5);
        broker
            .submit(first.clone(), BTreeSet::from([worker]), lifetime, 1)
            .unwrap();
        assert_eq!(
            broker.submit(first, BTreeSet::from([worker]), lifetime, 1),
            Err(BrokerError::Duplicate)
        );
        let lease = broker.claim(worker).unwrap().unwrap();
        assert!(lease.expires_in_ms <= 5_000);
        assert!(broker.renew(worker, lease.fence).unwrap() <= 5_000);
        for _ in 1..MAX_TASKS {
            broker
                .submit(task(), BTreeSet::from([worker]), lifetime, 1)
                .unwrap();
        }
        assert!(matches!(
            broker.replay(None, Some(cursor), 1),
            Err(BrokerError::ExpiredCursor)
        ));
        let retained = broker.replay(None, None, 1).unwrap();
        assert_eq!(retained.discarded_through, 1);
        assert_eq!(retained.next_cursor.sequence, 2);
        assert!(retained.has_more);
        let after = broker.replay(None, Some(retained.next_cursor), 1).unwrap();
        assert_eq!(after.entries[0].cursor.sequence, 3);
        assert!(matches!(
            LeaseBroker::default().replay(None, Some(cursor), 1),
            Err(BrokerError::ForeignCursor)
        ));
        assert_eq!(
            broker.submit(task(), BTreeSet::from([worker]), lifetime, 1),
            Err(BrokerError::Capacity)
        );
        assert!(matches!(
            broker.complete(worker, lease.fence, "x".repeat(MAX_TEXT + 1)),
            Err(BrokerError::Invalid(_))
        ));
        assert_eq!(broker.state(lease.task.id), Ok(TaskState::Running));
    }
}
