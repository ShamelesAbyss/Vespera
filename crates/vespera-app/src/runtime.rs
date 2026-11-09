//! Compose validated configuration into real providers, registry, scheduling and agents.
pub use crate::host_limits::{LocalHostLimits, NvidiaGpuMemory};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use vespera_agents::{
    AgentRuntime, AgentRuntimeError, ExecutionError, ExecutionOptions, MemoryContext,
    TaskExecutionResult,
};
use vespera_config::{AppConfig, ConfigError, ProviderConfig};
use vespera_core::{
    AgentId, AgentStatus, ModelAvailability, ModelId, Permission, ProviderId, ProviderKind, Task,
};
use vespera_model_registry::{
    ModelRegistration, ModelRegistry, ProviderHealthState, RegistryError,
};
use vespera_orchestrator::{
    DecompositionError, DecompositionRequest, Orchestrator, OrchestratorError, WorkerCapabilities,
    Workflow, WorkflowReport, WorkflowStore,
};
use vespera_providers::{
    CancellationToken, EndpointPolicy, InferenceProvider, OllamaConfig, OllamaProvider,
    OpenAiCompatibleProvider, OpenAiConfig, ProviderError, ProviderHealthStatus, SecretRef,
    SecretString, ValidatedEndpoint,
};
use vespera_scheduler::{
    QueuePolicy, ResourceScheduler, SchedulerError, SchedulingPolicy, SchedulingPreferences,
};

