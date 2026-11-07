//! Atomic model registration and conservative, expiring provider-health observations.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::RwLock;
use tokio::time::Instant;
use vespera_core::{Capability, ModelId, ProviderId, ResourceEstimate};
use vespera_providers::{CancellationToken, InferenceProvider, ProviderHealthStatus};

/// Registry validation and lifecycle failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// Missing model.
    #[error("model {model_id} not found in registry")]
    ModelNotFound {
        /// Target identity.
        model_id: ModelId,
    },
    /// Missing provider.
    #[error("provider {provider_id} not registered")]
    ProviderNotRegistered {
        /// Target identity.
        provider_id: ProviderId,
    },
    /// Identity, binding or referential conflict. Messages exclude model/provider payloads.
    #[error("registry conflict: {reason}")]
    Conflict {
        /// Sanitized explanation.
        reason: String,
    },
    /// Invalid registration or polling policy.
    #[error("invalid registry field: {field}")]
    InvalidField {
        /// Invalid field name, not its contents.
        field: &'static str,
    },
}

/// A model's operator-declared metadata, independent of provider residency.
#[derive(Debug, Clone)]
pub struct ModelRegistration {
    /// Stable model identity.
    pub model_id: ModelId,
    /// Hosting provider.
    pub provider_id: ProviderId,
    /// Display name.
    pub name: String,
    /// Exact provider model name.
    pub provider_model_identifier: String,
    /// Parameter description when known.
    pub parameter_size: Option<String>,
    /// Quantization when known.
    pub quantization: Option<String>,
    /// Maximum context when known.
    pub context_window: Option<u32>,
    /// Advertised capabilities, not authority.
    pub capabilities: BTreeSet<Capability>,
    /// Operator-supplied estimates, never measured telemetry.
    pub estimated_resources: ResourceEstimate,
    /// Operator enablement on registration/update. Routing snapshots additionally
    /// require a fresh healthy provider observation; raw metadata remains intact.
    pub available: bool,
}
impl ModelRegistration {
    fn validate(&self) -> Result<(), RegistryError> {
        if self.name.trim().is_empty() {
            return Err(RegistryError::InvalidField { field: "name" });
        }
        if self.provider_model_identifier.trim().is_empty() {
            return Err(RegistryError::InvalidField {
                field: "provider_model_identifier",
            });
        }
        if self.context_window == Some(0) {
            return Err(RegistryError::InvalidField {
                field: "context_window",
            });
        }
        Ok(())
    }
}

/// Health observations expire rather than remaining healthy indefinitely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderHealthState {
    /// No completed poll for this provider instance.
    Unknown,
    /// Fresh successful health response.
    Healthy,
    /// Fresh degraded response; excluded by conservative routing.
    Degraded,
    /// Poll failed, timed out, or returned a mismatched identity.
    Unreachable,
    /// Last observation exceeded the configured maximum age.
    Stale,
}
/// In-process monotonic health evidence. It is not persisted or compared across hosts.
#[derive(Debug, Clone, Copy)]
pub struct ProviderHealthObservation {
    /// Effective state at snapshot time.
    pub state: ProviderHealthState,
    /// Last completed observation, if any.
    pub checked_at: Option<Instant>,
}
struct ProviderEntry {
    provider: Arc<dyn InferenceProvider>,
    observation: Option<(ProviderHealthStatus, Instant)>,
    poll_revision: u64,
}
#[derive(Default)]
struct RegistryState {
    models: BTreeMap<ModelId, ModelRegistration>,
    providers: BTreeMap<ProviderId, ProviderEntry>,
    poll_revision: u64,
}

