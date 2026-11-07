//! Conservative resource admission and coordinated provider lifecycle operations.
//!
//! Accounting records are estimates reserved by this process, not measured residency.
//! Uncertain lifecycle outcomes retain their memory reservation until acknowledged unload.

mod queue;
use queue::{AdmissionQueue, QueuePermit};
pub use queue::{QueueError, QueuePolicy, QueueSnapshot};

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::Mutex as AsyncMutex;
pub use vespera_core::ReservationState as ResidencyState;
use vespera_core::{
    DomainError, ModelId, ModelReservationSample, ProviderId, ResourceBudget, ResourceKind,
    ResourceRequest, ResourceUsage, SchedulerSample, TaskLeaseSample,
};
use vespera_model_registry::{ModelRegistry, RegistryError};
use vespera_providers::{
    BoundedInferenceRequest, CancellationToken, InferenceProvider, LifecycleCapabilityState,
    ProviderError,
};

/// Typed admission and lifecycle failures.
#[derive(Debug, Error)]
pub enum SchedulerError {
    /// Startup reconciliation cannot replace evidence after scheduling starts.
    #[error("scheduler startup residency can only be reconciled once before admission")]
    StartupAlreadyUsed,
    /// Provider residency data is malformed or no longer identifies the active adapter.
    #[error("startup residency evidence is invalid for provider {provider_id}")]
    InvalidStartupResidency { provider_id: ProviderId },
    /// The selected provider model is already loaded outside local ownership.
    #[error("model {model_id} is externally resident and cannot be activated by this scheduler")]
    ExternallyResident { model_id: ModelId },
    /// Inventory differs from a validated request or an existing reservation.
    #[error("model {model_id} binding differs from the request or reservation")]
    RegistrationChanged {
        /// Target model.
        model_id: ModelId,
    },
    /// Bounded admission queue rejected or cancelled the request.
    #[error("{0}")]
    Queue(#[from] QueueError),
    /// Incomplete estimates cannot establish safe admission.
    #[error("model {model_id} has unknown resource estimates")]
    UnknownResources {
        /// Target model.
        model_id: ModelId,
    },
    /// Registry lookup failed.
    #[error("registry error during scheduling: {0}")]
    Registry(#[from] RegistryError),
    /// Model is unavailable.
    #[error("model {model_id} is currently marked unavailable")]
    ModelUnavailable {
        /// Target model.
        model_id: ModelId,
    },
    /// Provider state is uncertain and must be reconciled before reuse.
    #[error("model {model_id} residency is uncertain; unload before reuse")]
    UncertainResidency {
        /// Target model.
        model_id: ModelId,
    },
    /// A model cannot be reloaded at a different context while locally leased.
    #[error(
        "model {model_id} has active leases at context {loaded}; requested context {requested}"
    )]
    ContextInUse {
        model_id: ModelId,
        loaded: u32,
        requested: u32,
    },
    /// The provider did not acknowledge the expected lifecycle target.
    #[error("provider did not acknowledge lifecycle operation for model {model_id}")]
    UnacknowledgedLifecycle {
        /// Target model.
        model_id: ModelId,
    },
    /// Resource admission failed.
    #[error("resource budget exceeded: {0}")]
    BudgetExceeded(#[from] DomainError),
    /// Provider lifecycle failed.
    #[error("provider lifecycle operation failed: {0}")]
    Provider(#[from] ProviderError),
    /// Cooperative cancellation.
    #[error("scheduling operation cancelled")]
    Cancelled,
}

/// Scheduling limits and idle eviction policy.
#[derive(Debug, Clone)]
pub struct SchedulingPolicy {
    /// Waiting capacity and priority fairness bounds.
    pub queue: QueuePolicy,
    /// Explicit operator limits, not measured hardware capacity.
    pub budget: ResourceBudget,
    /// Provider load keep-alive request.
    pub default_keep_alive: Duration,
    /// Attempt acknowledged idle eviction when memory admission fails.
    pub evict_idle_on_pressure: bool,
}

/// Accounted model reservation (not physical telemetry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidentModel {
    /// Model identity.
    pub model_id: ModelId,
    /// Provider identity.
    pub provider_id: ProviderId,
    /// Provider model name.
    pub provider_model_identifier: String,
    /// Reserved estimated RAM.
    pub ram_bytes: u64,
    /// Reserved estimated VRAM.
    pub vram_bytes: u64,
    /// Active local inference slots.
    pub active_inferences: u32,
    /// LRU logical ordering.
    pub last_used_tick: u64,
    /// Knowledge state, distinct from actual measured residency.
    pub residency: ResidencyState,
}

/// Admitted provider access with a shared, uniquely identified local inference lease.
/// Clones share ownership; the last drop or explicit release returns the slot once.
#[derive(Clone)]
pub struct ScheduledInference {
    lease: Arc<InferenceLease>,
    model_name: String,
    /// Model identity.
    pub model_id: ModelId,
    /// Provider identity.
    pub provider_id: ProviderId,
    /// Provider handle.
    pub provider: Arc<dyn InferenceProvider>,
    /// Admitted context ceiling.
    pub context_tokens: u32,
    /// Whether explicit load was acknowledged by this admission.
    pub loaded_on_demand: bool,
}
impl ScheduledInference {
    /// Provider-native name captured at admission, unaffected by later registry changes.
    #[must_use]
    pub fn model_name(&self) -> &str {
        &self.model_name
    }
    /// Unique identity of this admission, shared by clones.
    #[must_use]
    pub fn lease_id(&self) -> uuid::Uuid {
        self.lease.id
    }