/// Application errors deliberately exclude configuration endpoints/credential values.
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Decomposition(#[from] DecompositionError),
    /// Process host measurements cannot represent a remote provider's machine.
    #[error("local host limits require loopback providers")]
    LocalHostLimitsRequireLoopback,
    /// Opt-in host limits require positive observed RAM and CPU capacity.
    #[error("local host limits could not establish positive RAM and CPU capacity")]
    LocalHostLimitsUnavailable,
    /// The GPU declaration applies only to a single native loopback Ollama provider.
    #[error("Ollama GPU limits require a single native Ollama provider in the runtime scope")]
    OllamaGpuScopeRequired,
    /// The declared device could not provide valid NVML evidence.
    #[error("could not observe VRAM for the configured Ollama GPU")]
    OllamaGpuLimitsUnavailable,
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
    /// The platform credential store cannot supply the selected secret.
    #[error("provider {provider_id} could not read its Windows credential")]
    WindowsCredentialUnavailable { provider_id: ProviderId },
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
        let endpoint = configured_endpoint(config, declared)?;
        let auth_secret_ref = declared
            .credential_env
            .as_ref()
            .map(|name| SecretRef::new(declared.id, name))
            .transpose()?;
        let pre_resolved_secret = if declared.credential_windows {
            Some(windows_credential(declared)?)
        } else {
            None
        };
        let timeout = Duration::from_secs(u64::from(declared.request_timeout_seconds));
        let provider: Arc<dyn InferenceProvider> = match declared.kind {
            ProviderKind::Ollama => Arc::new(OllamaProvider::new(OllamaConfig {
                provider_id: declared.id,
                endpoint,
                auth_secret_ref,
                pre_resolved_secret,
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
                    pre_resolved_secret,
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

/// A Windows generic credential target for the exact adapter and endpoint.
pub fn windows_credential_target(provider: &ProviderConfig) -> String {
    use std::fmt::Write as _;

    let adapter = match provider.kind {
        ProviderKind::Ollama => "ollama",
        ProviderKind::OpenAiCompatible => "openai-compatible",
        ProviderKind::Custom => "custom",
    };
    // Windows compares credential targets case-insensitively. URL paths can be case-sensitive.
    let digest = ring::digest::digest(&ring::digest::SHA256, provider.endpoint.as_bytes());
    let mut target = format!("Vespera/provider/{adapter}/");
    for byte in digest.as_ref() {
        write!(target, "{byte:02x}").expect("writing to a String cannot fail");
    }
    target
}

fn windows_credential(provider: &ProviderConfig) -> Result<SecretString, RuntimeError> {
    let unavailable = RuntimeError::WindowsCredentialUnavailable {
        provider_id: provider.id,
    };
    #[cfg(target_os = "windows")]
    {
        let value = keyring::Entry::new_with_target(
            &windows_credential_target(provider),
            "Vespera",
            "provider",
        )
        .and_then(|entry| entry.get_password())
        .map_err(|_| RuntimeError::WindowsCredentialUnavailable {
            provider_id: provider.id,
        })?;
        if !value.trim().is_empty() {
            return Ok(SecretString::new_in_memory(value));
        }
    }
    Err(unavailable)
}

/// Explicitly owned application services, initially containing no scheduler activations.
pub struct VesperaRuntime {
    registry: Arc<ModelRegistry>,
    scheduler: Arc<ResourceScheduler>,
    agents: AgentRuntime,
    orchestrator: Orchestrator,
    local_host_limits: Option<LocalHostLimits>,
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
fn scope_inventory(config: &mut AppConfig) {
    let providers: BTreeSet<_> = config
        .models
        .iter()
        .map(|model| model.provider_id)
        .collect();
    config
        .providers
        .retain(|provider| providers.contains(&provider.id));
    let models: BTreeSet<_> = config.models.iter().map(|model| model.id).collect();
    config
        .scheduling
        .model_priorities
        .retain(|model, _| models.contains(model));
}

fn configured_endpoint(
    config: &AppConfig,
    provider: &ProviderConfig,
) -> Result<ValidatedEndpoint, ProviderError> {
    ValidatedEndpoint::parse(
        provider.id,
        &provider.endpoint,
        EndpointPolicy {
            allow_remote_https: config.network.allow_remote,
            allow_insecure_remote_http: config.network.allow_insecure_http,
        },
    )
}

impl VesperaRuntime {
    /// Compose only the planning coordinator and the explicitly authorized roster.
    pub async fn from_config_for_decomposition(
        config: &AppConfig,
        request: &DecompositionRequest,
        resume: bool,
    ) -> Result<Self, RuntimeError> {
        request.validate()?;
        let coordinator = config
            .agents
            .iter()
            .find(|agent| agent.identity.id == request.coordinator)
            .ok_or(OrchestratorError::CoordinatorDenied)?;
        if !coordinator.permissions.contains(&Permission::Delegate) {
            return Err(OrchestratorError::CoordinatorDenied.into());
        }
        let mut roster = request.workers.clone();
        if !roster.contains(&request.coordinator) {
            roster.push(request.coordinator);
        }
        let mut scoped = config.clone();
        for id in &roster {
            let agent = scoped
                .agents
                .iter_mut()
                .find(|agent| agent.identity.id == *id)
                .ok_or(AgentRuntimeError::AgentNotFound { agent_id: *id })?;
            if agent.status != AgentStatus::Idle {
                if !resume {
                    return Err(RuntimeError::ActivationRequired { agent_id: *id });
                }
                agent.status = AgentStatus::Idle;
            }
        }
        Self::from_config_for_agents(&scoped, &roster).await
    }

    /// Execute one bounded planning call and accept only a validated proposal.
    pub async fn decompose_workflow(
        &self,
        request: &DecompositionRequest,
        cancel: CancellationToken,
    ) -> Result<(Workflow, TaskExecutionResult), RuntimeError> {
        request.validate()?;
        let coordinator = self.agents.get_agent(request.coordinator).await?;
        if coordinator.status != AgentStatus::Idle
            || !coordinator.permissions.contains(&Permission::Delegate)
        {
            return Err(OrchestratorError::CoordinatorDenied.into());
        }
        let models = self.registry.list_registrations().await;
        let mut profiles = Vec::with_capacity(request.workers.len());
        for id in &request.workers {
            let worker = self.agents.get_agent(*id).await?;
            if worker.status != AgentStatus::Idle {
                return Err(RuntimeError::ActivationRequired { agent_id: *id });
            }
            let capabilities = models
                .iter()
                .filter(|model| {
                    model.available
                        && worker
                            .model_policy
                            .accepts(model.model_id, &model.capabilities)
                })
                .map(|model| model.capabilities.clone())
                .collect();
            profiles.push(WorkerCapabilities {
                agent_id: *id,
                models: capabilities,
            });
        }
        let task = request.planning_task_for_agent(&profiles, &coordinator.identity)?;
        let result = self
            .run_agent(
                request.coordinator,
                task,
                request.planning_options(),
                false,
                cancel,
            )
            .await?;
        let workflow = request.accept(&result.response.content, &profiles)?;
        Ok((workflow, result))
    }

    /// Compose only a workflow's named agents and matching worker inventory.
    /// The coordinator's model is unnecessary unless it is also a step candidate.
    pub async fn from_config_for_workflow(
        config: &AppConfig,
        workflow: &Workflow,
        resume: bool,
    ) -> Result<Self, RuntimeError> {
        Self::from_config_for_workflow_with_suspensions(config, workflow, resume, &BTreeSet::new())
            .await
    }

    /// Compose a workflow without activating agents under durable operator holds.
    pub async fn from_config_for_workflow_with_suspensions(
        config: &AppConfig,
        workflow: &Workflow,
        resume: bool,
        suspended: &BTreeSet<AgentId>,
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
                if suspended.contains(id) {
                    return Err(AgentRuntimeError::DurablySuspended { agent_id: *id }.into());
                }
                if agent.status != AgentStatus::Idle && !resume {
                    return Err(RuntimeError::ActivationRequired { agent_id: *id });
                }
            }
            if suspended.contains(id) {
                agent.status = AgentStatus::Suspended;
            } else if resume && agent.status != AgentStatus::Idle {
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
                                && agent.model_policy.accepts(model.id, &model.capabilities)
                        })
                })
        });
        scope_inventory(&mut scoped);
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
                        && agent.model_policy.accepts(model.id, &model.capabilities)
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
        scope_inventory(&mut scoped);
        Self::from_config(&scoped).await
    }
    /// Construct without network requests or model activation.
    pub async fn from_config(config: &AppConfig) -> Result<Self, RuntimeError> {
        let settings = &config.scheduling;
        let local_host_limits = if settings.local_host_limits {
            config.validate()?;
            for provider in &config.providers {
                if !configured_endpoint(config, provider)?.is_loopback() {
                    return Err(RuntimeError::LocalHostLimitsRequireLoopback);
                }
            }
            if settings.ollama_gpu_uuid.is_some()
                && !matches!(config.providers.as_slice(), [provider] if provider.kind == ProviderKind::Ollama)
            {
                return Err(RuntimeError::OllamaGpuScopeRequired);
            }
            let limits = LocalHostLimits::collect(config.resources)
                .ok_or(RuntimeError::LocalHostLimitsUnavailable)?;
            Some(if let Some(uuid) = &settings.ollama_gpu_uuid {
                NvidiaGpuMemory::collect(uuid)
                    .and_then(|memory| limits.with_nvidia_gpu(memory))
                    .ok_or(RuntimeError::OllamaGpuLimitsUnavailable)?
            } else {
                limits
            })
        } else {
            None
        };
        let registry = configured_registry(config).await?;
        let keep_alive = settings.keep_alive_seconds.map(u64::from);
        let scheduler = Arc::new(ResourceScheduler::new_configured(
            registry.clone(),
            SchedulingPolicy {
                queue: QueuePolicy {
                    capacity: settings.queue_capacity,
                    max_priority_bypasses: settings.max_priority_bypasses,
                },
                budget: local_host_limits
                    .map_or(config.resources, |limits| limits.effective_budget),
                default_keep_alive: Duration::from_secs(keep_alive.unwrap_or(300)),
                evict_idle_on_pressure: settings.evict_idle_on_pressure,
            },
            config.inference_overhead,
            SchedulingPreferences {
                model_priorities: settings.model_priorities.clone(),
                inference_keep_alive_seconds: keep_alive,
                prefer_observed_latency: settings.prefer_observed_latency,
            },
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
            local_host_limits,
        })
    }
    /// Access the trusted agent control plane.
    pub fn agents(&self) -> &AgentRuntime {
        &self.agents
    }

    /// Returns the optional startup evidence used to tighten the scheduler budget.
    pub fn local_host_limits(&self) -> Option<LocalHostLimits> {
        self.local_host_limits
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
        self.run_workflow_checkpointed(workflow, cancel, observer, None)
            .await
    }

    /// Run an authorized workflow and retain its durable checkpoints when configured.
    pub async fn run_workflow_checkpointed(
        &self,
        workflow: Workflow,
        cancel: CancellationToken,
        observer: Option<&tokio::sync::mpsc::UnboundedSender<vespera_core::EventKind>>,
        store: Option<&WorkflowStore>,
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
            .execute_checkpointed(workflow, cancel, observer, store)
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
                    && agent
                        .model_policy
                        .accepts(model.model_id, &model.capabilities)
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
