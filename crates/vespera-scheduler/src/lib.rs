//! Resource-aware model scheduling and lifecycle management for VESPERA.
//!
//! Implements Milestone 5:
//! - Checked RAM, VRAM, concurrency, and context admission via `vespera_core::ResourceBudget`.
//! - Model residency tracking so resident models are reused without double-counting memory.
//! - Automatic least-recently-used (LRU) eviction of idle models under memory pressure.
//! - Rollback of resource reservations if provider model loading fails.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use thiserror::Error;
use vespera_core::{
    DomainError, ModelId, ProviderId, ResourceBudget, ResourceKind, ResourceRequest, ResourceUsage,
};
use vespera_model_registry::{ModelRegistry, RegistryError};
use vespera_providers::{
    CancellationToken, InferenceProvider, LifecycleCapabilityState, ProviderError,
};

/// Error taxonomy for resource-aware scheduling operations.
#[derive(Debug, Error)]
pub enum SchedulerError {
    /// Admission requires explicit RAM and VRAM estimates, including explicit CPU-only zero VRAM.
    #[error("model {model_id} has unknown resource estimates")]
    UnknownResources {
        /// Model whose estimates are incomplete.
        model_id: ModelId,
    },
    /// Registry lookup error for model or provider.
    #[error("registry error during scheduling: {0}")]
    Registry(#[from] RegistryError),
    /// Target model is marked unavailable for routing.
    #[error("model {model_id} is currently marked unavailable")]
    ModelUnavailable {
        /// Unavailable model identifier.
        model_id: ModelId,
    },
    /// Resource budget admission failed even after idle eviction attempts.
    #[error("resource budget exceeded: {0}")]
    BudgetExceeded(#[from] DomainError),
    /// Provider failed during explicit model load or unload.
    #[error("provider lifecycle operation failed: {0}")]
    Provider(#[from] ProviderError),
    /// Operation was cancelled via cancellation token.
    #[error("scheduling operation cancelled")]
    Cancelled,
}

/// Configuration policy for resource-aware scheduling.
#[derive(Debug, Clone)]
pub struct SchedulingPolicy {
    /// Hardware resource budget limits.
    pub budget: ResourceBudget,
    /// Default keep-alive duration passed to providers supporting explicit model load.
    pub default_keep_alive: Duration,
    /// Whether to automatically evict idle resident models when RAM or VRAM is full.
    pub evict_idle_on_pressure: bool,
}

/// Residency record for a model currently loaded in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidentModel {
    /// Unique model identifier.
    pub model_id: ModelId,
    /// Associated provider identifier.
    pub provider_id: ProviderId,
    /// Provider-specific model name.
    pub provider_model_identifier: String,
    /// Accounted system RAM in bytes.
    pub ram_bytes: u64,
    /// Accounted GPU VRAM in bytes.
    pub vram_bytes: u64,
    /// Number of currently running inferences using this model.
    pub active_inferences: u32,
    /// Monotonic logical tick of last admission for LRU ordering.
    pub last_used_tick: u64,
}

/// Admitted inference lease returned by the scheduler.
#[derive(Clone)]
pub struct ScheduledInference {
    /// Admitted model identifier.
    pub model_id: ModelId,
    /// Provider identifier hosting the model.
    pub provider_id: ProviderId,
    /// Provider handle ready for inference execution.
    pub provider: Arc<dyn InferenceProvider>,
    /// Admitted context window tokens for the request.
    pub context_tokens: u32,
    /// Whether the model was freshly loaded on demand for this request.
    pub loaded_on_demand: bool,
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

#[derive(Debug)]
struct SchedulerState {
    usage: ResourceUsage,
    resident_models: BTreeMap<ModelId, ResidentModel>,
    logical_tick: u64,
}

impl SchedulerState {
    fn new() -> Self {
        Self {
            usage: ResourceUsage {
                ram_bytes: 0,
                vram_bytes: 0,
                active_inferences: 0,
            },
            resident_models: BTreeMap::new(),
            logical_tick: 0,
        }
    }

    fn next_tick(&mut self) -> u64 {
        self.logical_tick = self.logical_tick.saturating_add(1);
        self.logical_tick
    }
}

/// Resource-aware scheduler and model lifecycle coordinator.
#[derive(Clone)]
pub struct ResourceScheduler {
    registry: Arc<ModelRegistry>,
    policy: SchedulingPolicy,
    state: Arc<Mutex<SchedulerState>>,
}

impl fmt::Debug for ResourceScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceScheduler")
            .field("policy", &self.policy)
            .finish()
    }
}

impl ResourceScheduler {
    /// Creates a new resource scheduler bound to the central model registry and policy.
    #[must_use]
    pub fn new(registry: Arc<ModelRegistry>, policy: SchedulingPolicy) -> Self {
        Self {
            registry,
            policy,
            state: Arc::new(Mutex::new(SchedulerState::new())),
        }
    }