    /// Return this local inference slot exactly once. Does not unload provider memory.
    /// Call only after local inference has stopped; remote cancellation is unverified.
    pub fn release(&self) -> bool {
        self.lease.release()
    }
}

struct InferenceLease {
    id: uuid::Uuid,
    state: Arc<Mutex<SchedulerState>>,
    permit: QueuePermit,
}
impl InferenceLease {
    fn release(&self) -> bool {
        let mut state = lock(&self.state);
        let Some(model_id) = state.leases.remove(&self.id) else {
            return false;
        };
        if let Some(model) = state.resident_models.get_mut(&model_id) {
            model.active_inferences -= 1;
            state.usage.active_inferences -= 1;
        }
        drop(state);
        self.permit.release();
        true
    }
}
impl Drop for InferenceLease {
    fn drop(&mut self) {
        self.release();
    }
}
impl fmt::Debug for ScheduledInference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScheduledInference")
            .field("model_id", &self.model_id)
            .field("provider_id", &self.provider_id)
            .field("context_tokens", &self.context_tokens)
            .field("loaded_on_demand", &self.loaded_on_demand)
            .finish()
    }
}

#[derive(Default)]
struct SchedulerState {
    usage: ResourceUsage,
    resident_models: BTreeMap<ModelId, ResidentModel>,
    logical_tick: u64,
    leases: BTreeMap<uuid::Uuid, ModelId>,
    // Retain the actual provider instance so removal/re-registration cannot redirect unload.
    bindings: BTreeMap<ModelId, ModelBinding>,
    admission_started: bool,
    startup: Option<StartupResidency>,
}

struct ModelBinding {
    provider: Arc<dyn InferenceProvider>,
    context_tokens: u32,
}

enum Preparation {
    Ready { needs_load: bool },
    ReloadContext,
    Evict(ModelId),
}

struct StartupResidency {
    providers: BTreeMap<ProviderId, ObservedProvider>,
    report: StartupResidencyReport,
}

struct ObservedProvider {
    adapter: Arc<dyn InferenceProvider>,
    models: BTreeSet<String>,
}

/// Aggregate provider observations used as external startup pressure.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct StartupResidencyReport {
    /// Number of scoped providers successfully queried.
    pub providers_observed: usize,
    /// Number of already resident models reported by those providers.
    pub external_models: usize,
    /// Reported external RAM bytes.
    pub ram_bytes: u64,
    /// Reported external VRAM bytes.
    pub vram_bytes: u64,
}

impl SchedulerState {
    fn reservation_counts(&self) -> (usize, usize) {
        let uncertain = self
            .resident_models
            .values()
            .filter(|model| model.residency == ResidencyState::Unknown)
            .count();
        (self.resident_models.len(), uncertain)
    }
}

/// One consistent accounting snapshot with bounded, credential-free model detail.
pub struct ReservationObservation {
    pub budget: ResourceBudget,
    pub usage: ResourceUsage,
    pub total: usize,
    pub uncertain: usize,
    pub models: Vec<ModelReservationSample>,
    pub tasks: Vec<TaskLeaseSample>,
}
impl fmt::Debug for SchedulerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SchedulerState")
            .field("usage", &self.usage)
            .field("resident_models", &self.resident_models)
            .field("leases", &self.leases)
            .finish_non_exhaustive()
    }
}

