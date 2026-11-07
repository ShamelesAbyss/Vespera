//! One bounded, owned inference invocation per accepted agent task.
use super::{AgentRuntime, AgentRuntimeError, lock_state};
use serde::Serialize;
use std::time::Duration;
use thiserror::Error;
use tokio::time::Instant;
use vespera_core::{AgentId, MemoryId, ModelId, Permission, ProviderId, Task, TaskId, TaskState};
use vespera_memory::{MemoryQuery, MemoryStore, StorageError};
use vespera_providers::{
    BoundedInferenceRequest, CancellationToken, ChatMessage, ChatRole, InferenceFinishReason,
    InferenceResponse, ProviderError,
};
use vespera_scheduler::ScheduledInference;

/// Owned retrieval snapshot. Content is kept out of task history and debug output.
#[derive(Clone)]
pub struct MemoryContext {
    owner: AgentId,
    ids: Vec<MemoryId>,
    message: Option<String>,
}

impl MemoryContext {
    /// Read up to eight phrase matches from the actor's namespace and include
    /// whole records that fit in an 8 KiB serialized user message.
    pub fn retrieve(
        store: &MemoryStore,
        actor: AgentId,
        phrase: &str,
    ) -> Result<Self, StorageError> {
        let query = MemoryQuery {
            limit: 8,
            ..Default::default()
        };
        let page = store.search(actor, actor, phrase, &query)?;
        let mut entries = Vec::new();
        let mut ids = Vec::new();
        let mut message = None;
        for memory in page.records {
            entries.push(serde_json::json!({
                "id": memory.record.id,
                "content": memory.record.content,
            }));
            let candidate = serde_json::json!({"untrusted_memory": entries}).to_string();
            if candidate.len() <= 8192 {
                ids.push(memory.record.id);
                message = Some(candidate);
            } else {
                entries.pop();
            }
        }
        Ok(Self {
            owner: actor,
            ids,
            message,
        })
    }
}

/// Explicit limits for a single task, including queueing, loading and inference time.
#[derive(Debug, Clone)]
pub struct ExecutionOptions {
    /// Maximum generated tokens; the provider must honor the transmitted bound.
    pub max_output_tokens: u32,
    /// Optional context ceiling, constrained by model and scheduler limits.
    pub context_tokens: Option<u32>,
    /// Absolute local task budget, including preparation and queue waits.
    pub timeout: Duration,
    /// Optional finite sampling temperature in 0..=2.
    pub temperature: Option<f32>,
    /// Maximum accepted response content bytes (1..=8 MiB).
    pub max_response_bytes: usize,
}
impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            max_output_tokens: 256,
            context_tokens: None,
            timeout: Duration::from_secs(60),
            temperature: None,
            max_response_bytes: 256 * 1024,
        }
    }
}
impl ExecutionOptions {
    /// Check execution bounds without changing runtime state or contacting providers.
    pub fn validate(&self) -> Result<(), ExecutionError> {
        let field = if self.max_output_tokens == 0 || self.max_output_tokens > 131_072 {
            Some("max_output_tokens")
        } else if self
            .context_tokens
            .is_some_and(|tokens| !(64..=1_048_576).contains(&tokens))
        {
            Some("context_tokens")
        } else if self.timeout.is_zero() || self.timeout > Duration::from_secs(3600) {
            Some("timeout")
        } else if self
            .temperature
            .is_some_and(|value| !value.is_finite() || !(0.0..=2.0).contains(&value))
        {
            Some("temperature")
        } else if self.max_response_bytes == 0 || self.max_response_bytes > 8 * 1024 * 1024 {
            Some("max_response_bytes")
        } else {
            None
        };
        if let Some(field) = field {
            Err(ExecutionError::InvalidOptions { field })
        } else {
            Ok(())
        }
    }
}

