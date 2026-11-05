use std::{collections::BTreeSet, fmt};

use serde::{Deserialize, Serialize};

use crate::{Capability, DomainError, TaskId, validate_nonempty};

/// Task lifecycle states. Terminal states are immutable through `transition`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    #[default]
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TaskState {
    /// Apply a permitted transition, leaving the state unchanged on rejection.
    ///
    /// Permitted edges: queued → running/cancelled and running →
    /// completed/failed/cancelled. All self-transitions and terminal exits fail.
    pub fn transition(&mut self, next: Self) -> Result<(), DomainError> {
        if matches!(
            (*self, next),
            (Self::Queued, Self::Running | Self::Cancelled)
                | (
                    Self::Running,
                    Self::Completed | Self::Failed | Self::Cancelled
                )
        ) {
            *self = next;
            Ok(())
        } else {
            Err(DomainError::InvalidTaskTransition {
                from: *self,
                to: next,
            })
        }
    }
}

impl fmt::Display for TaskState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        })
    }
}

/// Task data, not a runnable inference or execution placeholder.
///
/// Directly replacing the public `state` is possible for snapshots; use
/// `TaskState::transition` for lifecycle changes at the execution boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: TaskId,
    pub prompt: String,
    #[serde(default)]
    pub required_capabilities: BTreeSet<Capability>,
    #[serde(default)]
    pub state: TaskState,
}

impl Task {
    pub fn validate(&self) -> Result<(), DomainError> {
        self.id.validate()?;
        validate_nonempty(&self.prompt, "task.prompt")
    }
}