/// Marks an interrupted transition unknown synchronously, including future/task abort.
/// A load slot is released, but uncertain provider memory is deliberately retained.
struct TransitionGuard {
    state: Arc<Mutex<SchedulerState>>,
    model_id: ModelId,
    loading: bool,
    armed: bool,
}
impl Drop for TransitionGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut state = lock(&self.state);
        if let Some(model) = state.resident_models.get_mut(&self.model_id) {
            model.residency = ResidencyState::Unknown;
            if self.loading {
                model.active_inferences -= 1;
                state.usage.active_inferences -= 1;
            }
        }
    }
}

// No user code or network I/O executes while holding this lock. Poison recovery keeps
// conservative reservations visible if an unrelated task panics.
fn lock(state: &Mutex<SchedulerState>) -> MutexGuard<'_, SchedulerState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Thread-safe resource scheduler. Per-model gates serialize lifecycle operations;
/// the global accounting mutex is never held over provider I/O.
#[derive(Clone)]
pub struct ResourceScheduler {
    queue: AdmissionQueue,
    registry: Arc<ModelRegistry>,
    policy: SchedulingPolicy,
    state: Arc<Mutex<SchedulerState>>,
    gates: Arc<Mutex<BTreeMap<ModelId, Weak<AsyncMutex<()>>>>>,
}
impl fmt::Debug for ResourceScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceScheduler")
            .field("policy", &self.policy)
            .finish()
    }
}
impl ResourceScheduler {
    /// Create an empty scheduler without activating models.
    #[must_use]
    pub fn new(registry: Arc<ModelRegistry>, policy: SchedulingPolicy) -> Self {
        Self {
            queue: AdmissionQueue::new(policy.budget.max_concurrent_inferences, policy.queue),
            registry,
            policy,
            state: Arc::new(Mutex::new(SchedulerState::default())),
            gates: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
    /// Snapshot waiting and granted local admissions.
    #[must_use]
    pub fn queue_snapshot(&self) -> QueueSnapshot {
        self.queue.snapshot()
    }

    /// Read residency before the first local admission. Observed models are
    /// charged as external pressure and never become owned reservations.
    pub async fn reconcile_startup(&self, cancel: CancellationToken) -> Result<(), SchedulerError> {
        {
            let state = lock(&self.state);
            if state.admission_started || state.startup.is_some() {
                return Err(SchedulerError::StartupAlreadyUsed);
            }
        }
        let providers: BTreeSet<_> = self
            .registry
            .list_registrations()
            .await
            .into_iter()
            .map(|model| model.provider_id)
            .collect();
        let mut startup = StartupResidency {
            providers: BTreeMap::new(),
            report: StartupResidencyReport::default(),
        };
        for provider_id in providers {
            let provider = self.registry.get_provider(provider_id).await?;
            let snapshot = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(SchedulerError::Cancelled),
                result = provider.query_residency(cancel.clone()) => result?,
            };
            if snapshot.provider_id != provider_id {
                return Err(SchedulerError::InvalidStartupResidency { provider_id });
            }
            let mut names = BTreeSet::new();
            let mut ram = 0;
            let mut vram = 0;
            for model in snapshot.resident_models {
                if model.provider_model_name.trim().is_empty()
                    || model
                        .ram_resident_bytes
                        .checked_add(model.vram_resident_bytes)
                        != Some(model.total_resident_bytes)
                    || !names.insert(provider.residency_model_key(&model.provider_model_name))
                {
                    return Err(SchedulerError::InvalidStartupResidency { provider_id });
                }
                ram = add_memory(ram, model.ram_resident_bytes, ResourceKind::Ram)?;
                vram = add_memory(vram, model.vram_resident_bytes, ResourceKind::Vram)?;
            }
            if ram != snapshot.total_provider_ram_bytes
                || vram != snapshot.total_provider_vram_bytes
            {
                return Err(SchedulerError::InvalidStartupResidency { provider_id });
            }
            startup.report.ram_bytes =
                add_memory(startup.report.ram_bytes, ram, ResourceKind::Ram)?;
            startup.report.vram_bytes =
                add_memory(startup.report.vram_bytes, vram, ResourceKind::Vram)?;
            startup.report.external_models += names.len();
            startup.report.providers_observed += 1;
            startup.providers.insert(
                provider_id,
                ObservedProvider {
                    adapter: provider,
                    models: names,
                },
            );
        }
        let mut state = lock(&self.state);
        if cancel.is_cancelled() {
            return Err(SchedulerError::Cancelled);
        }
        if state.admission_started || state.startup.is_some() {
            return Err(SchedulerError::StartupAlreadyUsed);
        }
        state.startup = Some(startup);
        Ok(())
    }

    /// Observed startup pressure, separate from this runtime's reservations.
    pub fn startup_residency(&self) -> Option<StartupResidencyReport> {
        lock(&self.state)
            .startup
            .as_ref()
            .map(|startup| startup.report)
    }

    /// Capture reservation counters and model states under one accounting lock.
    pub fn reservation_snapshot(&self) -> (ResourceBudget, ResourceUsage, usize, usize) {
        let state = lock(&self.state);
        let (total, uncertain) = state.reservation_counts();
        (self.policy.budget, state.usage, total, uncertain)
    }

    pub fn observe_reservations(&self) -> ReservationObservation {
        self.observe_task_reservations(&[])
    }

    /// Include only task ownership candidates still backed by an issued lease
    /// and a displayed model reservation in this accounting snapshot.
    pub fn observe_task_reservations(
        &self,
        candidates: &[TaskLeaseSample],
    ) -> ReservationObservation {
        let state = lock(&self.state);
        let (total, uncertain) = state.reservation_counts();
        let mut leases = BTreeMap::<ModelId, u32>::new();
        for &model in state.leases.values() {
            *leases.entry(model).or_default() += 1;
        }
        let models: Vec<_> = state
            .resident_models
            .values()
            .filter(|model| model.active_inferences > 0)
            .chain(
                state
                    .resident_models
                    .values()
                    .filter(|model| model.active_inferences == 0),
            )
            .take(SchedulerSample::MAX_MODELS)
            .map(|model| ModelReservationSample {
                model_id: model.model_id,
                provider_id: model.provider_id,
                state: model.residency,
                ram_bytes: model.ram_bytes,
                vram_bytes: model.vram_bytes,
                charged_slots: model.active_inferences,
                issued_leases: leases.get(&model.model_id).copied().unwrap_or_default(),
            })
            .collect();
        let tasks = candidates
            .iter()
            .copied()
            .filter(|task| {
                state.leases.get(&task.lease_id) == Some(&task.model_id)
                    && models.iter().any(|model| {
                        model.model_id == task.model_id && model.provider_id == task.provider_id
                    })
            })
            .take(SchedulerSample::MAX_TASKS)
            .collect();
        ReservationObservation {
            budget: self.policy.budget,
            usage: state.usage,
            total,
            uncertain,
            models,
            tasks,
        }
    }

    /// Configured logical context ceiling, not a measured backend allocation.
    pub fn max_context_tokens(&self) -> u32 {
        self.policy.budget.max_context_tokens
    }

    /// Admit an already validated request, fencing its provider-native model binding
    /// before any lifecycle operation. The admitted context remains exactly the request's.
    pub async fn admit_request(
        &self,
        model_id: ModelId,
        request: &BoundedInferenceRequest,
        cancel: CancellationToken,
    ) -> Result<ScheduledInference, SchedulerError> {
        self.prepare(
            model_id,
            Some(request.context_window_limit()),
            0,
            Some(request.model_name()),
            cancel,
        )
        .await
    }

    /// Reject new/queued admissions and wait at most `timeout` for owned permits.
    /// Existing inference is not forcibly stopped or falsely reported as unloaded.
    /// A false return leaves active leases/reservations visible for operator recovery.
    pub async fn shutdown(&self, timeout: Duration) -> bool {
        self.queue.close();
        self.queue.drain(timeout).await
    }
    fn gate(&self, model_id: ModelId) -> Arc<AsyncMutex<()>> {
        let mut gates = self
            .gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = gates.get(&model_id).and_then(Weak::upgrade) {
            return gate;
        }
        let gate = Arc::new(AsyncMutex::new(()));
        gates.insert(model_id, Arc::downgrade(&gate));
        gate
    }
    /// Snapshot local reserved usage.
    pub async fn usage_snapshot(&self) -> ResourceUsage {
        lock(&self.state).usage
    }
    /// Snapshot all charged reservations, including unknown/in-flight states.
    pub async fn resident_models(&self) -> Vec<ResidentModel> {
        lock(&self.state)
            .resident_models
            .values()
            .cloned()
            .collect()
    }
    /// Admit with default priority through the bounded queue.
    pub async fn admit_and_prepare(
        &self,
        model_id: ModelId,
        requested_context_tokens: Option<u32>,
        cancel: CancellationToken,
    ) -> Result<ScheduledInference, SchedulerError> {
        self.admit_with_priority(model_id, requested_context_tokens, 0, cancel)
            .await
    }

    /// Enqueue bounded admission. Larger priority values run first, subject to
    /// the configured bypass bound; equal priorities preserve arrival order.
    pub async fn admit_with_priority(
        &self,
        model_id: ModelId,
        requested_context_tokens: Option<u32>,
        priority: u8,
        cancel: CancellationToken,
    ) -> Result<ScheduledInference, SchedulerError> {
        self.prepare(model_id, requested_context_tokens, priority, None, cancel)
            .await
    }

    async fn prepare(
        &self,
        model_id: ModelId,
        requested_context_tokens: Option<u32>,
        priority: u8,
        expected_model_name: Option<&str>,
        cancel: CancellationToken,
    ) -> Result<ScheduledInference, SchedulerError> {
        lock(&self.state).admission_started = true;
        let permit = self
            .queue
            .acquire(priority, &cancel)
            .await
            .map_err(|error| match error {
                QueueError::Cancelled => SchedulerError::Cancelled,
                error => SchedulerError::Queue(error),
            })?;
        let gate = self.gate(model_id);
        let _model_guard = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(SchedulerError::Cancelled),
            guard = gate.lock() => guard,
        };
        if cancel.is_cancelled() {
            return Err(SchedulerError::Cancelled);
        }
        let model = self.registry.get_model(model_id).await?;
        if expected_model_name.is_some_and(|name| name != model.provider_model_identifier) {
            return Err(SchedulerError::RegistrationChanged { model_id });
        }
        if !model.available {
            return Err(SchedulerError::ModelUnavailable { model_id });
        }
        let provider = self.registry.get_provider(model.provider_id).await?;
        let context_tokens = requested_context_tokens.unwrap_or_else(|| {
            model
                .context_window
                .unwrap_or(self.policy.budget.max_context_tokens)
                .min(self.policy.budget.max_context_tokens)
        });
        if let Some(limit) = model.context_window
            && context_tokens > limit
        {
            return Err(SchedulerError::BudgetExceeded(
                DomainError::ResourceLimitExceeded {
                    resource: ResourceKind::ContextTokens,
                    requested: u64::from(context_tokens),
                    limit: u64::from(limit),
                },
            ));
        }
        let (Some(ram_bytes), Some(vram_bytes)) = (
            model.estimated_resources.ram_bytes,
            model.estimated_resources.vram_bytes,
        ) else {
            return Err(SchedulerError::UnknownResources { model_id });
        };
        let mut attempted = std::collections::BTreeSet::new();
        let needs_load = loop {
            if cancel.is_cancelled() {
                return Err(SchedulerError::Cancelled);
            }
            let decision = 'admission: {
                let mut state = lock(&self.state);
                let external = if let Some(startup) = &state.startup {
                    let Some(observed) = startup.providers.get(&model.provider_id) else {
                        return Err(SchedulerError::InvalidStartupResidency {
                            provider_id: model.provider_id,
                        });
                    };
                    if !Arc::ptr_eq(&observed.adapter, &provider) {
                        return Err(SchedulerError::InvalidStartupResidency {
                            provider_id: model.provider_id,
                        });
                    }
                    if observed
                        .models
                        .contains(&provider.residency_model_key(&model.provider_model_identifier))
                    {
                        return Err(SchedulerError::ExternallyResident { model_id });
                    }
                    startup.report
                } else {
                    StartupResidencyReport::default()
                };
                let existing = state.resident_models.get(&model_id);
                let binding = state.bindings.get(&model_id);
                if existing.is_some_and(|entry| {
                    entry.provider_id != model.provider_id
                        || entry.provider_model_identifier != model.provider_model_identifier
                        || entry.ram_bytes != ram_bytes
                        || entry.vram_bytes != vram_bytes
                }) || binding.is_some_and(|bound| !Arc::ptr_eq(&bound.provider, &provider))
                {
                    return Err(SchedulerError::RegistrationChanged { model_id });
                }
                if existing.is_some_and(|entry| {
                    !matches!(
                        entry.residency,
                        ResidencyState::Loaded | ResidencyState::ProviderManaged
                    )
                }) {
                    return Err(SchedulerError::UncertainResidency { model_id });
                }
                let changed_context = existing.zip(binding).filter(|(entry, bound)| {
                    entry.residency == ResidencyState::Loaded
                        && bound.context_tokens != context_tokens
                });
                let reuse = existing.is_some() && changed_context.is_none();
                let request = ResourceRequest {
                    ram_bytes: if reuse { 0 } else { ram_bytes },
                    vram_bytes: if reuse { 0 } else { vram_bytes },
                    context_tokens,
                };
                let external_usage = ResourceUsage {
                    ram_bytes: external.ram_bytes,
                    vram_bytes: external.vram_bytes,
                    active_inferences: 0,
                };
                self.policy.budget.admit(&external_usage, &request)?;
                if let Some((entry, bound)) = changed_context {
                    if entry.active_inferences != 0 {
                        return Err(SchedulerError::ContextInUse {
                            model_id,
                            loaded: bound.context_tokens,
                            requested: context_tokens,
                        });
                    }
                    break 'admission Preparation::ReloadContext;
                }
                let accounted = ResourceUsage {
                    ram_bytes: add_memory(
                        state.usage.ram_bytes,
                        external.ram_bytes,
                        ResourceKind::Ram,
                    )?,
                    vram_bytes: add_memory(
                        state.usage.vram_bytes,
                        external.vram_bytes,
                        ResourceKind::Vram,
                    )?,
                    active_inferences: state.usage.active_inferences,
                };
                match self.policy.budget.admit(&accounted, &request) {
                    Ok(mut usage) => {
                        usage.ram_bytes -= external.ram_bytes;
                        usage.vram_bytes -= external.vram_bytes;
                        state.usage = usage;
                        state.logical_tick = state.logical_tick.saturating_add(1);
                        let tick = state.logical_tick;
                        if let Some(entry) = state.resident_models.get_mut(&model_id) {
                            entry.active_inferences += 1;
                            entry.last_used_tick = tick;
                        } else {
                            state.bindings.insert(
                                model_id,
                                ModelBinding {
                                    provider: provider.clone(),
                                    context_tokens,
                                },
                            );
                            state.resident_models.insert(
                                model_id,
                                ResidentModel {
                                    model_id,
                                    provider_id: model.provider_id,
                                    provider_model_identifier: model
                                        .provider_model_identifier
                                        .clone(),
                                    ram_bytes,
                                    vram_bytes,
                                    active_inferences: 1,
                                    last_used_tick: tick,
                                    residency: ResidencyState::Loading,
                                },
                            );
                        }
                        Preparation::Ready { needs_load: !reuse }
                    }
                    Err(error) => {
                        let candidate = if self.policy.evict_idle_on_pressure
                            && matches!(
                                error,
                                DomainError::ResourceLimitExceeded {
                                    resource: ResourceKind::Ram | ResourceKind::Vram,
                                    ..
                                }
                            ) {
                            state
                                .resident_models
                                .values()
                                .filter(|entry| {
                                    entry.active_inferences == 0
                                        && entry.model_id != model_id
                                        && !attempted.contains(&entry.model_id)
                                        && !matches!(
                                            entry.residency,
                                            ResidencyState::Loading | ResidencyState::Unloading
                                        )
                                })
                                .min_by_key(|entry| entry.last_used_tick)
                                .map(|entry| entry.model_id)
                        } else {
                            None
                        };
                        Preparation::Evict(candidate.ok_or(SchedulerError::BudgetExceeded(error))?)
                    }
                }
            };
            match decision {
                Preparation::Ready { needs_load } => break needs_load,
                Preparation::ReloadContext => {
                    self.evict_with_gate_held(model_id, cancel.clone()).await?;
                }
                Preparation::Evict(candidate) => {
                    attempted.insert(candidate);
                    self.evict_model(candidate, cancel.clone()).await?;
                }
            }
        };
        let mut loaded_on_demand = false;
        if needs_load {
            let mut transition = TransitionGuard {
                state: self.state.clone(),
                model_id,
                loading: true,
                armed: true,
            };
            let explicit = provider.lifecycle_support().explicit_model_load
                == LifecycleCapabilityState::Supported;
            if explicit {
                let outcome = provider
                    .load_model_with_context(
                        &model.provider_model_identifier,
                        context_tokens,
                        Some(self.policy.default_keep_alive),
                        cancel.clone(),
                    )
                    .await?;
                if !outcome.acknowledged
                    || outcome.provider_id != model.provider_id
                    || outcome.provider_model_name != model.provider_model_identifier
                {
                    return Err(SchedulerError::UnacknowledgedLifecycle { model_id });
                }
                loaded_on_demand = true;
            }
            {
                let mut state = lock(&self.state);
                if let Some(entry) = state.resident_models.get_mut(&model_id) {
                    entry.residency = if explicit {
                        ResidencyState::Loaded
                    } else {
                        ResidencyState::ProviderManaged
                    };
                }
            }
            transition.armed = false;
        }
        let id = uuid::Uuid::new_v4();
        lock(&self.state).leases.insert(id, model_id);
        let inference = ScheduledInference {
            model_name: model.provider_model_identifier.clone(),
            lease: Arc::new(InferenceLease {
                id,
                state: self.state.clone(),
                permit,
            }),
            model_id,
            provider_id: model.provider_id,
            provider,
            context_tokens,
            loaded_on_demand,
        };
        if cancel.is_cancelled() {
            return Err(SchedulerError::Cancelled);
        }
        Ok(inference)
    }
    /// Evict an idle reservation only after a matching acknowledged unload.
    /// Unsupported, failed, unacknowledged, and interrupted unloads retain accounting.
    pub async fn evict_model(
        &self,
        model_id: ModelId,
        cancel: CancellationToken,
    ) -> Result<bool, SchedulerError> {
        let gate = self.gate(model_id);
        let _guard = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(SchedulerError::Cancelled),
            guard = gate.lock() => guard,
        };
        self.evict_with_gate_held(model_id, cancel).await
    }

    async fn evict_with_gate_held(
        &self,
        model_id: ModelId,
        cancel: CancellationToken,
    ) -> Result<bool, SchedulerError> {
        if cancel.is_cancelled() {
            return Err(SchedulerError::Cancelled);
        }
        let model = {
            let state = lock(&self.state);
            match state.resident_models.get(&model_id) {
                Some(entry) if entry.active_inferences == 0 => entry.clone(),
                _ => return Ok(false),
            }
        };
        let provider = lock(&self.state)
            .bindings
            .get(&model_id)
            .map(|bound| bound.provider.clone())
            .ok_or(SchedulerError::UncertainResidency { model_id })?;
        if provider.lifecycle_support().explicit_model_unload != LifecycleCapabilityState::Supported
        {
            return Err(ProviderError::UnsupportedOperation {
                provider_id: model.provider_id,
                operation: "unload_model",
            }
            .into());
        }
        {
            let mut state = lock(&self.state);
            if let Some(entry) = state.resident_models.get_mut(&model_id) {
                entry.residency = ResidencyState::Unloading;
            }
        }
        let mut transition = TransitionGuard {
            state: self.state.clone(),
            model_id,
            loading: false,
            armed: true,
        };
        let outcome = provider
            .unload_model(&model.provider_model_identifier, cancel)
            .await?;
        if !outcome.acknowledged
            || outcome.provider_id != model.provider_id
            || outcome.provider_model_name != model.provider_model_identifier
        {
            return Err(SchedulerError::UnacknowledgedLifecycle { model_id });
        }
        let mut state = lock(&self.state);
        if let Some(entry) = state.resident_models.remove(&model_id) {
            state.usage.ram_bytes -= entry.ram_bytes;
            state.usage.vram_bytes -= entry.vram_bytes;
        }
        state.bindings.remove(&model_id);
        transition.armed = false;
        Ok(true)
    }
}

fn add_memory(
    current: u64,
    additional: u64,
    resource: ResourceKind,
) -> Result<u64, SchedulerError> {
    current
        .checked_add(additional)
        .ok_or(SchedulerError::BudgetExceeded(
            DomainError::ResourceOverflow { resource },
        ))
}
