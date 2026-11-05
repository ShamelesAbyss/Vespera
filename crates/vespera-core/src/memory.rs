use serde::{Deserialize, Serialize};

use crate::{AgentId, DomainError, MemoryId, TaskId, validate_nonempty};

/// A logical memory address derived solely from an agent ID, never a model ID.
///
/// This type does not create directories, read files, or implement persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryNamespace {
    agent_id: AgentId,
}

impl MemoryNamespace {
    #[must_use]
    pub fn for_agent(agent_id: AgentId) -> Self {
        Self { agent_id }
    }

    #[must_use]
    pub fn agent_id(&self) -> AgentId {
        self.agent_id
    }

    /// Portable logical relative path with no caller-controlled path segments.
    #[must_use]
    pub fn relative_path(&self) -> String {
        format!("agents/{}", self.agent_id)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        self.agent_id.validate()
    }
}

/// Agent-writable memory categories; identity/configuration is intentionally absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLayer {
    LongTerm,
    Episodic,
    Working,
    Relationship,
    Goal,
    Observation,
}

/// Memory record data only: no persistence, retrieval, or permission enforcement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryRecord {
    pub id: MemoryId,
    pub agent_id: AgentId,
    pub layer: MemoryLayer,
    pub content: String,
    /// Caller-supplied wall-clock timestamp, not an independently verified clock.
    pub created_at_unix_ms: u64,
    #[serde(default)]
    pub source_task_id: Option<TaskId>,
}

impl MemoryRecord {
    pub fn validate(&self) -> Result<(), DomainError> {
        self.id.validate()?;
        self.agent_id.validate()?;
        if let Some(task_id) = self.source_task_id {
            task_id.validate()?;
        }
        validate_nonempty(&self.content, "memory.content")
    }
}