/// Thread-safe inventory. No registry lock is held during provider I/O.
#[derive(Clone)]
pub struct ModelRegistry {
    state: Arc<RwLock<RegistryState>>,
    health_max_age: Duration,
    health_timeout: Duration,
}
impl fmt::Debug for ModelRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelRegistry")
            .field("health_max_age", &self.health_max_age)
            .field("health_timeout", &self.health_timeout)
            .finish_non_exhaustive()
    }
}
impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}
impl ModelRegistry {
    /// Create an empty registry with 30-second health freshness and 5-second poll deadlines.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(RegistryState::default())),
            health_max_age: Duration::from_secs(30),
            health_timeout: Duration::from_secs(5),
        }
    }
    /// Customize positive monotonic health freshness and per-provider poll deadlines.
    pub fn with_health_policy(
        health_max_age: Duration,
        health_timeout: Duration,
    ) -> Result<Self, RegistryError> {
        if health_max_age.is_zero() || health_timeout.is_zero() {
            return Err(RegistryError::InvalidField {
                field: "health_policy",
            });
        }
        Ok(Self {
            health_max_age,
            health_timeout,
            ..Self::new()
        })
    }
    /// Register without replacing an existing provider identity or polling/loading models.
    pub async fn register_provider(
        &self,
        provider: Arc<dyn InferenceProvider>,
    ) -> Result<(), RegistryError> {
        let id = provider.provider_id();
        let mut state = self.state.write().await;
        if state.providers.contains_key(&id) {
            return Err(RegistryError::Conflict {
                reason: "provider identity already registered".into(),
            });
        }
        state.providers.insert(
            id,
            ProviderEntry {
                provider,
                observation: None,
                poll_revision: 0,
            },
        );
        Ok(())
    }
    /// Obtain a provider handle. Lookup alone does not establish health.
    pub async fn get_provider(
        &self,
        provider_id: ProviderId,
    ) -> Result<Arc<dyn InferenceProvider>, RegistryError> {
        self.state
            .read()
            .await
            .providers
            .get(&provider_id)
            .map(|entry| entry.provider.clone())
            .ok_or(RegistryError::ProviderNotRegistered { provider_id })
    }
    /// Remove an unreferenced provider. Previously issued handles remain valid.
    pub async fn remove_provider(&self, provider_id: ProviderId) -> Result<(), RegistryError> {
        let mut state = self.state.write().await;
        if !state.providers.contains_key(&provider_id) {
            return Err(RegistryError::ProviderNotRegistered { provider_id });
        }
        if state
            .models
            .values()
            .any(|model| model.provider_id == provider_id)
        {
            return Err(RegistryError::Conflict {
                reason: "provider still has registered models".into(),
            });
        }
        state.providers.remove(&provider_id);
        Ok(())
    }
    /// Register unique model and provider-model identities without loading anything.
    pub async fn register_model(
        &self,
        registration: ModelRegistration,
    ) -> Result<(), RegistryError> {
        registration.validate()?;
        let mut state = self.state.write().await;
        if !state.providers.contains_key(&registration.provider_id) {
            return Err(RegistryError::ProviderNotRegistered {
                provider_id: registration.provider_id,
            });
        }
        if state.models.contains_key(&registration.model_id)
            || state.models.values().any(|model| {
                model.provider_id == registration.provider_id
                    && model.provider_model_identifier == registration.provider_model_identifier
            })
        {
            return Err(RegistryError::Conflict {
                reason: "model identity or provider-model binding already registered".into(),
            });
        }
        state.models.insert(registration.model_id, registration);
        Ok(())
    }
    /// Update metadata/operator enablement while preserving the model's provider binding.
    /// To change binding, create a new model identity and explicitly retire the old one.
    pub async fn update_model(&self, registration: ModelRegistration) -> Result<(), RegistryError> {
        registration.validate()?;
        let mut state = self.state.write().await;
        let existing =
            state
                .models
                .get(&registration.model_id)
                .ok_or(RegistryError::ModelNotFound {
                    model_id: registration.model_id,
                })?;
        if existing.provider_id != registration.provider_id
            || existing.provider_model_identifier != registration.provider_model_identifier
        {
            return Err(RegistryError::Conflict {
                reason: "model provider binding is immutable".into(),
            });
        }
        state.models.insert(registration.model_id, registration);
        Ok(())
    }
    /// Remove routing metadata, not provider memory. Evict reservations separately.
    pub async fn remove_model(
        &self,
        model_id: ModelId,
    ) -> Result<ModelRegistration, RegistryError> {
        self.state
            .write()
            .await
            .models
            .remove(&model_id)
            .ok_or(RegistryError::ModelNotFound { model_id })
    }
    /// Read original operator metadata, including enablement independent of provider health.
    pub async fn get_registration(
        &self,
        model_id: ModelId,
    ) -> Result<ModelRegistration, RegistryError> {
        self.state
            .read()
            .await
            .models
            .get(&model_id)
            .cloned()
            .ok_or(RegistryError::ModelNotFound { model_id })
    }
    fn health(&self, entry: &ProviderEntry) -> ProviderHealthObservation {
        let Some((status, checked_at)) = entry.observation else {
            return ProviderHealthObservation {
                state: ProviderHealthState::Unknown,
                checked_at: None,
            };
        };
        let state = if checked_at.elapsed() >= self.health_max_age {
            ProviderHealthState::Stale
        } else {
            match status {
                ProviderHealthStatus::Healthy => ProviderHealthState::Healthy,
                ProviderHealthStatus::Degraded => ProviderHealthState::Degraded,
                ProviderHealthStatus::Unreachable => ProviderHealthState::Unreachable,
            }
        };
        ProviderHealthObservation {
            state,
            checked_at: Some(checked_at),
        }
    }
    /// Inspect current health without initiating I/O.
    pub async fn provider_health(
        &self,
        provider_id: ProviderId,
    ) -> Result<ProviderHealthObservation, RegistryError> {
        let state = self.state.read().await;
        state
            .providers
            .get(&provider_id)
            .map(|entry| self.health(entry))
            .ok_or(RegistryError::ProviderNotRegistered { provider_id })
    }
    fn effective(&self, state: &RegistryState, model: &ModelRegistration) -> ModelRegistration {
        let mut model = model.clone();
        model.available &= state
            .providers
            .get(&model.provider_id)
            .is_some_and(|entry| self.health(entry).state == ProviderHealthState::Healthy);
        model
    }
    /// Read a routing snapshot whose availability requires fresh healthy evidence.
    pub async fn get_model(&self, model_id: ModelId) -> Result<ModelRegistration, RegistryError> {
        let state = self.state.read().await;
        state
            .models
            .get(&model_id)
            .map(|model| self.effective(&state, model))
            .ok_or(RegistryError::ModelNotFound { model_id })
    }
    /// List routing snapshots; unavailable entries remain inspectable.
    pub async fn list_models(&self) -> Vec<ModelRegistration> {
        let state = self.state.read().await;
        state
            .models
            .values()
            .map(|model| self.effective(&state, model))
            .collect()
    }
    /// Match declared capabilities only among currently routable models.
    pub async fn find_by_capability(
        &self,
        required: &BTreeSet<Capability>,
    ) -> Vec<ModelRegistration> {
        let state = self.state.read().await;
        state
            .models
            .values()
            .filter(|model| required.is_subset(&model.capabilities))
            .map(|model| self.effective(&state, model))
            .filter(|model| model.available)
            .collect()
    }
    /// Poll a stable snapshot sequentially with per-provider timeout/cancellation.
    /// Concurrent inventory operations remain available. Newer polls/provider instances
    /// fence out older in-flight results; cancelled polls do not invent failure evidence.
    pub async fn health_check_all(
        &self,
        cancel: CancellationToken,
    ) -> BTreeMap<ProviderId, ProviderHealthStatus> {
        let snapshot = {
            let mut state = self.state.write().await;
            let Some(revision) = state.poll_revision.checked_add(1) else {
                return BTreeMap::new();
            };
            state.poll_revision = revision;
            state
                .providers
                .iter_mut()
                .map(|(id, entry)| {
                    entry.poll_revision = revision;
                    (*id, entry.provider.clone(), revision)
                })
                .collect::<Vec<_>>()
        };
        let mut statuses = BTreeMap::new();
        for (id, provider, revision) in snapshot {
            let result = tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                result = tokio::time::timeout(self.health_timeout, provider.health_check(cancel.clone())) => result,
            };
            let status = match result {
                Ok(Ok(report)) if report.provider_id == id => report.status,
                _ => ProviderHealthStatus::Unreachable,
            };
            let mut state = self.state.write().await;
            if let Some(entry) = state.providers.get_mut(&id)
                && entry.poll_revision == revision
                && Arc::ptr_eq(&entry.provider, &provider)
            {
                entry.observation = Some((status, Instant::now()));
                statuses.insert(id, status);
            }
        }
        statuses
    }
}
