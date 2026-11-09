use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{AgentId, Capability, DomainError, ModelId, validate_nonempty};

/// Persistent identity data, deliberately independent of any model selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIdentity {
    pub id: AgentId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub role: String,
    #[serde(default)]
    pub personality: String,
    #[serde(default)]
    pub goals: Vec<String>,
}

impl AgentIdentity {
    pub fn validate(&self) -> Result<(), DomainError> {
        self.id.validate()?;
        validate_nonempty(&self.name, "agent.identity.name")?;
        validate_nonempty(&self.role, "agent.identity.role")
    }
}

/// Model binding may change without replacing an agent's identity or namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelSelectionPolicy {
    Fixed {
        model_id: ModelId,
    },
    FixedWithCapabilities {
        model_id: ModelId,
        required: BTreeSet<Capability>,
    },
    Capability {
        required: BTreeSet<Capability>,
    },
}

impl ModelSelectionPolicy {
    /// Returns the bound model for either fixed policy.
    pub fn fixed_model(&self) -> Option<ModelId> {
        match self {
            Self::Fixed { model_id } | Self::FixedWithCapabilities { model_id, .. } => {
                Some(*model_id)
            }
            Self::Capability { .. } => None,
        }
    }

    /// Checks the binding and capability requirements against model metadata.
    pub fn accepts(&self, id: ModelId, capabilities: &BTreeSet<Capability>) -> bool {
        match self {
            Self::Fixed { model_id } => *model_id == id,
            Self::FixedWithCapabilities { model_id, required } => {
                *model_id == id && required.is_subset(capabilities)
            }
            Self::Capability { required } => required.is_subset(capabilities),
        }
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::FixedWithCapabilities { required, .. } | Self::Capability { required }
                if required.is_empty() =>
            {
                Err(DomainError::EmptySet {
                    field: "agent.model_policy.required",
                })
            }
            Self::Fixed { model_id } | Self::FixedWithCapabilities { model_id, .. } => {
                model_id.validate()
            }
            Self::Capability { .. } => Ok(()),
        }
    }
}

/// Explicit grants interpreted by a future authorized execution boundary.
///
/// There is no default grant, tool execution API, or implicit escalation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Communicate,
    MemoryRead,
    MemoryWrite,
    Delegate,
    SimulationAct,
}

/// Declared agent state; a definition alone does not start execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    #[default]
    Suspended,
    Idle,
    Running,
    Failed,
}

/// Declarative agent configuration with no implicit permissions or execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDefinition {
    pub identity: AgentIdentity,
    pub model_policy: ModelSelectionPolicy,
    #[serde(default)]
    pub permissions: BTreeSet<Permission>,
    #[serde(default)]
    pub status: AgentStatus,
}

impl AgentDefinition {
    pub fn validate(&self) -> Result<(), DomainError> {
        self.identity.validate()?;
        self.model_policy.validate()
    }
}
