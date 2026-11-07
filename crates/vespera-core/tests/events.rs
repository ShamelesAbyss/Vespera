use serde_json::json;
use vespera_core::{
    AgentId, DomainError, EVENT_SCHEMA_VERSION, EventEnvelope, EventKind, MemoryId, MemoryLayer,
    ModelId, ProviderId, ResourceKind, TaskId,
};

const ID: &str = "12345678-1234-4234-8234-123456789abc";

fn event_json() -> serde_json::Value {
    json!({
        "schema_version": 1,
        "event_id": ID,
        "occurred_at_unix_ms": 1700000000000_u64,
        "source": "test.fixture",
        "kind": { "kind": "simulation_advanced", "simulation_id": "village", "tick": 42 }
    })
}

#[test]
fn every_event_payload_has_typed_round_trip() {
    let provider_id: ProviderId = ID.parse().unwrap();
    let model_id: ModelId = ID.parse().unwrap();
    let agent_id: AgentId = ID.parse().unwrap();
    let memory_id: MemoryId = ID.parse().unwrap();
    let task_id: TaskId = ID.parse().unwrap();
    let variants = [
        EventKind::ProviderConnected { provider_id },
        EventKind::ModelRegistered {
            model_id,
            provider_id,
        },
        EventKind::ModelActivated { model_id },
        EventKind::ModelDeactivated { model_id },
        EventKind::InferenceStarted {
            task_id,
            model_id,
            agent_id: None,
        },
        EventKind::InferenceCompleted {
            task_id,
            model_id,
            agent_id: Some(agent_id),
        },
        EventKind::AgentCreated { agent_id },
        EventKind::AgentMessageSent {
            from_agent_id: agent_id,
            to_agent_id: agent_id,
        },
        EventKind::AgentMemoryUpdated {
            agent_id,
            memory_id,
            layer: MemoryLayer::Observation,
        },
        EventKind::TaskDelegated {
            task_id,
            from_agent_id: agent_id,
            to_agent_id: agent_id,
        },
        EventKind::TaskCompleted { task_id },
        EventKind::SimulationAdvanced {
            simulation_id: "village".into(),
            tick: u64::MAX,
        },
        EventKind::ResourceLimitReached {
            resource: ResourceKind::Ram,
            requested: 1025,
            limit: 1024,
            task_id: Some(task_id),
        },
    ];
    for kind in variants {
        kind.validate().unwrap();
        let payload = serde_json::to_value(&kind).unwrap();
        assert!(payload["kind"].is_string());
        assert_eq!(serde_json::from_value::<EventKind>(payload).unwrap(), kind);
        let mut envelope = EventEnvelope::new("test.fixture", 0, kind).unwrap();
        envelope.correlation_task_id = Some(task_id);
        assert_eq!(envelope.schema_version, 1);
        envelope.validate().unwrap();
        let encoded = serde_json::to_string(&envelope).unwrap();
        assert_eq!(
            serde_json::from_str::<EventEnvelope>(&encoded).unwrap(),
            envelope
        );
    }
}

#[test]
fn simulation_ticks_are_not_wall_clock_timestamps() {
    let envelope: EventEnvelope = serde_json::from_value(event_json()).unwrap();
    assert_eq!(envelope.occurred_at_unix_ms, 1700000000000);
    assert_eq!(
        envelope.kind,
        EventKind::SimulationAdvanced {
            simulation_id: "village".into(),
            tick: 42
        }
    );
    assert_eq!(envelope.correlation_task_id, None);
}

#[test]
fn unsupported_schema_versions_reject_validation_and_deserialization() {
    for version in [0, EVENT_SCHEMA_VERSION + 1, u32::MAX] {
        let mut input = event_json();
        input["schema_version"] = json!(version);
        assert!(serde_json::from_value::<EventEnvelope>(input).is_err());
        let mut envelope: EventEnvelope = serde_json::from_value(event_json()).unwrap();
        envelope.schema_version = version;
        assert_eq!(
            envelope.validate(),
            Err(DomainError::UnsupportedEventSchema {
                version,
                supported: EVENT_SCHEMA_VERSION
            })
        );
    }
    let mut input = event_json();
    input.as_object_mut().unwrap().remove("schema_version");
    assert!(serde_json::from_value::<EventEnvelope>(input).is_err());
}

