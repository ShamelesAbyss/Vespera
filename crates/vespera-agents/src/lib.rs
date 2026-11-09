//! Agent runtime, capability routing, permission enforcement, and memory isolation for VESPERA.
//!
//! Independent agent runtime foundation:
//! - Agent lifecycle management (`Suspended`, `Idle`, `Running`, `Failed`).
//! - Decoupled model selection (`Fixed` and `Capability` policies) preserving agent identity.
//! - Permission-gated memory layers, inter-agent messaging, task delegation, and simulation actions.
//! - Resource-scheduled task execution integrated with `vespera-scheduler`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use vespera_core::{
    AgentDefinition, AgentId, AgentStatus, Capability, DomainError, EventEnvelope, MemoryId,
    MemoryLayer, ModelId, ModelSelectionPolicy, Permission, ProviderId, SchedulerSample, Task,
    TaskId, TaskLeaseSample, TaskState,
};
use vespera_model_registry::{ModelRegistration, ModelRegistry, RegistryError};
use vespera_providers::CancellationToken;
use vespera_scheduler::{
    ReservationObservation, ResourceScheduler, ScheduledInference, SchedulerError,
};

mod execution;
pub use execution::{ExecutionError, ExecutionOptions, MemoryContext, TaskExecutionResult};
mod limits;
mod task_store;
pub use limits::{RuntimeLimits, RuntimeUsage};
pub use task_store::{
    AgentTaskStore, AgentTaskStoreError, InboxAuditCursor, InboxAuditPage, StoredAgentTask,
    StoredTaskSummary,
};

/// Current agent grants/accepted assignments, shared with durable storage without
/// making model/provider identity an owner. Caller identity remains a trusted boundary.
struct RuntimeMemoryAuthority {
    state: Arc<Mutex<RuntimeState>>,
}
impl vespera_memory::MemoryAuthorizer for RuntimeMemoryAuthority {
    fn allowed(&self, actor: AgentId, permission: Permission) -> bool {
        matches!(permission, Permission::MemoryRead | Permission::MemoryWrite)
            && lock_state(&self.state)
                .agents
                .get(&actor)
                .is_some_and(|agent| agent.permissions.contains(&permission))
    }
    fn owns_task(&self, actor: AgentId, task_id: TaskId) -> bool {
        let state = lock_state(&self.state);
        state.tasks.contains_key(&task_id) && state.task_assignments.get(&task_id) == Some(&actor)
    }
}

/// Error taxonomy for agent runtime operations.
#[derive(Debug, Error)]
pub enum AgentRuntimeError {
    #[error(transparent)]
    TaskStore(#[from] AgentTaskStoreError),
    /// An operator hold prevents the agent from accepting work.
    #[error("agent {agent_id} has a durable operator suspension")]
    DurablySuspended {
        /// Suspended identity.
        agent_id: AgentId,
    },
    /// Invalid operator capacity policy.
    #[error("invalid runtime limit: {field}")]
    InvalidLimits {
        /// Field name.
        field: &'static str,
    },
    /// Retained record or content bound reached; no partial write was performed.
    #[error("agent runtime capacity reached: {resource}")]
    Capacity {
        /// Bound name.
        resource: &'static str,
    },
    /// Memory identities cannot overwrite previous records.
    #[error("memory {memory_id} is already recorded")]
    DuplicateMemory {
        /// Conflicting memory identity.
        memory_id: MemoryId,
    },
    /// A task reference does not belong to the claimed namespace/participants.
    #[error("task {task_id} is not an authorized source reference")]
    TaskReferenceDenied {
        /// Requested source identity.
        task_id: TaskId,
    },
    /// Managed provider work must stop before its lease can be released.
    #[error("task {task_id} has managed inference in progress; request cancellation instead")]
    ExecutionInProgress {
        /// Target task.
        task_id: TaskId,
    },
    /// Task identities and assignments are immutable once accepted.
    #[error("task {task_id} conflicts with an existing task or execution")]
    TaskConflict {
        /// Conflicting task identity.
        task_id: TaskId,
    },
    /// The caller supplied a model other than the execution's owned lease.
    #[error("model does not match the recorded execution for task {task_id}")]
    ExecutionMismatch {
        /// Target task.
        task_id: TaskId,
    },
    /// Domain validation error from `vespera-core`.
    #[error("agent domain validation error: {0}")]
    Domain(#[from] DomainError),
    /// Model registry error during model resolution.
    #[error("model registry error: {0}")]
    Registry(#[from] RegistryError),
    /// Scheduler error during resource admission.
    #[error("scheduler error: {0}")]
    Scheduler(#[from] SchedulerError),
    /// Requested agent ID is not registered.
    #[error("agent {agent_id} is not registered")]
    AgentNotFound {
        /// Missing agent identifier.
        agent_id: AgentId,
    },
    /// Attempted to register an agent ID that already exists.
    #[error("agent {agent_id} is already registered")]
    DuplicateAgent {
        /// Duplicate agent identifier.
        agent_id: AgentId,
    },
    /// Agent lacked the required permission for an operation.
    #[error("agent {agent_id} lacks required permission {permission:?}")]
    PermissionDenied {
        /// Agent attempting the operation.
        agent_id: AgentId,
        /// Missing permission.
        permission: Permission,
    },
    /// Agent is not in an executable status (`Idle`).
    #[error("agent {agent_id} cannot execute tasks while in status {status:?}")]
    InvalidAgentState {
        /// Agent identifier.
        agent_id: AgentId,
        /// Current non-runnable status.
        status: AgentStatus,
    },
    /// No registered and available model satisfied the agent policy and task capabilities.
    #[error("no available model satisfied policy for agent {agent_id}")]
    NoMatchingModel {
        /// Agent identifier.
        agent_id: AgentId,
    },
    /// Task ID was not found in the runtime task table.
    #[error("task {task_id} not found")]
    TaskNotFound {
        /// Missing task identifier.
        task_id: TaskId,
    },
}

/// A validated memory entry stored within an agent's isolated memory namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMemoryEntry {
    /// Unique memory record identifier.
    pub id: MemoryId,
    /// Owning agent identifier.
    pub agent_id: AgentId,
    /// Memory classification layer.
    pub layer: MemoryLayer,
    /// Non-empty memory content.
    pub content: String,
    /// Optional task that produced this memory entry.
    pub source_task_id: Option<TaskId>,
    /// Monotonic logical tick when the memory was recorded.
    pub logical_tick: u64,
}

/// A permission-checked message sent from one agent to another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMessage {
    /// Sender agent identifier.
    pub from_agent_id: AgentId,
    /// Recipient agent identifier.
    pub to_agent_id: AgentId,
    /// Non-empty message payload.
    pub content: String,
    /// Optional correlated task identifier.
    pub task_id: Option<TaskId>,
    /// Monotonic logical tick when the message was dispatched.
    pub logical_tick: u64,
}

/// Active task lease combining a running task, its assigned agent, and its scheduled model lease.
#[derive(Clone)]
pub struct ActiveTaskExecution {
    /// Task identifier.
    pub task_id: TaskId,
    /// Executing agent identifier.
    pub agent_id: AgentId,
    /// Admitted model and provider lease from `ResourceScheduler`.
    pub inference: ScheduledInference,
}

impl fmt::Debug for ActiveTaskExecution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActiveTaskExecution")
            .field("task_id", &self.task_id)
            .field("agent_id", &self.agent_id)
            .field("inference", &self.inference)
            .finish()
    }
}

