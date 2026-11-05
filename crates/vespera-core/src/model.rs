use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{Capability, DomainError, ModelId, ProviderId, ResourceEstimate, validate_nonempty};

/// Configuration categories only; provider implementations are deferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Ollama,
    OpenAiCompatible,
    Custom,
}

/// Reported availability, kept distinct from estimated resources and residency.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelAvailability {
    #[default]
    Unknown,
    Available,
    Unavailable,
}

/// Model registration metadata; registering this data does not load a model.
///
/// At least one capability is required for capability-based routing. Every
/// assigned responsibility must be included in that advertised capability set.
/// Public fields require revalidation after mutation and at input boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDescriptor {
    pub id: ModelId,
    pub provider_id: ProviderId,
    pub name: String,
    /// Provider-specific model identifier, not the model's domain UUID.
    pub identifier: String,
    pub capabilities: BTreeSet<Capability>,
    #[serde(default)]
    pub context_window_tokens: Option<u32>,
    #[serde(default)]
    pub parameter_count: Option<u64>,
    #[serde(default)]
    pub quantization: Option<String>,
    #[serde(default)]
    pub resources: ResourceEstimate,
    #[serde(default)]
    pub availability: ModelAvailability,
    #[serde(default)]
    pub responsibilities: BTreeSet<Capability>,
}

impl ModelDescriptor {
    pub fn validate(&self) -> Result<(), DomainError> {
        self.id.validate()?;
        self.provider_id.validate()?;
        validate_nonempty(&self.name, "model.name")?;
        validate_nonempty(&self.identifier, "model.identifier")?;
        if self.capabilities.is_empty() {
            return Err(DomainError::EmptySet {
                field: "model.capabilities",
            });
        }
        if self.context_window_tokens == Some(0) {
            return Err(DomainError::ZeroValue {
                field: "model.context_window_tokens",
            });
        }
        if self.parameter_count == Some(0) {
            return Err(DomainError::ZeroValue {
                field: "model.parameter_count",
            });
        }
        if let Some(quantization) = &self.quantization {
            validate_nonempty(quantization, "model.quantization")?;
        }
        for responsibility in &self.responsibilities {
            if !self.capabilities.contains(responsibility) {
                return Err(DomainError::UnsupportedResponsibility {
                    capability: responsibility.to_string(),
                });
            }
        }
        self.resources.validate()
    }
}
