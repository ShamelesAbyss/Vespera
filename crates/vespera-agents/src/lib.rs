//! Agent runtime, capability routing, permission enforcement, and memory isolation for VESPERA.
//!
//! Implements Milestone 6:
//! - Agent lifecycle management (`Suspended`, `Idle`, `Running`, `Failed`).
//! - Decoupled model selection (`Fixed` and `Capability` policies) preserving agent identity.
//! - Permission-gated memory layers, inter-agent messaging, task delegation, and simulation actions.
//! - Resource-scheduled task execution integrated with `vespera-scheduler`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use vespera_core::{
    AgentDefinition, AgentId, AgentStatus, Capability, DomainError, MemoryId, MemoryLayer, ModelId,
    ModelSelectionPolicy, Permission, Task, TaskId, TaskState,
};
use vespera_model_registry::{ModelRegistration, ModelRegistry, RegistryError};
use vespera_providers::CancellationToken;
use vespera_scheduler::{ResourceScheduler, ScheduledInference, SchedulerError};

/// Error taxonomy for agent runtime operations.
#[derive(Debug, Error)]
pub enum AgentRuntimeError {
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
    agents: BTreeMap<AgentId, AgentDefinition>,
    memories: BTreeMap<AgentId, Vec<AgentMemoryEntry>>,
    inbox: BTreeMap<AgentId, Vec<AgentMessage>>,
    tasks: BTreeMap<TaskId, Task>,
    task_assignments: BTreeMap<TaskId, AgentId>,
    logical_tick: u64,
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
    registry: Arc<ModelRegistry>,
    scheduler: Arc<ResourceScheduler>,
    state: Arc<RwLock<RuntimeState>>,
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
    /// Creates a new agent runtime bound to the model registry and resource scheduler.
    #[must_use]
    pub fn new(registry: Arc<ModelRegistry>, scheduler: Arc<ResourceScheduler>) -> Self {
        Self {
            registry,
            scheduler,
            state: Arc::new(RwLock::new(RuntimeState::default())),
        }
    }

    /// Registers a validated agent definition and initializes its isolated memory and inbox.
    pub async fn register_agent(&self, agent: AgentDefinition) -> Result<(), AgentRuntimeError> {
        agent.validate()?;
        let agent_id = agent.identity.id;

        let mut state = self.state.write().await;
        if state.agents.contains_key(&agent_id) {
            return Err(AgentRuntimeError::DuplicateAgent { agent_id });
        }

        state.agents.insert(agent_id, agent);
        state.memories.entry(agent_id).or_default();
        state.inbox.entry(agent_id).or_default();
        Ok(())
    }

    /// Retrieves a registered agent definition by ID.
    pub async fn get_agent(&self, agent_id: AgentId) -> Result<AgentDefinition, AgentRuntimeError> {
        let state = self.state.read().await;
        state
            .agents
            .get(&agent_id)
            .cloned()
            .ok_or(AgentRuntimeError::AgentNotFound { agent_id })
    }

    /// Updates an agent's operational status (`Suspended`, `Idle`, `Running`, `Failed`).
    pub async fn set_agent_status(
        &self,
        agent_id: AgentId,
        status: AgentStatus,
    ) -> Result<(), AgentRuntimeError> {
        let mut state = self.state.write().await;
        let agent = state
            .agents
            .get_mut(&agent_id)
            .ok_or(AgentRuntimeError::AgentNotFound { agent_id })?;
        agent.status = status;
        Ok(())
    }

    /// Rebinds an agent's model selection policy without changing its identity, permissions, or memory.
    pub async fn rebind_model_policy(
        &self,
        agent_id: AgentId,
        new_policy: ModelSelectionPolicy,
    ) -> Result<(), AgentRuntimeError> {
        new_policy.validate()?;
        let mut state = self.state.write().await;
        let agent = state
            .agents
            .get_mut(&agent_id)
            .ok_or(AgentRuntimeError::AgentNotFound { agent_id })?;
        agent.model_policy = new_policy;
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

        let mut state = self.state.write().await;
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
        let state = self.state.read().await;
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
        if content.trim().is_empty() {
            return Err(AgentRuntimeError::Domain(DomainError::EmptyField {
                field: "message.content",
            }));
        }

        let mut state = self.state.write().await;
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
        Ok(message)
    }