#[derive(Debug, Default)]
struct RuntimeState {
    retained_content_bytes: usize,
    memory_ids: BTreeSet<MemoryId>,
    agents: BTreeMap<AgentId, AgentDefinition>,
    memories: BTreeMap<AgentId, Vec<AgentMemoryEntry>>,
    inbox: BTreeMap<AgentId, Vec<AgentMessage>>,
    tasks: BTreeMap<TaskId, Task>,
    task_assignments: BTreeMap<TaskId, AgentId>,
    executions: BTreeMap<TaskId, OwnedInference>,
    managed_tasks: BTreeMap<TaskId, CancellationToken>,
    logical_tick: u64,
    task_store: Option<AgentTaskStore>,
    durably_suspended: BTreeSet<AgentId>,
}

#[derive(Debug)]
struct OwnedInference {
    inference: ScheduledInference,
    memory_context_records: Option<usize>,
}

fn lock_state(state: &Mutex<RuntimeState>) -> MutexGuard<'_, RuntimeState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// Cancels local task preparation even when the owning future is dropped/aborted.
// Scheduler lifecycle guards independently retain uncertain provider reservations.
struct TaskStartGuard {
    state: Arc<Mutex<RuntimeState>>,
    scheduler: Arc<ResourceScheduler>,
    task_id: TaskId,
    agent_id: AgentId,
    armed: bool,
    failed: bool,
}
impl Drop for TaskStartGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut state = lock_state(&self.state);
        state.managed_tasks.remove(&self.task_id);
        if let Some(mut task) = state.tasks.get(&self.task_id).cloned() {
            let previous = task.state;
            let _ = task.state.transition(if self.failed {
                TaskState::Failed
            } else {
                TaskState::Cancelled
            });
            if let Some(store) = &state.task_store {
                let _ = store.transition(&task, self.agent_id, previous, None);
            }
            state.tasks.insert(self.task_id, task);
        }
        if let Some(agent) = state.agents.get_mut(&self.agent_id) {
            let status = if self.failed {
                AgentStatus::Failed
            } else {
                AgentStatus::Idle
            };
            apply_observed_status(&self.scheduler, agent, status, Some(self.task_id));
        }
    }
}

fn apply_observed_status(
    scheduler: &ResourceScheduler,
    agent: &mut AgentDefinition,
    status: AgentStatus,
    task: Option<TaskId>,
) {
    let previous = std::mem::replace(&mut agent.status, status);
    scheduler.observe_agent_status(agent.identity.id, previous, status, task);
}

impl RuntimeState {
    fn next_tick(&mut self) -> u64 {
        self.logical_tick = self.logical_tick.saturating_add(1);
        self.logical_tick
    }
}

