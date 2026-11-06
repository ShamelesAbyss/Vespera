//! Central model registry and health monitoring engine for VESPERA.
//!
//! Implements Milestone 4:
//! - Thread-safe central registry for any number of models.
//! - Separation of model identity from agent identity.
//! - Health tracking, capability matching, and status observation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;

use thiserror::Error;
use vespera_core::Capability;
use vespera_core::ResourceEstimate;
use vespera_core::{ModelId, ProviderId};
use vespera_providers::{CancellationToken, InferenceProvider, ProviderHealthStatus};

/// Error taxonomy for model registry operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// Model ID not found in the registry.
    #[error("model {model_id} not found in registry")]
    ModelNotFound {
        /// Missing model identifier.
        model_id: ModelId,
    },
    /// Provider ID not found in the registry.
    #[error("provider {provider_id} not registered")]
    ProviderNotRegistered {
        /// Missing provider identifier.
        provider_id: ProviderId,
    },
    /// Model registration conflict or duplicate ID.
    #[error("model registration conflict: {reason}")]
    Conflict {
        /// Conflict description.
        reason: String,
    },
}

/// Full registered record for an intelligence model in VESPERA.
#[derive(Debug, Clone)]
pub struct ModelRegistration {
    /// Unique model identifier.
    pub model_id: ModelId,
    /// Associated provider identifier.
    pub provider_id: ProviderId,
    /// Human-readable model name.
    pub name: String,
    /// Provider-specific model identifier string.
    pub provider_model_identifier: String,
    /// Parameter size if known.
    pub parameter_size: Option<String>,
    /// Quantization if known.
    pub quantization: Option<String>,
    /// Context window size in tokens if known.
    pub context_window: Option<u32>,
    /// Advertised capabilities.
    pub capabilities: BTreeSet<Capability>,
    /// Estimated resource requirements.
    pub estimated_resources: ResourceEstimate,
    /// Whether the model is marked available for routing.
    pub available: bool,
}

/// Thread-safe central model registry and intelligence pool.
#[derive(Clone)]
pub struct ModelRegistry {
    models: Arc<RwLock<BTreeMap<ModelId, ModelRegistration>>>,
    providers: Arc<RwLock<BTreeMap<ProviderId, Arc<dyn InferenceProvider>>>>,
}

impl fmt::Debug for ModelRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelRegistry")
            .field("models_count", &"Arc<RwLock<BTreeMap>>")
            .field("providers_count", &"Arc<RwLock<BTreeMap>>")
            .finish()
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelRegistry {
    /// Creates a new empty model registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            models: Arc::new(RwLock::new(BTreeMap::new())),
            providers: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    /// Registers an inference provider into the pool.
    pub async fn register_provider(&self, provider: Arc<dyn InferenceProvider>) {
        let mut providers = self.providers.write().await;
        providers.insert(provider.provider_id(), provider);
    }

    /// Looks up a registered inference provider by its ID.
    pub async fn get_provider(
        &self,
        provider_id: ProviderId,
    ) -> Result<Arc<dyn InferenceProvider>, RegistryError> {
        let providers = self.providers.read().await;
        providers
            .get(&provider_id)
            .cloned()
            .ok_or(RegistryError::ProviderNotRegistered { provider_id })
    }

    /// Registers a model into the intelligence pool.
    pub async fn register_model(
        &self,
        registration: ModelRegistration,
    ) -> Result<(), RegistryError> {
        let providers = self.providers.read().await;
        if !providers.contains_key(&registration.provider_id) {
            return Err(RegistryError::ProviderNotRegistered {
                provider_id: registration.provider_id,
            });
        }

        let mut models = self.models.write().await;
        models.insert(registration.model_id, registration);
        Ok(())
    }

    /// Looks up a model registration by its ID.
    pub async fn get_model(&self, model_id: ModelId) -> Result<ModelRegistration, RegistryError> {
        let models = self.models.read().await;
        models
            .get(&model_id)
            .cloned()
            .ok_or(RegistryError::ModelNotFound { model_id })
    }

    /// Lists all registered models in the intelligence pool.
    pub async fn list_models(&self) -> Vec<ModelRegistration> {
        let models = self.models.read().await;
        models.values().cloned().collect()
    }

    /// Finds models matching specific required capabilities.
    pub async fn find_by_capability(
        &self,
        required: &BTreeSet<Capability>,
    ) -> Vec<ModelRegistration> {
        let models = self.models.read().await;
        models
            .values()
            .filter(|m| m.available && required.is_subset(&m.capabilities))
            .cloned()
            .collect()
    }

    /// Runs a health check on all registered providers and updates availability.
    pub async fn health_check_all(
        &self,
        cancel: CancellationToken,
    ) -> BTreeMap<ProviderId, ProviderHealthStatus> {
        let providers = self.providers.read().await;
        let mut statuses = BTreeMap::new();

        for (provider_id, provider) in providers.iter() {
            if cancel.is_cancelled() {
                break;
            }
            match provider.health_check(cancel.clone()).await {
                Ok(report) => {
                    statuses.insert(*provider_id, report.status);
                }
                Err(_) => {
                    statuses.insert(*provider_id, ProviderHealthStatus::Unreachable);
                }
            }
        }
        statuses
    }
}
