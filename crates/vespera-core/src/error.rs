use crate::{ResourceKind, TaskState};
use thiserror::Error;

/// Structured failures of domain validation and pure admission calculations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    /// A UUID was malformed or nil; IDs cannot represent an absent identity.
    #[error("invalid {kind} UUID `{value}`: {reason}")]
    InvalidId {
        kind: &'static str,
        value: String,
        reason: String,
    },
    /// Capability strings must match the documented ASCII slug grammar.
    #[error(
        "invalid capability `{value}`: expected 1..=64 ASCII bytes, starting with a-z, followed by a-z, 0-9, '.', '_', or '-'"
    )]
    InvalidCapability { value: String },
    #[error("{field} must not be empty or whitespace-only")]
    EmptyField { field: &'static str },
    #[error("{field} must be greater than zero")]
    ZeroValue { field: &'static str },
    #[error("{field} must contain at least one entry")]
    EmptySet { field: &'static str },
    #[error("responsibility `{capability}` is not supported by this model")]
    UnsupportedResponsibility { capability: String },
    #[error("arithmetic overflow while calculating {resource}")]
    ResourceOverflow { resource: ResourceKind },
    #[error("{resource} request/usage {requested} exceeds budget {limit}")]
    ResourceLimitExceeded {
        resource: ResourceKind,
        requested: u64,
        limit: u64,
    },
    #[error("task cannot transition from {from} to {to}")]
    InvalidTaskTransition { from: TaskState, to: TaskState },
    #[error("unsupported event schema version {version}; latest supported version is {supported}")]
    UnsupportedEventSchema { version: u32, supported: u32 },
    #[error("event schema version {version} is too old for this event kind; requires {minimum}")]
    EventSchemaTooOld { version: u32, minimum: u32 },
    #[error("invalid workflow event field: {field}")]
    InvalidWorkflowEvidence { field: &'static str },
}