/// Central agent runtime coordinating identity, permissions, memory, and scheduled execution.
#[derive(Clone)]
pub struct AgentRuntime {
    limits: RuntimeLimits,
    registry: Arc<ModelRegistry>,
    scheduler: Arc<ResourceScheduler>,
    state: Arc<Mutex<RuntimeState>>,
}

impl fmt::Debug for AgentRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentRuntime")
            .field("registry", &self.registry)
            .field("scheduler", &self.scheduler)
            .finish()
    }
}

impl AgentRuntime {
    /// Attach durable ownership and inboxes before accepting tasks or messages.
    /// Existing operator holds are applied to registered agents.
    pub fn attach_task_store(&self, store: AgentTaskStore) -> Result<(), AgentRuntimeError> {
        let mut state = lock_state(&self.state);
        if state.task_store.is_some()
            || !state.tasks.is_empty()
            || state.inbox.values().any(|inbox| !inbox.is_empty())
        {
            return Err(AgentTaskStoreError::Busy.into());
        }
        let suspended = store.suspensions()?;
        state.logical_tick = state.logical_tick.max(store.last_message_tick()?);
        for id in &suspended {
            if let Some(agent) = state.agents.get_mut(id) {
                apply_observed_status(&self.scheduler, agent, AgentStatus::Suspended, None);
            }
        }
        state.durably_suspended = suspended;
        state.task_store = Some(store);
        Ok(())
    }
    /// Authorization port for explicit durable memory stores. This creates no file,
    /// runs no inference and does not replace the separate in-process memory APIs.
    pub fn memory_authorizer(&self) -> Arc<dyn vespera_memory::MemoryAuthorizer> {
        Arc::new(RuntimeMemoryAuthority {
            state: self.state.clone(),
        })
    }
    /// Creates a new agent runtime bound to the model registry and resource scheduler.
    #[must_use]
    pub fn new(registry: Arc<ModelRegistry>, scheduler: Arc<ResourceScheduler>) -> Self {
        Self {
            limits: RuntimeLimits::default(),
            registry,
            scheduler,
            state: Arc::new(Mutex::new(RuntimeState::default())),
        }
    }

    /// Create a runtime with explicit positive retained-data bounds.
    pub fn with_limits(
        registry: Arc<ModelRegistry>,
        scheduler: Arc<ResourceScheduler>,
        limits: RuntimeLimits,
    ) -> Result<Self, AgentRuntimeError> {
        limits.validate()?;
        Ok(Self {
            limits,
            ..Self::new(registry, scheduler)
        })
    }

    /// Inspect local record counts and logical retained text, not measured process memory.
    pub async fn usage_snapshot(&self) -> RuntimeUsage {
        let state = lock_state(&self.state);
        RuntimeUsage {
            agents: state.agents.len(),
            tasks: state.tasks.len(),
            retained_content_bytes: state.retained_content_bytes,
        }
    }

    /// Registers a validated agent definition and initializes its isolated memory and inbox.
    pub async fn register_agent(
        &self,
        mut agent: AgentDefinition,
    ) -> Result<(), AgentRuntimeError> {
        agent.validate()?;
        let agent_id = agent.identity.id;
        if agent.status == AgentStatus::Running {
            return Err(AgentRuntimeError::InvalidAgentState {
                agent_id,
                status: agent.status,
            });
        }

        let mut state = lock_state(&self.state);
        if state.agents.contains_key(&agent_id) {
            return Err(AgentRuntimeError::DuplicateAgent { agent_id });
        }

        if state.agents.len() >= self.limits.max_agents {
            return Err(limits::capacity("agents"));
        }
        self.limits
            .reserve(&mut state, limits::agent_bytes(&agent)?)?;

        if state.durably_suspended.contains(&agent_id) {
            agent.status = AgentStatus::Suspended;
        }
        let status = agent.status;
        state.agents.insert(agent_id, agent);
        state.memories.entry(agent_id).or_default();
        state.inbox.entry(agent_id).or_default();
        self.scheduler.observe_agent_registration(agent_id, status);
        Ok(())
    }

    /// Observe all identities and effective statuses already registered in this runtime.
    pub fn observe_registered_agents(&self) {
        for (agent_id, agent) in &lock_state(&self.state).agents {
            self.scheduler
                .observe_agent_registration(*agent_id, agent.status);
        }
    }

    /// Retrieves a registered agent definition by ID.
    pub async fn get_agent(&self, agent_id: AgentId) -> Result<AgentDefinition, AgentRuntimeError> {
        let state = lock_state(&self.state);
        state
            .agents
            .get(&agent_id)
            .cloned()
            .ok_or(AgentRuntimeError::AgentNotFound { agent_id })
    }

    /// Inspect a control-plane task snapshot without exposing mutable runtime state.
    pub async fn get_task(&self, task_id: TaskId) -> Result<Task, AgentRuntimeError> {
        lock_state(&self.state)
            .tasks
            .get(&task_id)
            .cloned()
            .ok_or(AgentRuntimeError::TaskNotFound { task_id })
    }

