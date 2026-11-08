use crate::{ConflictPolicy, OutputPolicy, PlanError, Workflow, WorkflowStore, WorkflowStoreError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Instant;
use vespera_agents::{AgentRuntime, ExecutionError, TaskExecutionResult};
use vespera_core::{
    AgentId, AgentStatus, EventKind, ModelId, Permission, Task, TaskId, TaskState,
    WorkflowDependency,
};
use vespera_providers::{CancellationToken, ProviderError};

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Bounded, payload-free failure categories. Generated text never becomes an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    NoCandidate,
    DelegationDenied,
    ExecutionFailed,
    AdmissionDenied,
    RejectedOutput,
    TransientRejection,
    AttemptBudget,
    OutputBudget,
    PromptLimit,
    Cancelled,
    Deadline,
    ConflictingOutputs,
    Storage,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub step: String,
    pub number: u32,
    pub task_id: TaskId,
    pub agent_id: AgentId,
    /// Routing snapshot; actual execution provenance is retained in `result`.
    pub candidate_model_id: ModelId,
    pub delegated: bool,
    pub task_state: Option<TaskState>,
    pub failure: Option<Failure>,
    pub result: Option<TaskExecutionResult>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowReport {
    pub task: Task,
    pub coordinator: AgentId,
    pub output_policy: OutputPolicy,
    pub attempts: Vec<Attempt>,
    pub reserved_output_tokens: u64,
    pub accepted_output_bytes: usize,
    pub selected_step: Option<String>,
    /// Names whose complete generated outputs disagreed under the chosen policy.
    pub conflicting_steps: Vec<String>,
    pub failure: Option<Failure>,
    pub elapsed_ms: u64,
}
impl WorkflowReport {
    /// Selected contribution only after successful policy evaluation. Conflicting
    /// contributions remain in attempts even when explicit precedence selects one.
    pub fn selected_result(&self) -> Option<&TaskExecutionResult> {
        if self.task.state != TaskState::Completed {
            return None;
        }
        let step = self.selected_step.as_ref()?;
        self.attempts
            .iter()
            .rev()
            .find(|attempt| &attempt.step == step && attempt.result.is_some())
            .and_then(|attempt| attempt.result.as_ref())
    }
}
#[derive(Debug, Error)]
pub enum OrchestratorError {
    #[error(transparent)]
    Store(#[from] WorkflowStoreError),
    #[error("{0}")]
    Plan(#[from] PlanError),
    #[error("coordinator must be registered, idle and explicitly permitted to delegate")]
    CoordinatorDenied,
    #[error("workflow identity already exists")]
    Duplicate,
    #[error("workflow history capacity reached; explicitly forget terminal history")]
    Capacity,
    #[error("running workflow history cannot be forgotten")]
    Running,
}

/// At most eight retained/running workflows. Each plan is capped at 48 attempts,
/// 64-KiB expanded prompts and one MiB total accepted generated content.
/// Caller-owned result clones are outside this logical retention bound.
#[derive(Clone)]
pub struct Orchestrator {
    agents: AgentRuntime,
    history: Arc<Mutex<BTreeMap<TaskId, Arc<Mutex<WorkflowReport>>>>>,
}
struct RunGuard<'a> {
    report: Arc<Mutex<WorkflowReport>>,
    cancel: CancellationToken,
    started: Instant,
    store: Option<&'a WorkflowStore>,
}
impl RunGuard<'_> {
    fn finish(&self, failure: Option<Failure>) -> WorkflowReport {
        let mut report = lock(&self.report);
        if report.task.state == TaskState::Running {
            let state = match failure {
                None => TaskState::Completed,
                Some(Failure::Cancelled | Failure::Deadline) => TaskState::Cancelled,
                _ => TaskState::Failed,
            };
            report
                .task
                .state
                .transition(state)
                .expect("legal workflow terminal edge");
            report.failure = failure;
            report.elapsed_ms =
                u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        }
        report.clone()
    }
}
impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        self.cancel.cancel();
        if lock(&self.report).task.state == TaskState::Running {
            let report = self.finish(Some(Failure::Cancelled));
            if let Some(store) = self.store {
                let _ = store.checkpoint(&report);
            }
        }
    }
}
struct AttemptGuard<'a> {
    agents: AgentRuntime,
    report: Arc<Mutex<WorkflowReport>>,
    index: usize,
    agent_id: AgentId,
    task_id: TaskId,
    armed: bool,
    observer: Option<&'a UnboundedSender<EventKind>>,
}
impl Drop for AttemptGuard<'_> {
    fn drop(&mut self) {
        // Covers cancellation between acceptance and the first execution poll.
        let _ = self.agents.cancel_queued_task(self.agent_id, self.task_id);
        if self.armed {
            let mut report = lock(&self.report);
            let attempt = &mut report.attempts[self.index];
            if attempt.delegated {
                attempt.task_state = self.agents.task_state(self.task_id);
                emit_finish(self.observer, self.task_id, attempt.task_state);
            }
            attempt.failure = Some(Failure::Cancelled);
        }
    }
}

