//! Compose validated configuration into real providers, registry, scheduling and agents.
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use vespera_agents::{
    AgentRuntime, AgentRuntimeError, ExecutionError, ExecutionOptions, MemoryContext,
    TaskExecutionResult,
};
use vespera_config::{AppConfig, ConfigError};
use vespera_core::{
    AgentId, AgentStatus, ModelAvailability, ModelId, ModelSelectionPolicy, Permission, ProviderId,
    ProviderKind, Task,
};
use vespera_model_registry::{
    ModelRegistration, ModelRegistry, ProviderHealthState, RegistryError,
};
use vespera_orchestrator::{Orchestrator, OrchestratorError, Workflow, WorkflowReport};
use vespera_providers::{
    CancellationToken, EndpointPolicy, InferenceProvider, OllamaConfig, OllamaProvider,
    OpenAiCompatibleProvider, OpenAiConfig, ProviderError, ProviderHealthStatus, SecretRef,
    ValidatedEndpoint,
};
use vespera_scheduler::{ResourceScheduler, SchedulerError, SchedulingPolicy};

/// Application errors deliberately exclude configuration endpoints/credential values.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// The optional startup residency guard could not establish safe accounting.
    #[error("startup residency reconciliation failed: {0}")]
    StartupResidency(SchedulerError),
    #[error("{0}")]
    Orchestration(#[from] OrchestratorError),
    /// Invalid offline declarations.
    #[error("{0}")]
    Config(#[from] ConfigError),
    /// Provider construction/protocol failure.
    #[error("{0}")]
    Provider(#[from] ProviderError),
    /// Registry mutation failure.
    #[error("{0}")]
    Registry(#[from] RegistryError),
    /// Agent control-plane failure.
    #[error("{0}")]
    Agent(#[from] AgentRuntimeError),
    /// Actual task execution failed.
    #[error("{0}")]
    Execution(#[from] ExecutionError),
    /// A declared category has no installed adapter.
    #[error("provider {provider_id} uses a category without an installed adapter")]
    UnsupportedProvider {
        /// Provider identity.
        provider_id: ProviderId,
    },
    /// Suspended/failed agents require explicit operator activation for this process.
    #[error("agent {agent_id} is not idle; use --resume to activate it for this invocation")]
    ActivationRequired {
        /// Agent identity.
        agent_id: AgentId,
    },
    /// A bounded operation was interrupted locally.
    #[error("operation cancelled; remote computation stop is unverified")]
    Cancelled,
    /// Overall command budget expired.
    #[error("operation deadline exceeded; remote computation stop is unverified")]
    Deadline,
    /// Failed to install/read an OS cancellation signal.
    #[error("could not receive the operating-system cancellation signal")]
    Signal,
}

/// Current expiring health evidence, without endpoint or credential material.
#[derive(Debug, Serialize)]
pub struct ProviderCheck {
    /// Configured provider identity.
    pub provider_id: ProviderId,
    /// Effective state, including unknown/stale observations.
    pub state: ProviderHealthState,
    /// Age of the last completed poll in local monotonic milliseconds.
    pub checked_age_ms: Option<u64>,
}

/// Explicit health checks; no agent registration or model activation is performed.
pub async fn check_providers(
    config: &AppConfig,
    cancel: CancellationToken,
) -> Result<Vec<ProviderCheck>, RuntimeError> {
    let registry = configured_registry(config).await?;
    registry.health_check_all(cancel.clone()).await;
    if cancel.is_cancelled() {
        return Err(RuntimeError::Cancelled);
    }
    observed_provider_checks(config, &registry).await
}

/// Read current registry evidence in configured order without provider I/O.
pub async fn observed_provider_checks(
    config: &AppConfig,
    registry: &ModelRegistry,
) -> Result<Vec<ProviderCheck>, RuntimeError> {
    let mut checks = Vec::new();
    for provider in &config.providers {
        let observation = registry.provider_health(provider.id).await?;
        checks.push(ProviderCheck {
            provider_id: provider.id,
            state: observation.state,
            checked_age_ms: observation
                .checked_at
                .map(|instant| u64::try_from(instant.elapsed().as_millis()).unwrap_or(u64::MAX)),
        });
    }
    Ok(checks)
}

/// Build adapter objects and registration metadata without sending provider requests.
/// Constructors resolve configured credential references. Offline CLI inventory bypasses this.
pub async fn configured_registry(config: &AppConfig) -> Result<Arc<ModelRegistry>, RuntimeError> {
    config.validate()?;
    let registry = Arc::new(ModelRegistry::new());
    for declared in &config.providers {
        let endpoint = ValidatedEndpoint::parse(
            declared.id,
            &declared.endpoint,
            EndpointPolicy {
                allow_remote_https: config.network.allow_remote,
                allow_insecure_remote_http: config.network.allow_insecure_http,
            },
        )?;
        let auth_secret_ref = declared
            .credential_env
            .as_ref()
            .map(|name| SecretRef::new(declared.id, name))
            .transpose()?;
        let timeout = Duration::from_secs(u64::from(declared.request_timeout_seconds));
        let provider: Arc<dyn InferenceProvider> = match declared.kind {
            ProviderKind::Ollama => Arc::new(OllamaProvider::new(OllamaConfig {
                provider_id: declared.id,
                endpoint,
                auth_secret_ref,
                pre_resolved_secret: None,
                connect_timeout: timeout.min(Duration::from_secs(5)),
                request_timeout: timeout,
                stream_idle_timeout: timeout.min(Duration::from_secs(30)),
                stream_total_timeout: timeout,
            })?),
            ProviderKind::OpenAiCompatible => {
                Arc::new(OpenAiCompatibleProvider::new(OpenAiConfig {
                    provider_id: declared.id,
                    endpoint,
                    auth_secret_ref,
                    pre_resolved_secret: None,
                    connect_timeout: timeout.min(Duration::from_secs(5)),
                    request_timeout: timeout,
                    stream_idle_timeout: timeout.min(Duration::from_secs(30)),
                    stream_total_timeout: timeout,
                })?)
            }
            ProviderKind::Custom => {
                return Err(RuntimeError::UnsupportedProvider {
                    provider_id: declared.id,
                });
            }
        };
        registry.register_provider(provider).await?;
    }
    for model in &config.models {
        registry
            .register_model(ModelRegistration {
                model_id: model.id,
                provider_id: model.provider_id,
                name: model.name.clone(),
                provider_model_identifier: model.identifier.clone(),
                parameter_size: model
                    .parameter_count
                    .map(|count| format!("{count} parameters (declared)")),
                quantization: model.quantization.clone(),
                context_window: model.context_window_tokens,
                capabilities: model.capabilities.clone(),
                estimated_resources: model.resources,
                available: model.availability != ModelAvailability::Unavailable,
            })
            .await?;
    }
    Ok(registry)
}

/// Explicitly owned application services, initially containing no scheduler activations.
pub struct VesperaRuntime {
    registry: Arc<ModelRegistry>,
    scheduler: Arc<ResourceScheduler>,
    agents: AgentRuntime,
    orchestrator: Orchestrator,
}
/// A charged model reservation, separate from measured provider residency.
#[derive(Debug, Serialize)]
pub struct ReservationSummary {
    /// Registered model identity.
    pub model_id: ModelId,
    /// Provider lifecycle knowledge.
    pub state: String,
    /// Accounted estimated RAM.
    pub reserved_ram_bytes: u64,
    /// Accounted estimated VRAM.
    pub reserved_vram_bytes: u64,
}
/// Shutdown evidence, including unsupported or uncertain unloads.
#[derive(Debug, Serialize)]
pub struct CleanupReport {
    /// Whether all local admission permits drained.
    pub drained: bool,
    /// Successfully acknowledged unloads.
    pub unloaded_models: Vec<ModelId>,
    /// Reservations whose provider memory was not confirmed freed.
    pub remaining_models: Vec<ReservationSummary>,
    /// Sanitized per-model lifecycle failures.
    pub unload_errors: BTreeMap<ModelId, String>,
}
impl VesperaRuntime {
    /// Compose only a workflow's named agents and matching worker inventory.
    /// The coordinator's model is unnecessary unless it is also a step candidate.
    pub async fn from_config_for_workflow(
        config: &AppConfig,
        workflow: &Workflow,
        resume: bool,
    ) -> Result<Self, RuntimeError> {
        config.validate()?;
        workflow.validate().map_err(OrchestratorError::from)?;
        let ids: BTreeSet<_> = workflow
            .steps
            .iter()
            .flat_map(|step| step.candidates.iter().copied())
            .chain([workflow.coordinator])
            .collect();
        if ids.len() > 64 {
            return Err(AgentRuntimeError::Capacity { resource: "agents" }.into());
        }
        let mut selected = Vec::new();
        for id in &ids {
            let mut agent = config
                .agents
                .iter()
                .find(|a| a.identity.id == *id)
                .cloned()
                .ok_or(AgentRuntimeError::AgentNotFound { agent_id: *id })?;
            if agent.status == AgentStatus::Running {
                return Err(AgentRuntimeError::InvalidAgentState {
                    agent_id: *id,
                    status: agent.status,
                }
                .into());
            }
            if *id == workflow.coordinator {
                if !agent.permissions.contains(&Permission::Delegate) {
                    return Err(OrchestratorError::CoordinatorDenied.into());
                }
                if agent.status != AgentStatus::Idle && !resume {
                    return Err(RuntimeError::ActivationRequired { agent_id: *id });
                }
            }
            if resume && agent.status != AgentStatus::Idle {
                agent.status = AgentStatus::Idle;
            }
            selected.push(agent);
        }
        let mut scoped = config.clone();
        scoped.agents.clear();
        scoped.models.retain(|model| {
            model.availability != ModelAvailability::Unavailable
                && workflow.steps.iter().any(|step| {
                    step.required_capabilities.is_subset(&model.capabilities)
                        && selected.iter().any(|agent| {
                            agent.status == AgentStatus::Idle
                                && step.candidates.contains(&agent.identity.id)
                                && match &agent.model_policy {
                                    ModelSelectionPolicy::Fixed { model_id } => {
                                        *model_id == model.id
                                    }
                                    ModelSelectionPolicy::Capability { required } => {
                                        required.is_subset(&model.capabilities)
                                    }
                                }
                        })
                })
        });
        let providers: BTreeSet<_> = scoped
            .models
            .iter()
            .map(|model| model.provider_id)
            .collect();
        scoped
            .providers
            .retain(|provider| providers.contains(&provider.id));
        let runtime = Self::from_config(&scoped).await?;
        for agent in selected {
            runtime.agents.register_agent(agent).await?;
        }
        Ok(runtime)
    }
    /// Compose one agent and its candidate inventory without initializing unrelated credentials.
    /// The complete source configuration is still validated before selecting the inventory.
    pub async fn from_config_for_agent(
        config: &AppConfig,
        agent_id: AgentId,
    ) -> Result<Self, RuntimeError> {
        Self::from_config_for_agents(config, &[agent_id]).await
    }

    /// Compose an explicit roster with a single resource scheduler and provider registry.
    pub async fn from_config_for_agents(
        config: &AppConfig,
        agent_ids: &[AgentId],
    ) -> Result<Self, RuntimeError> {
        config.validate()?;
        let mut scoped = config.clone();
        scoped.agents.clear();
        let mut models = BTreeSet::new();
        for &agent_id in agent_ids {
            let agent = config
                .agents
                .iter()
                .find(|agent| agent.identity.id == agent_id)
                .ok_or(AgentRuntimeError::AgentNotFound { agent_id })?;
            let eligible = config
                .models
                .iter()
                .filter(|model| {
                    model.availability != ModelAvailability::Unavailable
                        && match &agent.model_policy {
                            ModelSelectionPolicy::Fixed { model_id } => model.id == *model_id,
                            ModelSelectionPolicy::Capability { required } => {
                                required.is_subset(&model.capabilities)
                            }
                        }
                })
                .map(|model| model.id)
                .collect::<Vec<_>>();
            if eligible.is_empty() {
                return Err(AgentRuntimeError::NoMatchingModel { agent_id }.into());
            }
            models.extend(eligible);
            scoped.agents.push(agent.clone());
        }
        scoped.models.retain(|model| models.contains(&model.id));
        let providers: BTreeSet<_> = scoped
            .models
            .iter()
            .map(|model| model.provider_id)
            .collect();
        scoped
            .providers
            .retain(|provider| providers.contains(&provider.id));
        Self::from_config(&scoped).await
    }
    /// Construct without network requests or model activation.
    pub async fn from_config(config: &AppConfig) -> Result<Self, RuntimeError> {
        let registry = configured_registry(config).await?;
        let scheduler = Arc::new(ResourceScheduler::new_with_overhead(
            registry.clone(),
            SchedulingPolicy {
                queue: Default::default(),
                budget: config.resources,
                default_keep_alive: Duration::from_secs(300),
                evict_idle_on_pressure: true,
            },
            config.inference_overhead,
        ));
        let agents = AgentRuntime::new(registry.clone(), scheduler.clone());
        for agent in &config.agents {
            agents.register_agent(agent.clone()).await?;
        }
        let orchestrator = Orchestrator::new(agents.clone());
        Ok(Self {
            registry,
            scheduler,
            agents,
            orchestrator,
        })
    }
    /// Access the trusted agent control plane.
    pub fn agents(&self) -> &AgentRuntime {
        &self.agents
    }
    pub fn orchestrator(&self) -> &Orchestrator {
        &self.orchestrator
    }

    /// Bounded workflow execution with relevant health refreshed before each attempt.
    pub async fn run_workflow(
        &self,
        workflow: Workflow,
        cancel: CancellationToken,
    ) -> Result<WorkflowReport, RuntimeError> {
        self.run_workflow_observed(workflow, cancel, None).await
    }

    /// Run with a bounded stream of payload-free workflow attempt observations.
    pub async fn run_workflow_observed(
        &self,
        workflow: Workflow,
        cancel: CancellationToken,
        observer: Option<&tokio::sync::mpsc::UnboundedSender<vespera_core::EventKind>>,
    ) -> Result<WorkflowReport, RuntimeError> {
        workflow.validate().map_err(OrchestratorError::from)?;
        let coordinator = self.agents.get_agent(workflow.coordinator).await?;
        if coordinator.status != AgentStatus::Idle
            || !coordinator.permissions.contains(&Permission::Delegate)
        {
            return Err(OrchestratorError::CoordinatorDenied.into());
        }
        if cancel.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        Ok(self
            .orchestrator
            .execute_observed(workflow, cancel, observer)
            .await?)
    }
    /// Access the registry; lookups alone do not establish provider health.
    pub fn registry(&self) -> &Arc<ModelRegistry> {
        &self.registry
    }
    /// Access local reservation accounting.
    pub fn scheduler(&self) -> &Arc<ResourceScheduler> {
        &self.scheduler
    }

    /// Establish external memory pressure without taking ownership of resident models.
    pub async fn reconcile_startup(&self, cancel: CancellationToken) -> Result<(), RuntimeError> {
        self.scheduler
            .reconcile_startup(cancel)
            .await
            .map_err(RuntimeError::StartupResidency)
    }

    /// Capture startup residency using a durable local lifecycle journal.
    pub async fn reconcile_startup_with_journal(
        &self,
        path: &std::path::Path,
        acknowledge: bool,
        cancel: CancellationToken,
    ) -> Result<(), RuntimeError> {
        self.scheduler
            .reconcile_startup_with_journal(path, acknowledge, cancel)
            .await
            .map_err(RuntimeError::StartupResidency)
    }
    /// Explicit provider health I/O without model loading.
    pub async fn refresh_health(
        &self,
        cancel: CancellationToken,
    ) -> BTreeMap<ProviderId, ProviderHealthStatus> {
        self.registry.health_check_all(cancel).await
    }
    /// Run an agent once; optional activation changes only this in-process runtime.
    pub async fn run_agent(
        &self,
        agent_id: AgentId,
        task: Task,
        options: ExecutionOptions,
        resume: bool,
        cancel: CancellationToken,
    ) -> Result<TaskExecutionResult, RuntimeError> {
        self.run_agent_with_context(agent_id, task, options, resume, None, cancel)
            .await
    }

    /// Run an agent with an optional authorized durable memory snapshot.
    pub async fn run_agent_with_context(
        &self,
        agent_id: AgentId,
        task: Task,
        options: ExecutionOptions,
        resume: bool,
        context: Option<MemoryContext>,
        cancel: CancellationToken,
    ) -> Result<TaskExecutionResult, RuntimeError> {
        options.validate()?;
        let agent = self.agents.get_agent(agent_id).await?;
        if agent.status != AgentStatus::Idle {
            if !resume {
                return Err(RuntimeError::ActivationRequired { agent_id });
            }
            self.agents
                .set_agent_status(agent_id, AgentStatus::Idle)
                .await?;
        }
        let providers: BTreeSet<_> = self
            .registry
            .list_registrations()
            .await
            .into_iter()
            .filter(|model| {
                model.available
                    && task.required_capabilities.is_subset(&model.capabilities)
                    && match &agent.model_policy {
                        ModelSelectionPolicy::Fixed { model_id } => model.model_id == *model_id,
                        ModelSelectionPolicy::Capability { required } => {
                            required.is_subset(&model.capabilities)
                        }
                    }
            })
            .map(|model| model.provider_id)
            .collect();
        self.registry
            .health_check_providers(&providers, cancel.clone())
            .await;
        if cancel.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        Ok(self
            .agents
            .execute_task_with_context(agent_id, task, options, context, cancel)
            .await?)
    }
    /// Stop admissions and optionally request explicit unload of this runtime's idle models.
    /// Unloading is opt-in because providers may be shared with other applications.
    pub async fn cleanup(&self, unload: bool, timeout: Duration) -> CleanupReport {
        let deadline = tokio::time::Instant::now().checked_add(timeout);
        let drained = self.scheduler.shutdown(timeout).await;
        let mut unloaded_models = Vec::new();
        let mut unload_errors = BTreeMap::new();
        if unload && let Some(deadline) = deadline {
            for model in self.scheduler.resident_models().await {
                let operation = self
                    .scheduler
                    .evict_model(model.model_id, CancellationToken::new());
                match tokio::time::timeout_at(deadline, operation).await {
                    Ok(Ok(true)) => unloaded_models.push(model.model_id),
                    Ok(Ok(false)) => {
                        unload_errors.insert(
                            model.model_id,
                            "model still has active local inference".into(),
                        );
                    }
                    Ok(Err(error)) => {
                        unload_errors.insert(model.model_id, error.to_string());
                    }
                    Err(_) => {
                        unload_errors.insert(
                            model.model_id,
                            "unload deadline exceeded; provider state uncertain".into(),
                        );
                    }
                }
            }
        }
        let remaining_models = self
            .scheduler
            .resident_models()
            .await
            .into_iter()
            .map(|model| ReservationSummary {
                model_id: model.model_id,
                state: format!("{:?}", model.residency),
                reserved_ram_bytes: model.ram_bytes,
                reserved_vram_bytes: model.vram_bytes,
            })
            .collect();
        CleanupReport {
            drained,
            unloaded_models,
            remaining_models,
            unload_errors,
        }
    }
}