/// Task execution outcomes are data, never permission grants or tool instructions.
#[derive(Debug, Clone, Serialize)]
pub struct TaskExecutionResult {
    /// Accepted terminal task snapshot.
    pub task: Task,
    /// Independent agent identity.
    pub agent_id: AgentId,
    /// Selected registry model identity.
    pub model_id: ModelId,
    /// Provider instance identity.
    pub provider_id: ProviderId,
    /// Unique scheduler admission identity for provenance.
    pub lease_id: String,
    /// Generated content and provider execution observations.
    pub response: InferenceResponse,
    /// Exact durable records included as untrusted user input.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub memory_context_ids: Vec<MemoryId>,
    /// Runtime-observed local elapsed milliseconds, including queue/preparation.
    pub elapsed_ms: u64,
}

/// Bounded task failures; errors never embed prompt or generated content.
#[derive(Debug, Error)]
pub enum ExecutionError {
    /// Agent/registry/scheduler rejected preparation or completion.
    #[error("{0}")]
    Runtime(#[from] AgentRuntimeError),
    /// Provider validation or I/O failed.
    #[error("{0}")]
    Provider(#[from] ProviderError),
    /// Invalid operator execution bounds.
    #[error("invalid execution option: {field}")]
    InvalidOptions {
        /// Field name, not its value.
        field: &'static str,
    },
    /// Caller/operator requested local cancellation.
    #[error("task execution cancelled; remote computation stop is unverified")]
    Cancelled,
    /// Absolute local task deadline expired.
    #[error("task execution deadline exceeded; remote computation stop is unverified")]
    DeadlineExceeded,
    /// Provider result violated identity, output or supported-result constraints.
    #[error("invalid task inference response: {reason}")]
    InvalidResponse {
        /// Sanitized reason with no provider payload.
        reason: &'static str,
    },
}

struct RunGuard {
    runtime: AgentRuntime,
    task_id: TaskId,
    inference: ScheduledInference,
    armed: bool,
}
impl RunGuard {
    fn finish(&mut self, state: TaskState) -> Result<Task, AgentRuntimeError> {
        let task = self.runtime.finish_task_execution_inner(
            self.task_id,
            self.inference.model_id,
            state,
            Some(&self.inference),
        )?;
        self.armed = false;
        Ok(task)
    }
}
impl Drop for RunGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.finish(TaskState::Cancelled);
        }
    }
}

impl AgentRuntime {
    /// Request cancellation of a managed invocation without prematurely releasing its lease.
    /// Returns false if the task exists but no managed invocation remains.
    pub async fn cancel_task(&self, task_id: TaskId) -> Result<bool, AgentRuntimeError> {
        let token = {
            let state = lock_state(&self.state);
            if !state.tasks.contains_key(&task_id) {
                return Err(AgentRuntimeError::TaskNotFound { task_id });
            }
            state.managed_tasks.get(&task_id).cloned()
        };
        if let Some(token) = token {
            let newly_requested = !token.is_cancelled();
            token.cancel();
            Ok(newly_requested)
        } else {
            Ok(false)
        }
    }

    /// Execute one real bounded provider request through the agent's selected model and lease.
    /// Output remains untrusted data. This operation runs no tools, modifies no permissions,
    /// and does not automatically inject or write agent memory.
    pub async fn execute_task(
        &self,
        agent_id: AgentId,
        task: Task,
        options: ExecutionOptions,
        cancel: CancellationToken,
    ) -> Result<TaskExecutionResult, ExecutionError> {
        self.execute_task_with_context(agent_id, task, options, None, cancel)
            .await
    }