fn emit(observer: Option<&UnboundedSender<EventKind>>, event: EventKind) {
    if let Some(observer) = observer {
        let _ = observer.send(event);
    }
}

fn emit_finish(
    observer: Option<&UnboundedSender<EventKind>>,
    task_id: TaskId,
    state: Option<TaskState>,
) {
    if let Some(state @ (TaskState::Completed | TaskState::Cancelled | TaskState::Failed)) = state {
        emit(
            observer,
            EventKind::WorkflowAttemptFinished { task_id, state },
        );
    }
}

impl Orchestrator {
    pub fn new(agents: AgentRuntime) -> Self {
        Self {
            agents,
            history: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
    pub fn report(&self, task_id: TaskId) -> Option<WorkflowReport> {
        let report = lock(&self.history).get(&task_id).cloned()?;
        Some(lock(&report).clone())
    }
    /// Explicit terminal-history removal never deletes agent task history or user data.
    pub fn forget(&self, task_id: TaskId) -> Result<bool, OrchestratorError> {
        let mut history = lock(&self.history);
        if let Some(report) = history.get(&task_id)
            && lock(report).task.state == TaskState::Running
        {
            return Err(OrchestratorError::Running);
        }
        Ok(history.remove(&task_id).is_some())
    }
    pub async fn execute(
        &self,
        workflow: Workflow,
        cancel: CancellationToken,
    ) -> Result<WorkflowReport, OrchestratorError> {
        self.execute_observed(workflow, cancel, None).await
    }

    /// Observe at most two payload-free events per admitted attempt, including cancellation.
    /// A disconnected receiver never changes workflow execution.
    pub async fn execute_observed(
        &self,
        workflow: Workflow,
        cancel: CancellationToken,
        observer: Option<&UnboundedSender<EventKind>>,
    ) -> Result<WorkflowReport, OrchestratorError> {
        self.execute_checkpointed(workflow, cancel, observer, None)
            .await
    }

    /// Persist an authorized plan, attempts, and results before advancing dependent work.
    pub async fn execute_checkpointed(
        &self,
        workflow: Workflow,
        cancel: CancellationToken,
        observer: Option<&UnboundedSender<EventKind>>,
        store: Option<&WorkflowStore>,
    ) -> Result<WorkflowReport, OrchestratorError> {
        workflow.validate()?;
        let coordinator = self
            .agents
            .get_agent(workflow.coordinator)
            .await
            .map_err(|_| OrchestratorError::CoordinatorDenied)?;
        if coordinator.status != AgentStatus::Idle
            || !coordinator.permissions.contains(&Permission::Delegate)
        {
            return Err(OrchestratorError::CoordinatorDenied);
        }
        let mut task = workflow.task.clone();
        task.state
            .transition(TaskState::Running)
            .expect("validated queued workflow");
        let report = Arc::new(Mutex::new(WorkflowReport {
            task,
            coordinator: workflow.coordinator,
            output_policy: workflow.output.clone(),
            attempts: Vec::new(),
            reserved_output_tokens: 0,
            accepted_output_bytes: 0,
            selected_step: None,
            conflicting_steps: Vec::new(),
            failure: None,
            elapsed_ms: 0,
        }));
        {
            let mut history = lock(&self.history);
            if history.contains_key(&workflow.task.id) {
                return Err(OrchestratorError::Duplicate);
            }
            if history.len() >= 8 {
                return Err(OrchestratorError::Capacity);
            }
            if let Some(store) = store {
                store.begin(&workflow, &lock(&report))?;
            }
            history.insert(workflow.task.id, report.clone());
        }
        let owned_cancel = CancellationToken::new();
        let guard = RunGuard {
            report: report.clone(),
            cancel: owned_cancel.clone(),
            started: Instant::now(),
            store,
        };
        let failure = {
            let work = self.run_steps(&workflow, &report, &owned_cancel, observer, store);
            tokio::pin!(work);
            tokio::select! {
                biased;
                () = cancel.cancelled() => Some(Failure::Cancelled),
                () = tokio::time::sleep(Duration::from_millis(workflow.timeout_ms)) => Some(Failure::Deadline),
                result = &mut work => result.err(),
            }
        };
        let terminal = guard.finish(failure);
        if let Some(store) = store {
            store.checkpoint(&terminal)?;
        }
        Ok(terminal)
    }

    async fn run_steps(
        &self,
        workflow: &Workflow,
        report: &Arc<Mutex<WorkflowReport>>,
        cancel: &CancellationToken,
        observer: Option<&UnboundedSender<EventKind>>,
        store: Option<&WorkflowStore>,
    ) -> Result<(), Failure> {
        let mut completed: BTreeMap<String, usize> = BTreeMap::new();
        for step in &workflow.steps {
            let prompt = {
                let record = lock(report);
                let dependencies: Vec<_> = step.dependencies.iter().map(|name| {
                    let result = record.attempts[completed[name]].result.as_ref().expect("completed dependency");
                    serde_json::json!({ "step": name, "task_id": result.task.id, "agent_id": result.agent_id, "model_id": result.model_id, "content": result.response.content })
                }).collect();
                serde_json::json!({ "workflow_request": workflow.task.prompt, "step_instructions": step.prompt, "untrusted_dependency_outputs": dependencies }).to_string()
            };
            if prompt.len() > 64 * 1024 {
                return Err(Failure::PromptLimit);
            }
            let mut succeeded = false;
            for number in 1..=step.max_attempts {
                if cancel.is_cancelled() {
                    return Err(Failure::Cancelled);
                }
                let (attempt_count, reserved_tokens, output_bytes) = {
                    let record = lock(report);
                    (
                        record.attempts.len(),
                        record.reserved_output_tokens,
                        record.accepted_output_bytes,
                    )
                };
                if attempt_count >= workflow.max_total_attempts as usize {
                    return Err(Failure::AttemptBudget);
                }
                if reserved_tokens + u64::from(step.max_output_tokens)
                    > workflow.max_total_output_tokens
                    || output_bytes >= 1024 * 1024
                {
                    return Err(Failure::OutputBudget);
                }
                self.agents
                    .refresh_candidate_health(
                        &step.candidates,
                        &step.required_capabilities,
                        cancel.clone(),
                    )
                    .await
                    .map_err(|_| Failure::NoCandidate)?;
                if cancel.is_cancelled() {
                    return Err(Failure::Cancelled);
                }
                let mut options = step.options();
                options.max_response_bytes = 1024 * 1024 - output_bytes;
                let mut chosen = None;
                for agent_id in &step.candidates {
                    if self
                        .agents
                        .get_agent(*agent_id)
                        .await
                        .is_ok_and(|a| a.status == AgentStatus::Idle)
                        && let Ok(model) = self
                            .agents
                            .resolve_model_for_execution(
                                *agent_id,
                                &step.required_capabilities,
                                &options,
                            )
                            .await
                    {
                        chosen = Some((*agent_id, model.model_id));
                        break;
                    }
                }
                let (agent_id, candidate_model_id) = chosen.ok_or(Failure::NoCandidate)?;
                let task = Task {
                    id: TaskId::new(),
                    prompt: prompt.clone(),
                    required_capabilities: step.required_capabilities.clone(),
                    state: TaskState::Queued,
                };
                let index = {
                    let mut record = lock(report);
                    record.reserved_output_tokens += u64::from(step.max_output_tokens);
                    let index = record.attempts.len();
                    record.attempts.push(Attempt {
                        step: step.name.clone(),
                        number,
                        task_id: task.id,
                        agent_id,
                        candidate_model_id,
                        delegated: false,
                        task_state: None,
                        failure: None,
                        result: None,
                    });
                    index
                };
                checkpoint(store, report)?;
                let mut guard = AttemptGuard {
                    agents: self.agents.clone(),
                    report: report.clone(),
                    index,
                    agent_id,
                    task_id: task.id,
                    armed: true,
                    observer,
                };
                if self
                    .agents
                    .delegate_task(workflow.coordinator, agent_id, task.clone())
                    .await
                    .is_err()
                {
                    lock(report).attempts[index].failure = Some(Failure::DelegationDenied);
                    guard.armed = false;
                    return Err(Failure::DelegationDenied);
                }
                lock(report).attempts[index].delegated = true;
                checkpoint(store, report)?;
                if let Some(observer) = observer {
                    let record = lock(report);
                    let dependencies = step
                        .dependencies
                        .iter()
                        .map(|name| {
                            let result = record.attempts[completed[name]]
                                .result
                                .as_ref()
                                .expect("completed dependency");
                            WorkflowDependency {
                                task_id: result.task.id,
                                agent_id: result.agent_id,
                            }
                        })
                        .collect();
                    emit(
                        Some(observer),
                        EventKind::WorkflowAttemptAccepted {
                            task_id: task.id,
                            from_agent_id: workflow.coordinator,
                            to_agent_id: agent_id,
                            dependencies,
                        },
                    );
                }
                let result = self
                    .agents
                    .execute_task(agent_id, task, options, cancel.clone())
                    .await;
                let _ = self.agents.cancel_queued_task(agent_id, guard.task_id);
                let state = self.agents.task_state(guard.task_id);
                guard.armed = false;
                emit_finish(observer, guard.task_id, state);
                match result {
                    Ok(result) => {
                        let rejected = result.response.content.trim().is_empty();
                        {
                            let mut record = lock(report);
                            record.accepted_output_bytes += result.response.content.len();
                            record.attempts[index].task_state = state;
                            record.attempts[index].result = Some(result);
                            if rejected {
                                record.attempts[index].failure = Some(Failure::RejectedOutput);
                            }
                        }
                        checkpoint(store, report)?;
                        if rejected {
                            return Err(Failure::RejectedOutput);
                        }
                        completed.insert(step.name.clone(), index);
                        succeeded = true;
                        break;
                    }
                    Err(error) => {
                        let failure = match &error {
                            ExecutionError::Cancelled => Failure::Cancelled,
                            ExecutionError::DeadlineExceeded => Failure::Deadline,
                            ExecutionError::Runtime(
                                vespera_agents::AgentRuntimeError::Scheduler(_),
                            ) => Failure::AdmissionDenied,
                            // Only explicit busy/rate-limit rejections are retried. A timeout
                            // may leave remote work running and is never retried here.
                            ExecutionError::Provider(ProviderError::HttpStatus {
                                status_code: 429 | 503,
                                ..
                            }) => Failure::TransientRejection,
                            _ => Failure::ExecutionFailed,
                        };
                        {
                            let mut record = lock(report);
                            record.attempts[index].task_state = state;
                            record.attempts[index].failure = Some(failure);
                        }
                        checkpoint(store, report)?;
                        if failure != Failure::TransientRejection || number == step.max_attempts {
                            return Err(failure);
                        }
                        // Retry routing considers remaining idle candidates; never resume a
                        // failed/suspended identity or silently grant it permissions.
                    }
                }
            }
            if !succeeded {
                return Err(Failure::ExecutionFailed);
            }
        }
        let mut record = lock(report);
        let selected = match &workflow.output {
            OutputPolicy::Step { step } => step.clone(),
            OutputPolicy::Agreement { steps, on_conflict } => {
                let first = &record.attempts[completed[&steps[0]]]
                    .result
                    .as_ref()
                    .expect("completed step")
                    .response
                    .content;
                let conflict = steps.iter().skip(1).any(|name| {
                    record.attempts[completed[name]]
                        .result
                        .as_ref()
                        .expect("completed step")
                        .response
                        .content
                        != *first
                });
                if conflict {
                    record.conflicting_steps = steps.clone();
                    match on_conflict {
                        ConflictPolicy::Fail => return Err(Failure::ConflictingOutputs),
                        ConflictPolicy::Prefer { step } => step.clone(),
                    }
                } else {
                    steps[0].clone()
                }
            }
        };
        record.selected_step = Some(selected);
        Ok(())
    }
}

fn checkpoint(
    store: Option<&WorkflowStore>,
    report: &Mutex<WorkflowReport>,
) -> Result<(), Failure> {
    if let Some(store) = store {
        store
            .checkpoint(&lock(report))
            .map_err(|_| Failure::Storage)?;
    }
    Ok(())
}