    /// Returns a snapshot of current aggregate resource usage.
    pub async fn usage_snapshot(&self) -> ResourceUsage {
        let state = self.state.lock().await;
        state.usage
    }

    /// Returns a list of all currently resident models.
    pub async fn resident_models(&self) -> Vec<ResidentModel> {
        let state = self.state.lock().await;
        state.resident_models.values().cloned().collect()
    }

    /// Admits an inference request against the resource budget, evicting idle models if needed,
    /// and prepares the model on the target provider.
    pub async fn admit_and_prepare(
        &self,
        model_id: ModelId,
        requested_context_tokens: Option<u32>,
        cancel: CancellationToken,
    ) -> Result<ScheduledInference, SchedulerError> {
        if cancel.is_cancelled() {
            return Err(SchedulerError::Cancelled);
        }

        let model_reg = self.registry.get_model(model_id).await?;
        if !model_reg.available {
            return Err(SchedulerError::ModelUnavailable { model_id });
        }

        let provider = self.registry.get_provider(model_reg.provider_id).await?;
        let context_tokens = requested_context_tokens
            .or(model_reg.context_window)
            .unwrap_or(self.policy.budget.max_context_tokens);

        let mut evicted_to_unload: Vec<ResidentModel> = Vec::new();
        let loaded_on_demand: bool;

        {
            let mut state = self.state.lock().await;

            if state.resident_models.contains_key(&model_id) {
                let request = ResourceRequest {
                    ram_bytes: 0,
                    vram_bytes: 0,
                    context_tokens,
                };
                let new_usage = self.policy.budget.admit(&state.usage, &request)?;
                state.usage = new_usage;
                let tick = state.next_tick();
                if let Some(entry) = state.resident_models.get_mut(&model_id) {
                    entry.active_inferences = entry.active_inferences.saturating_add(1);
                    entry.last_used_tick = tick;
                }
                loaded_on_demand = false;
            } else {
                let (Some(model_ram), Some(model_vram)) = (
                    model_reg.estimated_resources.ram_bytes,
                    model_reg.estimated_resources.vram_bytes,
                ) else {
                    return Err(SchedulerError::UnknownResources { model_id });
                };
                let request = ResourceRequest {
                    ram_bytes: model_ram,
                    vram_bytes: model_vram,
                    context_tokens,
                };

                while let Err(err) = self.policy.budget.admit(&state.usage, &request) {
                    let is_memory_pressure = matches!(
                        err,
                        DomainError::ResourceLimitExceeded {
                            resource: ResourceKind::Ram | ResourceKind::Vram,
                            ..
                        }
                    );

                    if !self.policy.evict_idle_on_pressure || !is_memory_pressure {
                        return Err(SchedulerError::BudgetExceeded(err));
                    }

                    let lru_idle_model_id = state
                        .resident_models
                        .values()
                        .filter(|m| m.active_inferences == 0)
                        .min_by_key(|m| m.last_used_tick)
                        .map(|m| m.model_id);

                    if let Some(evict_id) = lru_idle_model_id
                        && let Some(removed) = state.resident_models.remove(&evict_id)
                    {
                        state.usage.ram_bytes =
                            state.usage.ram_bytes.saturating_sub(removed.ram_bytes);
                        state.usage.vram_bytes =
                            state.usage.vram_bytes.saturating_sub(removed.vram_bytes);
                        evicted_to_unload.push(removed);
                    } else {
                        return Err(SchedulerError::BudgetExceeded(err));
                    }
                }

                let new_usage = self.policy.budget.admit(&state.usage, &request)?;
                state.usage = new_usage;
                let tick = state.next_tick();
                state.resident_models.insert(
                    model_id,
                    ResidentModel {
                        model_id,
                        provider_id: model_reg.provider_id,
                        provider_model_identifier: model_reg.provider_model_identifier.clone(),
                        ram_bytes: model_ram,
                        vram_bytes: model_vram,
                        active_inferences: 1,
                        last_used_tick: tick,
                    },
                );
                loaded_on_demand = true;
            }
        }

        for evicted in evicted_to_unload {
            if let Ok(evicted_provider) = self.registry.get_provider(evicted.provider_id).await
                && matches!(
                    evicted_provider.lifecycle_support().explicit_model_unload,
                    LifecycleCapabilityState::Supported
                )
            {
                let _ = evicted_provider
                    .unload_model(&evicted.provider_model_identifier, cancel.clone())
                    .await;
            }
        }

        if loaded_on_demand
            && matches!(
                provider.lifecycle_support().explicit_model_load,
                LifecycleCapabilityState::Supported
            )
            && let Err(err) = provider
                .load_model(
                    &model_reg.provider_model_identifier,
                    Some(self.policy.default_keep_alive),
                    cancel,
                )
                .await
        {
            let mut state = self.state.lock().await;
            if let Some(removed) = state.resident_models.remove(&model_id) {
                state.usage.ram_bytes = state.usage.ram_bytes.saturating_sub(removed.ram_bytes);
                state.usage.vram_bytes = state.usage.vram_bytes.saturating_sub(removed.vram_bytes);
            }
            state.usage.active_inferences = state.usage.active_inferences.saturating_sub(1);
            return Err(SchedulerError::Provider(err));
        }

        Ok(ScheduledInference {
            model_id,
            provider_id: model_reg.provider_id,
            provider,
            context_tokens,
            loaded_on_demand,
        })
    }

