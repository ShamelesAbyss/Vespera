use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;
use vespera_agents::ExecutionOptions;
use vespera_core::{AgentId, Capability, Task, TaskState};

/// Validated plans are limited to sixteen steps, forty-eight total attempts and
/// 64 KiB of authored text. Dependencies must precede their consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub task: Task,
    pub coordinator: AgentId,
    pub steps: Vec<Step>,
    pub max_total_attempts: u32,
    /// Sum of requested output bounds across attempts, including failed attempts.
    pub max_total_output_tokens: u64,
    pub timeout_ms: u64,
    pub output: OutputPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub name: String,
    pub prompt: String,
    pub required_capabilities: BTreeSet<Capability>,
    /// Explicit routing allowlist in preference order. Suspended/failed agents stay so.
    pub candidates: Vec<AgentId>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub max_attempts: u32,
    pub max_output_tokens: u32,
    pub context_tokens: Option<u32>,
    pub timeout_ms: u64,
    pub temperature: Option<f32>,
}

/// Protocol-level evaluation; never a claim that generated assertions are true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutputPolicy {
    Step {
        step: String,
    },
    /// Compare complete outputs byte-for-byte, preserving disagreement evidence.
    Agreement {
        steps: Vec<String>,
        on_conflict: ConflictPolicy,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConflictPolicy {
    Fail,
    /// Operator-selected precedence, not a semantic judgment.
    Prefer {
        step: String,
    },
}

#[derive(Debug, Error)]
#[error("invalid workflow plan: {field}")]
pub struct PlanError {
    pub field: &'static str,
}
fn invalid(field: &'static str) -> PlanError {
    PlanError { field }
}
impl Workflow {
    pub fn validate(&self) -> Result<(), PlanError> {
        self.task.validate().map_err(|_| invalid("task"))?;
        self.coordinator
            .validate()
            .map_err(|_| invalid("coordinator"))?;
        if self.task.state != TaskState::Queued {
            return Err(invalid("task.state"));
        }
        if self.steps.is_empty() || self.steps.len() > 16 {
            return Err(invalid("steps"));
        }
        if !(1..=48).contains(&self.max_total_attempts) {
            return Err(invalid("max_total_attempts"));
        }
        if !(1..=1_048_576).contains(&self.max_total_output_tokens) {
            return Err(invalid("max_total_output_tokens"));
        }
        if !(1..=3_600_000).contains(&self.timeout_ms) {
            return Err(invalid("timeout_ms"));
        }
        let mut names = BTreeSet::new();
        let mut text_bytes = self.task.prompt.len();
        for step in &self.steps {
            if step.name.is_empty()
                || step.name.len() > 64
                || !step
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err(invalid("step.name"));
            }
            if names.contains(&step.name) {
                return Err(invalid("step.name.duplicate"));
            }
            let deps: BTreeSet<_> = step.dependencies.iter().collect();
            if deps.len() != step.dependencies.len() || deps.iter().any(|dep| !names.contains(*dep))
            {
                return Err(invalid("step.dependencies"));
            }
            names.insert(step.name.clone());
            if step.prompt.trim().is_empty() {
                return Err(invalid("step.prompt"));
            }
            text_bytes = text_bytes
                .checked_add(step.prompt.len())
                .ok_or(invalid("text_bytes"))?;
            if text_bytes > 64 * 1024 {
                return Err(invalid("text_bytes"));
            }
            if step.required_capabilities.is_empty() || step.required_capabilities.len() > 32 {
                return Err(invalid("step.required_capabilities"));
            }
            for capability in &step.required_capabilities {
                capability
                    .validate()
                    .map_err(|_| invalid("step.required_capabilities"))?;
            }
            if step.candidates.is_empty()
                || step.candidates.len() > 32
                || step.candidates.iter().collect::<BTreeSet<_>>().len() != step.candidates.len()
            {
                return Err(invalid("step.candidates"));
            }
            for candidate in &step.candidates {
                candidate
                    .validate()
                    .map_err(|_| invalid("step.candidates"))?;
            }
            if !(1..=3).contains(&step.max_attempts) {
                return Err(invalid("step.max_attempts"));
            }
            step.options()
                .validate()
                .map_err(|_| invalid("step.execution_bounds"))?;
        }
        match &self.output {
            OutputPolicy::Step { step } if !names.contains(step) => {
                return Err(invalid("output.step"));
            }
            OutputPolicy::Agreement { steps, on_conflict } => {
                if steps.len() < 2
                    || steps.len() > 16
                    || steps.iter().collect::<BTreeSet<_>>().len() != steps.len()
                    || steps.iter().any(|step| !names.contains(step))
                {
                    return Err(invalid("output.steps"));
                }
                if let ConflictPolicy::Prefer { step } = on_conflict
                    && !steps.contains(step)
                {
                    return Err(invalid("output.prefer"));
                }
            }
            _ => (),
        }
        let covered: BTreeSet<_> = self
            .steps
            .iter()
            .flat_map(|step| step.required_capabilities.iter().cloned())
            .collect();
        if !self.task.required_capabilities.is_subset(&covered) {
            return Err(invalid("task.required_capabilities"));
        }
        Ok(())
    }
}
impl Step {
    pub(crate) fn options(&self) -> ExecutionOptions {
        ExecutionOptions {
            max_output_tokens: self.max_output_tokens,
            context_tokens: self.context_tokens,
            timeout: std::time::Duration::from_millis(self.timeout_ms),
            temperature: self.temperature,
            max_response_bytes: 1024 * 1024,
        }
    }
}