    /// Read task state without cloning its prompt or awaiting a runtime.
    pub fn task_state(&self, task_id: TaskId) -> Option<TaskState> {
        lock_state(&self.state)
            .tasks
            .get(&task_id)
            .map(|task| task.state)
    }

    /// Read the envelope committed with the terminal task state, when available.
    pub fn durable_terminal_event(
        &self,
        task_id: TaskId,
    ) -> Result<Option<EventEnvelope>, AgentTaskStoreError> {
        match &lock_state(&self.state).task_store {
            Some(store) => store.terminal_event(task_id),
            None => Ok(None),
        }
    }

    /// Read identities from an admitted local inference lease without exposing its payload.
    pub fn active_inference(&self, task_id: TaskId) -> Option<(ModelId, ProviderId)> {
        lock_state(&self.state)
            .executions
            .get(&task_id)
            .map(|execution| {
                (
                    execution.inference.model_id,
                    execution.inference.provider_id,
                )
            })
    }

    /// Capture bounded running task ownership together with scheduler accounting.
    pub fn observe_scheduler(&self) -> ReservationObservation {
        // Completion releases its lease under the same agent-then-scheduler lock order.
        let state = lock_state(&self.state);
        let candidates: Vec<_> = state
            .executions
            .iter()
            .filter_map(|(&task_id, execution)| {
                let agent_id = *state.task_assignments.get(&task_id)?;
                Some(TaskLeaseSample {
                    task_id,
                    agent_id,
                    lease_id: execution.inference.lease_id(),
                    model_id: execution.inference.model_id,
                    provider_id: execution.inference.provider_id,
                    memory_context_records: execution.memory_context_records,
                })
            })
            .take(SchedulerSample::MAX_TASKS)
            .collect();
        let observation = self.scheduler.observe_task_reservations(&candidates);
        drop(state);
        observation
    }