    /// Releases an active inference slot for a model while keeping the model resident in memory.
    pub async fn release_inference(&self, model_id: ModelId) {
        let mut state = self.state.lock().await;
        if let Some(entry) = state.resident_models.get_mut(&model_id)
            && entry.active_inferences > 0
        {
            entry.active_inferences -= 1;
            state.usage.active_inferences = state.usage.active_inferences.saturating_sub(1);
        }
    }

    /// Explicitly evicts an idle model from residency and unloads it from the provider.
    pub async fn evict_model(
        &self,
        model_id: ModelId,
        cancel: CancellationToken,
    ) -> Result<bool, SchedulerError> {
        let removed = {
            let mut state = self.state.lock().await;
            match state.resident_models.get(&model_id) {
                Some(entry) if entry.active_inferences == 0 => {
                    let removed = state.resident_models.remove(&model_id);
                    if let Some(ref r) = removed {
                        state.usage.ram_bytes = state.usage.ram_bytes.saturating_sub(r.ram_bytes);
                        state.usage.vram_bytes =
                            state.usage.vram_bytes.saturating_sub(r.vram_bytes);
                    }
                    removed
                }
                _ => None,
            }
        };

        if let Some(evicted) = removed {
            let provider = self.registry.get_provider(evicted.provider_id).await?;
            if matches!(
                provider.lifecycle_support().explicit_model_unload,
                LifecycleCapabilityState::Supported
            ) {
                provider
                    .unload_model(&evicted.provider_model_identifier, cancel)
                    .await?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