    /// Execute with explicitly retrieved memory from the same authorized agent.
    pub async fn execute_task_with_context(
        &self,
        agent_id: AgentId,
        task: Task,
        options: ExecutionOptions,
        context: Option<MemoryContext>,
        cancel: CancellationToken,
    ) -> Result<TaskExecutionResult, ExecutionError> {
        options.validate()?;
        if cancel.is_cancelled() {
            return Err(ExecutionError::Cancelled);
        }
        let mut messages = Vec::new();
        let mut memory_context_ids = Vec::new();
        if let Some(context) = context {
            let agent = self.get_agent(agent_id).await?;
            if context.owner != agent_id || !agent.permissions.contains(&Permission::MemoryRead) {
                return Err(AgentRuntimeError::PermissionDenied {
                    agent_id,
                    permission: Permission::MemoryRead,
                }
                .into());
            }
            if let Some(content) = context.message {
                messages.push(ChatMessage {
                    role: ChatRole::User,
                    content,
                });
            }
            memory_context_ids = context.ids;
        }
        messages.push(ChatMessage {
            role: ChatRole::User,
            content: task.prompt.clone(),
        });
        let started = Instant::now();
        let deadline = started
            .checked_add(options.timeout)
            .ok_or(ExecutionError::InvalidOptions { field: "timeout" })?;
        let owned_cancel = CancellationToken::new();
        let work = async {
            let active = self
                .start_task_execution_inner(
                    agent_id,
                    task.clone(),
                    options.context_tokens,
                    owned_cancel.clone(),
                    Some((owned_cancel.clone(), &options, &messages)),
                )
                .await
                .map_err(|error| match error {
                    AgentRuntimeError::Scheduler(vespera_scheduler::SchedulerError::Provider(
                        error,
                    )) => ExecutionError::Provider(error),
                    error => ExecutionError::Runtime(error),
                })?;
            let mut guard = RunGuard {
                runtime: self.clone(),
                task_id: task.id,
                inference: active.inference.clone(),
                armed: true,
            };
            let outcome = async {
                let request = BoundedInferenceRequest::new(
                    active.inference.model_name(),
                    messages,
                    options.max_output_tokens,
                    active.inference.context_tokens,
                    options.temperature,
                    None,
                )?;
                let response = active
                    .inference
                    .provider
                    .infer(request, owned_cancel.clone())
                    .await?;
                if response.provider_id != active.inference.provider_id
                    || response.model_name != active.inference.model_name()
                {
                    return Err(ExecutionError::InvalidResponse {
                        reason: "provider/model identity mismatch",
                    });
                }
                if response.content.len() > options.max_response_bytes
                    || response
                        .metrics
                        .completion_tokens
                        .is_some_and(|tokens| tokens > options.max_output_tokens)
                {
                    return Err(ExecutionError::InvalidResponse {
                        reason: "response exceeded accepted output bound",
                    });
                }
                if matches!(
                    response.finish_reason,
                    InferenceFinishReason::ToolCalls | InferenceFinishReason::Unload
                ) {
                    return Err(ExecutionError::InvalidResponse {
                        reason: "result requires an unsupported execution action",
                    });
                }
                Ok(response)
            }
            .await;
            match outcome {
                Ok(response) => {
                    let task = guard.finish(TaskState::Completed)?;
                    Ok(TaskExecutionResult {
                        task,
                        agent_id,
                        model_id: active.inference.model_id,
                        provider_id: active.inference.provider_id,
                        lease_id: active.inference.lease_id().to_string(),
                        response,
                        memory_context_ids,
                        elapsed_ms: u64::try_from(started.elapsed().as_millis())
                            .unwrap_or(u64::MAX),
                    })
                }
                Err(error) => {
                    let terminal = if matches!(
                        error,
                        ExecutionError::Provider(ProviderError::Cancelled { .. })
                    ) {
                        TaskState::Cancelled
                    } else {
                        TaskState::Failed
                    };
                    guard.finish(terminal)?;
                    Err(error)
                }
            }
        };
        tokio::pin!(work);
        tokio::select! {
            biased;
            () = cancel.cancelled() => { owned_cancel.cancel(); Err(ExecutionError::Cancelled) },
            () = owned_cancel.cancelled() => Err(ExecutionError::Cancelled),
            () = tokio::time::sleep_until(deadline) => { owned_cancel.cancel(); Err(ExecutionError::DeadlineExceeded) },
            result = &mut work => result,
        }
    }
}
