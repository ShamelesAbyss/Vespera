//! Operator-declared bounds for retained in-process agent data.
use super::{AgentRuntimeError, RuntimeState};
use serde::Serialize;
use vespera_core::{AgentDefinition, ModelSelectionPolicy, Task};

/// Bounds logical retained content and record counts; these are not measured host RAM.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeLimits {
    /// Registered agents.
    pub max_agents: usize,
    /// Accepted task history, including terminal records.
    pub max_tasks: usize,
    /// Memory records per agent.
    pub max_memories_per_agent: usize,
    /// Delivered messages per inbox.
    pub max_messages_per_agent: usize,
    /// Maximum textual bytes in an agent definition, task, memory or message.
    pub max_record_bytes: usize,
    /// Aggregate retained textual bytes, excluding caller clones and allocation overhead.
    pub max_retained_content_bytes: usize,
}
impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_agents: 64,
            max_tasks: 1024,
            max_memories_per_agent: 256,
            max_messages_per_agent: 256,
            max_record_bytes: 64 * 1024,
            max_retained_content_bytes: 64 * 1024 * 1024,
        }
    }
}
/// Inspectable local counts and logical content accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RuntimeUsage {
    /// Registered agents.
    pub agents: usize,
    /// Accepted task records.
    pub tasks: usize,
    /// Charged retained textual content, not physical process RAM.
    pub retained_content_bytes: usize,
}
impl RuntimeLimits {
    pub(super) fn validate(&self) -> Result<(), AgentRuntimeError> {
        for (field, value) in [
            ("max_agents", self.max_agents),
            ("max_tasks", self.max_tasks),
            ("max_memories_per_agent", self.max_memories_per_agent),
            ("max_messages_per_agent", self.max_messages_per_agent),
            ("max_record_bytes", self.max_record_bytes),
            (
                "max_retained_content_bytes",
                self.max_retained_content_bytes,
            ),
        ] {
            if value == 0 {
                return Err(AgentRuntimeError::InvalidLimits { field });
            }
        }
        Ok(())
    }
    pub(super) fn check_record(&self, bytes: usize) -> Result<(), AgentRuntimeError> {
        if bytes > self.max_record_bytes {
            Err(capacity("record_bytes"))
        } else {
            Ok(())
        }
    }
    pub(super) fn reserve(
        &self,
        state: &mut RuntimeState,
        bytes: usize,
    ) -> Result<(), AgentRuntimeError> {
        self.check_record(bytes)?;
        let total = state
            .retained_content_bytes
            .checked_add(bytes)
            .ok_or_else(|| capacity("retained_content_bytes"))?;
        if total > self.max_retained_content_bytes {
            return Err(capacity("retained_content_bytes"));
        }
        state.retained_content_bytes = total;
        Ok(())
    }
}
pub(super) fn capacity(resource: &'static str) -> AgentRuntimeError {
    AgentRuntimeError::Capacity { resource }
}
fn sum<'a>(mut values: impl Iterator<Item = &'a str>) -> Result<usize, AgentRuntimeError> {
    values.try_fold(0usize, |total, value| {
        total
            .checked_add(value.len())
            .ok_or_else(|| capacity("record_bytes"))
    })
}
pub(super) fn policy_bytes(policy: &ModelSelectionPolicy) -> Result<usize, AgentRuntimeError> {
    match policy {
        ModelSelectionPolicy::Fixed { .. } => Ok(0),
        ModelSelectionPolicy::Capability { required } => {
            sum(required.iter().map(|capability| capability.as_str()))
        }
    }
}
pub(super) fn agent_bytes(agent: &AgentDefinition) -> Result<usize, AgentRuntimeError> {
    let identity = &agent.identity;
    let bytes = sum([
        identity.name.as_str(),
        identity.description.as_str(),
        identity.role.as_str(),
        identity.personality.as_str(),
    ]
    .into_iter()
    .chain(identity.goals.iter().map(String::as_str)))?;
    bytes
        .checked_add(policy_bytes(&agent.model_policy)?)
        .ok_or_else(|| capacity("record_bytes"))
}
pub(super) fn task_bytes(task: &Task) -> Result<usize, AgentRuntimeError> {
    task.prompt
        .len()
        .checked_add(sum(task
            .required_capabilities
            .iter()
            .map(|capability| capability.as_str()))?)
        .ok_or_else(|| capacity("record_bytes"))
}