    /// Cancel accepted queued work for its recorded owner. Running work must use
    /// managed cancellation instead. Synchronous so ownership guards can clean up on drop.
    pub fn cancel_queued_task(
        &self,
        agent_id: AgentId,
        task_id: TaskId,
    ) -> Result<bool, AgentRuntimeError> {
        let mut state = lock_state(&self.state);
        if state.task_assignments.get(&task_id) != Some(&agent_id) {
            return Err(AgentRuntimeError::TaskReferenceDenied { task_id });
        }
        let mut task = state
            .tasks
            .get(&task_id)
            .cloned()
            .ok_or(AgentRuntimeError::TaskNotFound { task_id })?;
        if task.state == TaskState::Queued {
            task.state.transition(TaskState::Cancelled)?;
            if let Some(store) = &state.task_store {
                store.transition(&task, agent_id, TaskState::Queued, None)?;
            }
            state.tasks.insert(task_id, task);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Updates an agent's operational status (`Suspended`, `Idle`, `Running`, `Failed`).
    pub async fn set_agent_status(
        &self,
        agent_id: AgentId,
        status: AgentStatus,
    ) -> Result<(), AgentRuntimeError> {
        let mut state = lock_state(&self.state);
        if status != AgentStatus::Suspended && state.durably_suspended.contains(&agent_id) {
            return Err(AgentRuntimeError::DurablySuspended { agent_id });
        }
        let agent = state
            .agents
            .get_mut(&agent_id)
            .ok_or(AgentRuntimeError::AgentNotFound { agent_id })?;
        if agent.status == AgentStatus::Running || status == AgentStatus::Running {
            return Err(AgentRuntimeError::InvalidAgentState {
                agent_id,
                status: agent.status,
            });
        }
        apply_observed_status(&self.scheduler, agent, status, None);
        Ok(())
    }

    /// Rebinds an agent's model selection policy without changing its identity, permissions, or memory.
    pub async fn rebind_model_policy(
        &self,
        agent_id: AgentId,
        new_policy: ModelSelectionPolicy,
    ) -> Result<(), AgentRuntimeError> {
        new_policy.validate()?;
        let mut state = lock_state(&self.state);
        let agent = state
            .agents
            .get(&agent_id)
            .ok_or(AgentRuntimeError::AgentNotFound { agent_id })?;
        if agent.status == AgentStatus::Running {
            return Err(AgentRuntimeError::InvalidAgentState {
                agent_id,
                status: agent.status,
            });
        }
        let old_size = limits::agent_bytes(agent)?;
        let mut updated = agent.clone();
        updated.model_policy = new_policy;
        let new_size = limits::agent_bytes(&updated)?;
        self.limits.check_record(new_size)?;
        let total = state
            .retained_content_bytes
            .checked_sub(old_size)
            .and_then(|bytes| bytes.checked_add(new_size))
            .ok_or_else(|| limits::capacity("retained_content_bytes"))?;
        if total > self.limits.max_retained_content_bytes {
            return Err(limits::capacity("retained_content_bytes"));
        }
        state.retained_content_bytes = total;
        state.agents.insert(agent_id, updated);
        Ok(())
    }

    /// Records a memory entry into the agent's isolated namespace if the agent holds `Permission::MemoryWrite`.
    pub async fn write_memory(
        &self,
        agent_id: AgentId,
        memory_id: MemoryId,
        layer: MemoryLayer,
        content: String,
        source_task_id: Option<TaskId>,
    ) -> Result<AgentMemoryEntry, AgentRuntimeError> {
        if content.trim().is_empty() {
            return Err(AgentRuntimeError::Domain(DomainError::EmptyField {
                field: "memory.content",
            }));
        }

        let mut state = lock_state(&self.state);
        let agent = state
            .agents
            .get(&agent_id)
            .ok_or(AgentRuntimeError::AgentNotFound { agent_id })?;

        if !agent.permissions.contains(&Permission::MemoryWrite) {
            return Err(AgentRuntimeError::PermissionDenied {
                agent_id,
                permission: Permission::MemoryWrite,
            });
        }

        if state.memory_ids.contains(&memory_id) {
            return Err(AgentRuntimeError::DuplicateMemory { memory_id });
        }
        if let Some(task_id) = source_task_id
            && state.task_assignments.get(&task_id) != Some(&agent_id)
        {
            return Err(AgentRuntimeError::TaskReferenceDenied { task_id });
        }
        if state
            .memories
            .get(&agent_id)
            .is_some_and(|entries| entries.len() >= self.limits.max_memories_per_agent)
        {
            return Err(limits::capacity("agent_memories"));
        }
        self.limits.reserve(&mut state, content.len())?;
        state.memory_ids.insert(memory_id);

        let tick = state.next_tick();
        let entry = AgentMemoryEntry {
            id: memory_id,
            agent_id,
            layer,
            content,
            source_task_id,
            logical_tick: tick,
        };

        state
            .memories
            .entry(agent_id)
            .or_default()
            .push(entry.clone());
        Ok(entry)
    }

    /// Reads memory entries for an agent (optionally filtered by layer) if the agent holds `Permission::MemoryRead`.
    pub async fn read_memory(
        &self,
        agent_id: AgentId,
        layer_filter: Option<MemoryLayer>,
    ) -> Result<Vec<AgentMemoryEntry>, AgentRuntimeError> {
        let state = lock_state(&self.state);
        let agent = state
            .agents
            .get(&agent_id)
            .ok_or(AgentRuntimeError::AgentNotFound { agent_id })?;

        if !agent.permissions.contains(&Permission::MemoryRead) {
            return Err(AgentRuntimeError::PermissionDenied {
                agent_id,
                permission: Permission::MemoryRead,
            });
        }

        let entries = state
            .memories
            .get(&agent_id)
            .map(|list| {
                list.iter()
                    .filter(|item| layer_filter.is_none_or(|l| item.layer == l))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        Ok(entries)
    }

    /// Sends a message from one registered agent to another, requiring `Permission::Communicate` on the sender.
    pub async fn send_message(
        &self,
        from_agent_id: AgentId,
        to_agent_id: AgentId,
        content: String,
        task_id: Option<TaskId>,
    ) -> Result<AgentMessage, AgentRuntimeError> {
        self.send_message_observed(from_agent_id, to_agent_id, content, task_id)
            .await
            .map(|(message, _)| message)
    }

    /// Return the audit envelope when the message committed to durable storage.
    pub async fn send_message_observed(
        &self,
        from_agent_id: AgentId,
        to_agent_id: AgentId,
        content: String,
        task_id: Option<TaskId>,
    ) -> Result<(AgentMessage, Option<EventEnvelope>), AgentRuntimeError> {
        if content.trim().is_empty() {
            return Err(AgentRuntimeError::Domain(DomainError::EmptyField {
                field: "message.content",
            }));
        }

        let mut state = lock_state(&self.state);
        let sender = state
            .agents
            .get(&from_agent_id)
            .ok_or(AgentRuntimeError::AgentNotFound {
                agent_id: from_agent_id,
            })?;

        if !sender.permissions.contains(&Permission::Communicate) {
            return Err(AgentRuntimeError::PermissionDenied {
                agent_id: from_agent_id,
                permission: Permission::Communicate,
            });
        }

        if !state.agents.contains_key(&to_agent_id) {
            return Err(AgentRuntimeError::AgentNotFound {
                agent_id: to_agent_id,
            });
        }

        if let Some(task_id) = task_id {
            let owner = match &state.task_store {
                Some(store) => store.task_owner(task_id)?,
                None => state.task_assignments.get(&task_id).copied(),
            };
            if !owner.is_some_and(|owner| owner == from_agent_id || owner == to_agent_id) {
                return Err(AgentRuntimeError::TaskReferenceDenied { task_id });
            }
        }
        if let Some(store) = &state.task_store {
            if state.durably_suspended.contains(&from_agent_id) {
                return Err(AgentRuntimeError::DurablySuspended {
                    agent_id: from_agent_id,
                });
            }
            self.limits.check_record(content.len())?;
            let message = AgentMessage {
                from_agent_id,
                to_agent_id,
                content,
                task_id,
                logical_tick: state.logical_tick.saturating_add(1),
            };
            let event = store.insert_message(&message, self.limits)?;
            state.logical_tick = message.logical_tick;
            return Ok((message, Some(event)));
        }
        if state
            .inbox
            .get(&to_agent_id)
            .is_some_and(|entries| entries.len() >= self.limits.max_messages_per_agent)
        {
            return Err(limits::capacity("agent_inbox"));
        }
        self.limits.reserve(&mut state, content.len())?;

        let tick = state.next_tick();
        let message = AgentMessage {
            from_agent_id,
            to_agent_id,
            content,
            task_id,
            logical_tick: tick,
        };

        state
            .inbox
            .entry(to_agent_id)
            .or_default()
            .push(message.clone());
        Ok((message, None))
    }

    /// Lists all messages delivered to an agent's inbox.
    pub async fn inbox_messages(
        &self,
        agent_id: AgentId,
    ) -> Result<Vec<AgentMessage>, AgentRuntimeError> {
        let state = lock_state(&self.state);
        let agent = state
            .agents
            .get(&agent_id)
            .ok_or(AgentRuntimeError::AgentNotFound { agent_id })?;
        if !agent.permissions.contains(&Permission::Communicate) {
            return Err(AgentRuntimeError::PermissionDenied {
                agent_id,
                permission: Permission::Communicate,
            });
        }
        if let Some(store) = &state.task_store {
            return Ok(store.inbox(agent_id)?);
        }
        Ok(state.inbox.get(&agent_id).cloned().unwrap_or_default())
    }

    /// Verifies that an agent is permitted to perform a simulation action (`Permission::SimulationAct`).
    pub async fn authorize_simulation_action(
        &self,
        agent_id: AgentId,
    ) -> Result<(), AgentRuntimeError> {
        let state = lock_state(&self.state);
        let agent = state
            .agents
            .get(&agent_id)
            .ok_or(AgentRuntimeError::AgentNotFound { agent_id })?;

        if !agent.permissions.contains(&Permission::SimulationAct) {
            return Err(AgentRuntimeError::PermissionDenied {
                agent_id,
                permission: Permission::SimulationAct,
            });
        }
        Ok(())
    }

    /// Delegates a validated queued task from one agent to another, requiring `Permission::Delegate`.
    pub async fn delegate_task(
        &self,
        from_agent_id: AgentId,
        to_agent_id: AgentId,
        task: Task,
    ) -> Result<TaskId, AgentRuntimeError> {
        task.validate()?;
        if task.state != TaskState::Queued {
            return Err(AgentRuntimeError::Domain(
                DomainError::InvalidTaskTransition {
                    from: task.state,
                    to: TaskState::Queued,
                },
            ));
        }
        let mut state = lock_state(&self.state);

        let sender = state
            .agents
            .get(&from_agent_id)
            .ok_or(AgentRuntimeError::AgentNotFound {
                agent_id: from_agent_id,
            })?;

        if !sender.permissions.contains(&Permission::Delegate) {
            return Err(AgentRuntimeError::PermissionDenied {
                agent_id: from_agent_id,
                permission: Permission::Delegate,
            });
        }

        if !state.agents.contains_key(&to_agent_id) {
            return Err(AgentRuntimeError::AgentNotFound {
                agent_id: to_agent_id,
            });
        }

        for agent_id in [from_agent_id, to_agent_id] {
            if state.durably_suspended.contains(&agent_id) {
                return Err(AgentRuntimeError::DurablySuspended { agent_id });
            }
        }

        let task_id = task.id;
        if state.tasks.contains_key(&task_id) {
            return Err(AgentRuntimeError::TaskConflict { task_id });
        }
        if state.tasks.len() >= self.limits.max_tasks {
            return Err(limits::capacity("tasks"));
        }
        let previous_bytes = state.retained_content_bytes;
        self.limits
            .reserve(&mut state, limits::task_bytes(&task)?)?;
        if let Some(store) = &state.task_store
            && let Err(error) = store.insert(&task, to_agent_id, Some(from_agent_id))
        {
            state.retained_content_bytes = previous_bytes;
            return Err(error.into());
        }
        state.tasks.insert(task_id, task);
        state.task_assignments.insert(task_id, to_agent_id);
        Ok(task_id)
    }

    /// Refresh only providers eligible for the given idle candidates and capabilities.
    /// This is explicit health I/O, not model activation or agent resumption.
    pub async fn refresh_candidate_health(
        &self,
        candidates: &[AgentId],
        required: &BTreeSet<Capability>,
        cancel: CancellationToken,
    ) -> Result<(), AgentRuntimeError> {
        let models = self.registry.list_registrations().await;
        let mut providers = BTreeSet::new();
        for id in candidates {
            let agent = self.get_agent(*id).await?;
            if agent.status != AgentStatus::Idle {
                continue;
            }
            for model in &models {
                if model.available
                    && required.is_subset(&model.capabilities)
                    && agent
                        .model_policy
                        .accepts(model.model_id, &model.capabilities)
                {
                    providers.insert(model.provider_id);
                }
            }
        }
        self.registry
            .health_check_providers(&providers, cancel)
            .await;
        Ok(())
    }

    /// Resolves the appropriate model for an agent and a set of required task capabilities.
    pub async fn resolve_model_for_agent(
        &self,
        agent_id: AgentId,
        task_capabilities: &BTreeSet<Capability>,
    ) -> Result<ModelRegistration, AgentRuntimeError> {
        self.resolve_model(agent_id, task_capabilities, None).await
    }

    /// Resolve an eligible model using the request's bounds for any configured latency preference.
    pub async fn resolve_model_for_execution(
        &self,
        agent_id: AgentId,
        task_capabilities: &BTreeSet<Capability>,
        options: &ExecutionOptions,
    ) -> Result<ModelRegistration, AgentRuntimeError> {
        self.resolve_model(agent_id, task_capabilities, Some(options))
            .await
    }

    async fn resolve_model(
        &self,
        agent_id: AgentId,
        task_capabilities: &BTreeSet<Capability>,
        options: Option<&ExecutionOptions>,
    ) -> Result<ModelRegistration, AgentRuntimeError> {
        let agent = self.get_agent(agent_id).await?;

        match &agent.model_policy {
            ModelSelectionPolicy::Fixed { model_id }
            | ModelSelectionPolicy::FixedWithCapabilities { model_id, .. } => {
                let model = self.registry.get_model(*model_id).await?;
                if !model.available
                    || !task_capabilities.is_subset(&model.capabilities)
                    || !agent
                        .model_policy
                        .accepts(model.model_id, &model.capabilities)
                {
                    return Err(AgentRuntimeError::NoMatchingModel { agent_id });
                }
                Ok(model)
            }
            ModelSelectionPolicy::Capability { required } => {
                let combined: BTreeSet<Capability> =
                    required.union(task_capabilities).cloned().collect();
                let mut candidates = self.registry.find_by_capability(&combined).await;
                candidates.sort_by_key(|m| m.model_id);
                if let Some(options) = options
                    && let Some(preferred) = self
                        .scheduler
                        .prefer_lower_latency(
                            &candidates,
                            options.context_tokens,
                            options.max_output_tokens,
                        )
                        .await
                    && let Some(index) = candidates
                        .iter()
                        .position(|candidate| candidate.model_id == preferred)
                {
                    return Ok(candidates.swap_remove(index));
                }
                candidates
                    .into_iter()
                    .next()
                    .ok_or(AgentRuntimeError::NoMatchingModel { agent_id })
            }
        }
    }

    /// Begins execution of a task by an `Idle` agent, resolving a model and acquiring a scheduler lease.
    pub async fn start_task_execution(
        &self,
        agent_id: AgentId,
        task: Task,
        requested_context_tokens: Option<u32>,
        cancel: CancellationToken,
    ) -> Result<ActiveTaskExecution, AgentRuntimeError> {
        self.start_task_execution_inner(agent_id, task, requested_context_tokens, cancel, None)
            .await
    }

    async fn start_task_execution_inner(
        &self,
        agent_id: AgentId,
        mut task: Task,
        requested_context_tokens: Option<u32>,
        cancel: CancellationToken,
        managed: Option<(
            CancellationToken,
            &ExecutionOptions,
            &[vespera_providers::ChatMessage],
        )>,
    ) -> Result<ActiveTaskExecution, AgentRuntimeError> {
        task.validate()?;
        if task.state != TaskState::Queued {
            return Err(AgentRuntimeError::Domain(
                DomainError::InvalidTaskTransition {
                    from: task.state,
                    to: TaskState::Running,
                },
            ));
        }

        {
            let mut state = lock_state(&self.state);
            let agent = state
                .agents
                .get(&agent_id)
                .ok_or(AgentRuntimeError::AgentNotFound { agent_id })?;
            if agent.status != AgentStatus::Idle {
                return Err(AgentRuntimeError::InvalidAgentState {
                    agent_id,
                    status: agent.status,
                });
            }
            if let Some(existing) = state.tasks.get(&task.id)
                && (existing != &task || state.task_assignments.get(&task.id) != Some(&agent_id))
            {
                return Err(AgentRuntimeError::TaskConflict { task_id: task.id });
            }
            let queued = state.tasks.contains_key(&task.id);
            let previous_bytes = state.retained_content_bytes;
            if !queued {
                if state.tasks.len() >= self.limits.max_tasks {
                    return Err(limits::capacity("tasks"));
                }
                self.limits
                    .reserve(&mut state, limits::task_bytes(&task)?)?;
            }
            task.state.transition(TaskState::Running)?;
            if let Some(store) = &state.task_store {
                let checkpoint = if queued {
                    store.transition(&task, agent_id, TaskState::Queued, None)
                } else {
                    store.insert(&task, agent_id, None)
                };
                if let Err(error) = checkpoint {
                    state.retained_content_bytes = previous_bytes;
                    return Err(error.into());
                }
            }
            state.tasks.insert(task.id, task.clone());
            state.task_assignments.insert(task.id, agent_id);
            if let Some((token, _, _)) = &managed {
                state.managed_tasks.insert(task.id, token.clone());
            }
            if let Some(agent) = state.agents.get_mut(&agent_id) {
                apply_observed_status(&self.scheduler, agent, AgentStatus::Running, Some(task.id));
            }
        }

        let mut preparation = TaskStartGuard {
            state: self.state.clone(),
            scheduler: self.scheduler.clone(),
            task_id: task.id,
            agent_id,
            armed: true,
            failed: false,
        };

        let resolved_model = match self
            .resolve_model(
                agent_id,
                &task.required_capabilities,
                managed.as_ref().map(|(_, options, _)| *options),
            )
            .await
        {
            Ok(model) => model,
            Err(error) => {
                preparation.failed = true;
                return Err(error);
            }
        };

        let admitted = if let Some((_, options, messages)) = managed {
            let context_tokens = requested_context_tokens.unwrap_or_else(|| {
                resolved_model
                    .context_window
                    .unwrap_or(self.scheduler.max_context_tokens())
                    .min(self.scheduler.max_context_tokens())
            });
            let request = match vespera_providers::BoundedInferenceRequest::new(
                &resolved_model.provider_model_identifier,
                messages.to_vec(),
                options.max_output_tokens,
                context_tokens,
                options.temperature,
                self.scheduler.inference_keep_alive_seconds(),
            ) {
                Ok(request) => request,
                Err(error) => {
                    preparation.failed = true;
                    return Err(SchedulerError::Provider(error).into());
                }
            };
            self.scheduler
                .admit_request(resolved_model.model_id, &request, cancel)
                .await
        } else {
            self.scheduler
                .admit_and_prepare(resolved_model.model_id, requested_context_tokens, cancel)
                .await
        };
        let inference = match admitted {
            Ok(inference) => inference,
            Err(error) => {
                preparation.failed = !matches!(
                    error,
                    SchedulerError::Cancelled
                        | SchedulerError::Provider(
                            vespera_providers::ProviderError::Cancelled { .. }
                        )
                );
                return Err(error.into());
            }
        };

        let task_id = task.id;

        {
            let mut state = lock_state(&self.state);
            state.executions.insert(
                task_id,
                OwnedInference {
                    inference: inference.clone(),
                    memory_context_records: None,
                },
            );
        }
        preparation.armed = false;

        Ok(ActiveTaskExecution {
            task_id,
            agent_id,
            inference,
        })
    }

    /// Finishes an active task execution, releasing the scheduler slot and updating task and agent states.
    pub async fn finish_task_execution(
        &self,
        task_id: TaskId,
        model_id: ModelId,
        terminal_state: TaskState,
    ) -> Result<Task, AgentRuntimeError> {
        self.finish_task_execution_inner(task_id, model_id, terminal_state, None, None)
    }

    fn finish_task_execution_inner(
        &self,
        task_id: TaskId,
        model_id: ModelId,
        terminal_state: TaskState,
        owned: Option<&ScheduledInference>,
        result: Option<&TaskExecutionResult>,
    ) -> Result<Task, AgentRuntimeError> {
        if !matches!(
            terminal_state,
            TaskState::Completed | TaskState::Failed | TaskState::Cancelled
        ) {
            return Err(AgentRuntimeError::Domain(
                DomainError::InvalidTaskTransition {
                    from: TaskState::Running,
                    to: terminal_state,
                },
            ));
        }

        let mut state = lock_state(&self.state);
        if state.managed_tasks.contains_key(&task_id) && owned.is_none() {
            return Err(AgentRuntimeError::ExecutionInProgress { task_id });
        }
        let inference = &state
            .executions
            .get(&task_id)
            .ok_or(AgentRuntimeError::TaskNotFound { task_id })?
            .inference;
        if inference.model_id != model_id
            || owned.is_some_and(|owned| owned.lease_id() != inference.lease_id())
        {
            return Err(AgentRuntimeError::ExecutionMismatch { task_id });
        }
        let assigned_agent = state
            .task_assignments
            .get(&task_id)
            .copied()
            .ok_or(AgentRuntimeError::TaskReferenceDenied { task_id })?;
        let mut task = state
            .tasks
            .get(&task_id)
            .cloned()
            .ok_or(AgentRuntimeError::TaskNotFound { task_id })?;

        if task.state != TaskState::Running {
            return Err(AgentRuntimeError::Domain(
                DomainError::InvalidTaskTransition {
                    from: task.state,
                    to: terminal_state,
                },
            ));
        }

        task.state.transition(terminal_state)?;
        let checkpoint = state
            .task_store
            .as_ref()
            .map(|store| store.transition(&task, assigned_agent, TaskState::Running, result))
            .transpose();
        if checkpoint.is_err() {
            task.state = TaskState::Failed;
        }
        state.tasks.insert(task_id, task.clone());

        if let Some(agent) = state.agents.get_mut(&assigned_agent) {
            let status = if task.state == TaskState::Failed {
                AgentStatus::Failed
            } else {
                AgentStatus::Idle
            };
            apply_observed_status(&self.scheduler, agent, status, Some(task_id));
        }

        if let Some(execution) = state.executions.remove(&task_id) {
            execution.inference.release();
        }
        state.managed_tasks.remove(&task_id);

        checkpoint?;
        Ok(task)
    }
}