#[test]
fn terminal_failure_kinds_require_version_two_while_legacy_envelopes_still_decode() {
    let task_id = ID.parse().unwrap();
    for kind in [
        EventKind::TaskFailed { task_id },
        EventKind::TaskCancelled { task_id },
    ] {
        let envelope = EventEnvelope::new("fixture", 0, kind).unwrap();
        assert_eq!(envelope.schema_version, 2);
        let mut wire = serde_json::to_value(&envelope).unwrap();
        assert_eq!(
            serde_json::from_value::<EventEnvelope>(wire.clone()).unwrap(),
            envelope
        );
        wire["schema_version"] = json!(1);
        assert!(serde_json::from_value::<EventEnvelope>(wire).is_err());
    }
    let mut legacy = event_json();
    assert!(serde_json::from_value::<EventEnvelope>(legacy.clone()).is_ok());
    legacy["schema_version"] = json!(2);
    assert!(serde_json::from_value::<EventEnvelope>(legacy).is_ok());
}

#[test]
fn external_task_event_requires_version_three_and_a_valid_attempt() {
    let mut wire = event_json();
    wire["schema_version"] = json!(3);
    wire["correlation_task_id"] = json!(ID);
    wire["kind"] = json!({
        "kind": "external_task_changed",
        "task_id": ID,
        "change": "claimed",
        "state": "running",
        "attempt": { "worker_id": ID, "attempt_id": ID, "generation": 1 }
    });
    let decoded = serde_json::from_value::<EventEnvelope>(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), wire);
    for version in [1, 2] {
        let mut old = wire.clone();
        old["schema_version"] = json!(version);
        assert!(serde_json::from_value::<EventEnvelope>(old).is_err());
    }
    let mut nil_attempt = wire.clone();
    nil_attempt["kind"]["attempt"]["attempt_id"] = json!("00000000-0000-0000-0000-000000000000");
    assert!(serde_json::from_value::<EventEnvelope>(nil_attempt).is_err());
    wire["kind"]["attempt"]["generation"] = json!(0);
    assert!(serde_json::from_value::<EventEnvelope>(wire).is_err());
}

#[test]
fn unknown_fields_malformed_payloads_and_missing_ids_are_rejected() {
    let mut input = event_json();
    input["unknown"] = json!(true);
    assert!(serde_json::from_value::<EventEnvelope>(input).is_err());
    let mut input = event_json();
    input["kind"]["unknown"] = json!(true);
    assert!(serde_json::from_value::<EventEnvelope>(input).is_err());
    let mut input = event_json();
    input["event_id"] = json!("00000000-0000-0000-0000-000000000000");
    assert!(serde_json::from_value::<EventEnvelope>(input).is_err());
    assert!(serde_json::from_value::<EventKind>(json!({ "kind": "provider_connected" })).is_err());
    assert!(
        serde_json::from_value::<EventKind>(
            json!({ "kind": "inference_started", "task_id": ID, "model_id": "invalid" })
        )
        .is_err()
    );
    assert!(serde_json::from_value::<EventKind>(json!({ "kind": "future_event" })).is_err());
}

#[test]
fn envelope_validates_source_and_payload_on_construction_and_decode() {
    let kind = EventKind::TaskCompleted {
        task_id: ID.parse().unwrap(),
    };
    assert!(EventEnvelope::new(" ", 0, kind).is_err());
    let kind = EventKind::SimulationAdvanced {
        simulation_id: "\n".into(),
        tick: 0,
    };
    assert!(EventEnvelope::new("test.fixture", 0, kind).is_err());
    let mut input = event_json();
    input["source"] = json!("");
    assert!(serde_json::from_value::<EventEnvelope>(input).is_err());
    let mut input = event_json();
    input["kind"]["simulation_id"] = json!(" ");
    assert!(serde_json::from_value::<EventEnvelope>(input).is_err());
}