    /// Lists all messages delivered to an agent's inbox.
    pub async fn inbox_messages(
        &self,
        agent_id: AgentId,
    ) -> Result<Vec<AgentMessage>, AgentRuntimeError> {
        let state = self.state.read().await;
        if !state.agents.contains_key(&agent_id) {
            return Err(AgentRuntimeError::AgentNotFound { agent_id });
        }
        Ok(state.inbox.get(&agent_id).cloned().unwrap_or_default())
    }

    /// Verifies that an agent is permitted to perform a simulation action (`Permission::SimulationAct`).
    pub async fn authorize_simulation_action(
        &self,
        agent_id: AgentId,
    ) -> Result<(), AgentRuntimeError> {
        let state = self.state.read().await;
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
        let mut state = self.state.write().await;

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

        let task_id = task.id;
        state.tasks.insert(task_id, task);
        state.task_assignments.insert(task_id, to_agent_id);
        Ok(task_id)
    }

    /// Resolves the appropriate model for an agent and a set of required task capabilities.
    pub async fn resolve_model_for_agent(
        &self,
        agent_id: AgentId,
        task_capabilities: &BTreeSet<Capability>,
    ) -> Result<ModelRegistration, AgentRuntimeError> {
        let agent = self.get_agent(agent_id).await?;

        match &agent.model_policy {
            ModelSelectionPolicy::Fixed { model_id } => {
                let model = self.registry.get_model(*model_id).await?;
                if !model.available || !task_capabilities.is_subset(&model.capabilities) {
                    return Err(AgentRuntimeError::NoMatchingModel { agent_id });
                }
                Ok(model)
            }
            ModelSelectionPolicy::Capability { required } => {
                let combined: BTreeSet<Capability> =
                    required.union(task_capabilities).cloned().collect();
                let mut candidates = self.registry.find_by_capability(&combined).await;
                candidates.sort_by_key(|m| m.model_id);
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
        mut task: Task,
        requested_context_tokens: Option<u32>,
        cancel: CancellationToken,
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
            let state = self.state.read().await;
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
        }

        let resolved_model = self
            .resolve_model_for_agent(agent_id, &task.required_capabilities)
            .await?;

        let inference = self
            .scheduler
            .admit_and_prepare(resolved_model.model_id, requested_context_tokens, cancel)
            .await?;

        let task_id = task.id;
        task.state = TaskState::Running;

        {
            let mut state = self.state.write().await;
            if let Some(agent) = state.agents.get_mut(&agent_id) {
                agent.status = AgentStatus::Running;
            }
            state.tasks.insert(task_id, task);
            state.task_assignments.insert(task_id, agent_id);
        }

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

        self.scheduler.release_inference(model_id).await;

        let mut state = self.state.write().await;
        let assigned_agent = state.task_assignments.get(&task_id).copied();
        let task = state
            .tasks
            .get_mut(&task_id)
            .ok_or(AgentRuntimeError::TaskNotFound { task_id })?;

        if task.state != TaskState::Running {
            return Err(AgentRuntimeError::Domain(
                DomainError::InvalidTaskTransition {
                    from: task.state,
                    to: terminal_state,
                },
            ));
        }

        task.state = terminal_state;
        let snapshot = task.clone();

        if let Some(agent_id) = assigned_agent
            && let Some(agent) = state.agents.get_mut(&agent_id)
        {
            agent.status = if terminal_state == TaskState::Failed {
                AgentStatus::Failed
            } else {
                AgentStatus::Idle
            };
        }

        Ok(snapshot)
    }
}
