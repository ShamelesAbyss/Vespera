use serde::{Deserialize, Serialize};

use crate::{
    AgentId, DomainError, EventId, MemoryId, MemoryLayer, ModelId, ProviderId, ResourceKind,
    TaskId, validate_nonempty,
};

/// The only event wire schema supported by this milestone.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

/// Typed event payloads, not proof that the described activity has occurred.
///
/// The `kind` discriminator uses snake_case. Payloads contain domain IDs rather
/// than untyped JSON, and do not include prompts, responses, or credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventKind {
    ProviderConnected {
        provider_id: ProviderId,
    },
    ModelRegistered {
        model_id: ModelId,
        provider_id: ProviderId,
    },
    ModelActivated {
        model_id: ModelId,
    },
    ModelDeactivated {
        model_id: ModelId,
    },
    InferenceStarted {
        task_id: TaskId,
        model_id: ModelId,
        #[serde(default)]
        agent_id: Option<AgentId>,
    },
    InferenceCompleted {
        task_id: TaskId,
        model_id: ModelId,
        #[serde(default)]
        agent_id: Option<AgentId>,
    },
    AgentCreated {
        agent_id: AgentId,
    },
    AgentMessageSent {
        from_agent_id: AgentId,
        to_agent_id: AgentId,
    },
    AgentMemoryUpdated {
        agent_id: AgentId,
        memory_id: MemoryId,
        layer: MemoryLayer,
    },
    TaskDelegated {
        task_id: TaskId,
        from_agent_id: AgentId,
        to_agent_id: AgentId,
    },
    TaskCompleted {
        task_id: TaskId,
    },
    /// Logical tick is distinct from the envelope wall-clock timestamp.
    SimulationAdvanced {
        simulation_id: String,
        tick: u64,
    },
    ResourceLimitReached {
        resource: ResourceKind,
        requested: u64,
        limit: u64,
        #[serde(default)]
        task_id: Option<TaskId>,
    },
}

impl EventKind {
    pub fn validate(&self) -> Result<(), DomainError> {
        // All domain IDs are invariant-preserving newtypes. A simulation name
        // is the sole free-form identifier in these payloads.
        if let Self::SimulationAdvanced { simulation_id, .. } = self {
            validate_nonempty(simulation_id, "event.simulation_id")?;
        }
        Ok(())
    }
}

/// A validated version-1 event wire envelope, with no publication side effects.
///
/// `new` builds caller-supplied data; it does not observe activity, consult a
/// clock, append to an event log, or publish to subscribers. Public field changes
/// require explicit revalidation. Deserialization always validates the envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "EventEnvelopeWire")]
pub struct EventEnvelope {
    pub schema_version: u32,
    pub event_id: EventId,
    /// Caller-supplied Unix milliseconds; zero (the epoch) is valid.
    pub occurred_at_unix_ms: u64,
    pub correlation_task_id: Option<TaskId>,
    pub source: String,
    pub kind: EventKind,
}

impl EventEnvelope {
    /// Construct data with a new envelope ID and the supported schema version.
    pub fn new(
        source: impl Into<String>,
        occurred_at_unix_ms: u64,
        kind: EventKind,
    ) -> Result<Self, DomainError> {
        let envelope = Self {
            schema_version: EVENT_SCHEMA_VERSION,
            event_id: EventId::new(),
            occurred_at_unix_ms,
            correlation_task_id: None,
            source: source.into(),
            kind,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.schema_version != EVENT_SCHEMA_VERSION {
            return Err(DomainError::UnsupportedEventSchema {
                version: self.schema_version,
                supported: EVENT_SCHEMA_VERSION,
            });
        }
        self.event_id.validate()?;
        if let Some(task_id) = self.correlation_task_id {
            task_id.validate()?;
        }
        validate_nonempty(&self.source, "event.source")?;
        self.kind.validate()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventEnvelopeWire {
    schema_version: u32,
    event_id: EventId,
    occurred_at_unix_ms: u64,
    #[serde(default)]
    correlation_task_id: Option<TaskId>,
    source: String,
    kind: EventKind,
}

impl TryFrom<EventEnvelopeWire> for EventEnvelope {
    type Error = DomainError;

    fn try_from(wire: EventEnvelopeWire) -> Result<Self, Self::Error> {
        let envelope = Self {
            schema_version: wire.schema_version,
            event_id: wire.event_id,
            occurred_at_unix_ms: wire.occurred_at_unix_ms,
            correlation_task_id: wire.correlation_task_id,
            source: wire.source,
            kind: wire.kind,
        };
        envelope.validate()?;
        Ok(envelope)
    }
}
