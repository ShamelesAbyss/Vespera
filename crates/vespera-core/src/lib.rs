//! Side-effect-free domain contracts for VESPERA.
//!
//! This crate provides identities, validation, checked resource arithmetic, task
//! lifecycle rules, and event data. It does not execute inference, connect to
//! providers, enforce hardware limits, persist memory, or run simulations.
//!
//! UUID identities and capabilities validate during deserialization. Other
//! public-field records must be explicitly validated at trust boundaries after
//! deserialization and after mutation. Event envelopes additionally validate on
//! deserialization. Validation is not authorization: permissions are explicit
//! data for a future execution boundary, never implicitly granted here.

mod agent;
mod capability;
mod error;
mod event;
mod id;
mod memory;
mod model;
mod resource;
mod task;

pub use agent::{AgentDefinition, AgentIdentity, AgentStatus, ModelSelectionPolicy, Permission};
pub use capability::Capability;
pub use error::DomainError;
pub use event::{EVENT_SCHEMA_VERSION, EventEnvelope, EventKind};
pub use id::{AgentId, EventId, MemoryId, ModelId, ProviderId, TaskId};
pub use memory::{MemoryLayer, MemoryNamespace, MemoryRecord};
pub use model::{ModelAvailability, ModelDescriptor, ProviderKind};
pub use resource::{
    ResourceBudget, ResourceEstimate, ResourceKind, ResourceRequest, ResourceUsage,
};
pub use task::{Task, TaskState};

/// Reject empty or whitespace-only human-readable fields without rewriting them.
pub(crate) fn validate_nonempty(value: &str, field: &'static str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        Err(DomainError::EmptyField { field })
    } else {
        Ok(())
    }
}
