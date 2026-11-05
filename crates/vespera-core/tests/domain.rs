use std::collections::BTreeSet;

use serde_json::{Value, json};
use vespera_core::{
    AgentDefinition, AgentId, AgentIdentity, AgentStatus, Capability, DomainError, MemoryLayer,
    MemoryNamespace, MemoryRecord, ModelAvailability, ModelDescriptor, ModelSelectionPolicy,
    Permission, ProviderKind,
};

const AGENT_ID: &str = "11111111-1111-4111-8111-111111111111";
const MODEL_ID: &str = "22222222-2222-4222-8222-222222222222";
const SECOND_MODEL_ID: &str = "33333333-3333-4333-8333-333333333333";
const PROVIDER_ID: &str = "44444444-4444-4444-8444-444444444444";
const MEMORY_ID: &str = "55555555-5555-4555-8555-555555555555";

fn model_json() -> Value {
    json!({ "id": MODEL_ID, "provider_id": PROVIDER_ID, "name": "Local reasoner", "identifier": "reasoner:q4", "capabilities": ["reasoning"] })
}

fn agent_json() -> Value {
    json!({
        "identity": { "id": AGENT_ID, "name": "Researcher", "role": "research" },
        "model_policy": { "kind": "fixed", "model_id": MODEL_ID }
    })
}

#[test]
fn optional_model_metadata_defaults_to_unknown_not_zero() {
    let model: ModelDescriptor = serde_json::from_value(model_json()).unwrap();
    model.validate().unwrap();
    assert_eq!(model.availability, ModelAvailability::Unknown);
    assert_eq!(model.context_window_tokens, None);
    assert_eq!(model.parameter_count, None);
    assert_eq!(model.quantization, None);
    assert_eq!(model.resources.ram_bytes, None);
    assert_eq!(model.resources.vram_bytes, None);
    assert!(model.responsibilities.is_empty());
    let encoded = serde_json::to_string(&model).unwrap();
    assert_eq!(
        serde_json::from_str::<ModelDescriptor>(&encoded).unwrap(),
        model
    );
}

#[test]
fn models_reject_empty_required_values_and_zero_known_metadata() {
    for (field, invalid) in [
        ("name", json!(" \n")),
        ("identifier", json!("")),
        ("capabilities", json!([])),
        ("context_window_tokens", json!(0)),
        ("parameter_count", json!(0)),
        ("quantization", json!(" ")),
    ] {
        let mut input = model_json();
        input[field] = invalid;
        let model: ModelDescriptor = serde_json::from_value(input).unwrap();
        assert!(model.validate().is_err(), "accepted invalid {field}");
    }
}

#[test]
fn responsibilities_must_be_advertised_capabilities() {
    let mut model: ModelDescriptor = serde_json::from_value(model_json()).unwrap();
    model
        .responsibilities
        .insert(Capability::new("reasoning").unwrap());
    model.validate().unwrap();
    model
        .responsibilities
        .insert(Capability::new("coding").unwrap());
    assert_eq!(
        model.validate(),
        Err(DomainError::UnsupportedResponsibility {
            capability: "coding".into()
        })
    );
}

#[test]
fn unknown_model_fields_and_resource_fields_are_rejected() {
    let mut input = model_json();
    input["unexpected"] = json!(true);
    assert!(serde_json::from_value::<ModelDescriptor>(input).is_err());
    let mut input = model_json();
    input["resources"] = json!({ "ram_btyes": 4 });
    assert!(serde_json::from_value::<ModelDescriptor>(input).is_err());
}

#[test]
fn agents_default_to_suspended_with_no_permissions() {
    let agent: AgentDefinition = serde_json::from_value(agent_json()).unwrap();
    agent.validate().unwrap();
    assert_eq!(agent.status, AgentStatus::Suspended);
    assert!(agent.permissions.is_empty());
    assert_eq!(agent.identity.description, "");
    assert_eq!(agent.identity.personality, "");
    assert!(agent.identity.goals.is_empty());
    assert_eq!(
        serde_json::from_value::<AgentDefinition>(serde_json::to_value(&agent).unwrap()).unwrap(),
        agent
    );
}

#[test]
fn rebinding_a_model_preserves_identity_namespace_and_permissions() {
    let mut agent: AgentDefinition = serde_json::from_value(agent_json()).unwrap();
    agent.permissions = BTreeSet::from([Permission::MemoryRead]);
    let identity_before = agent.identity.clone();
    let namespace_before = MemoryNamespace::for_agent(agent.identity.id);
    agent.model_policy = ModelSelectionPolicy::Fixed {
        model_id: SECOND_MODEL_ID.parse().unwrap(),
    };
    agent.validate().unwrap();
    let namespace_after = MemoryNamespace::for_agent(agent.identity.id);
    assert_eq!(identity_before, agent.identity);
    assert_eq!(namespace_before, namespace_after);
    assert_eq!(
        namespace_after.agent_id(),
        AGENT_ID.parse::<AgentId>().unwrap()
    );
    assert_eq!(
        namespace_after.relative_path(),
        format!("agents/{AGENT_ID}")
    );
    assert_eq!(agent.permissions, BTreeSet::from([Permission::MemoryRead]));
    namespace_after.validate().unwrap();
    assert_eq!(
        serde_json::from_value::<MemoryNamespace>(serde_json::to_value(namespace_after).unwrap())
            .unwrap(),
        namespace_after
    );
}

#[test]
fn capability_policy_requires_at_least_one_capability_and_is_strict() {
    let policy = ModelSelectionPolicy::Capability {
        required: BTreeSet::new(),
    };
    assert!(policy.validate().is_err());
    let policy = ModelSelectionPolicy::Capability {
        required: BTreeSet::from([Capability::new("coding").unwrap()]),
    };
    policy.validate().unwrap();
    let encoded = serde_json::to_value(&policy).unwrap();
    assert_eq!(encoded["kind"], "capability");
    assert_eq!(
        serde_json::from_value::<ModelSelectionPolicy>(encoded).unwrap(),
        policy
    );
    assert!(
        serde_json::from_value::<ModelSelectionPolicy>(
            json!({ "kind": "fixed", "model_id": MODEL_ID, "permissions": ["delegate"] })
        )
        .is_err()
    );
}

#[test]
fn agent_name_and_role_are_required_but_descriptions_are_optional() {
    for field in ["name", "role"] {
        let mut input = agent_json();
        input["identity"][field] = json!("  ");
        let agent: AgentDefinition = serde_json::from_value(input).unwrap();
        assert!(agent.validate().is_err());
    }
    let mut input = agent_json();
    input["identity"]["unexpected"] = json!("ignored?");
    assert!(serde_json::from_value::<AgentDefinition>(input).is_err());
    let identity: AgentIdentity = serde_json::from_value(agent_json()["identity"].clone()).unwrap();
    identity.validate().unwrap();
}

#[test]
fn memory_records_validate_content_and_exclude_identity_layer() {
    let mut record: MemoryRecord = serde_json::from_value(json!({
        "id": MEMORY_ID, "agent_id": AGENT_ID, "layer": "long_term",
        "content": "Verified observation", "created_at_unix_ms": 0
    }))
    .unwrap();
    record.validate().unwrap();
    assert_eq!(record.source_task_id, None);
    assert_eq!(record.layer, MemoryLayer::LongTerm);
    assert_eq!(
        serde_json::from_value::<MemoryRecord>(serde_json::to_value(&record).unwrap()).unwrap(),
        record
    );
    record.content = " \n ".into();
    assert!(record.validate().is_err());
    assert!(serde_json::from_str::<MemoryLayer>("\"identity\"").is_err());
    for layer in [
        MemoryLayer::LongTerm,
        MemoryLayer::Episodic,
        MemoryLayer::Working,
        MemoryLayer::Relationship,
        MemoryLayer::Goal,
        MemoryLayer::Observation,
    ] {
        assert_eq!(
            serde_json::from_value::<MemoryLayer>(serde_json::to_value(layer).unwrap()).unwrap(),
            layer
        );
    }
}

#[test]
fn provider_kind_and_permission_spellings_are_stable() {
    assert_eq!(
        serde_json::to_string(&ProviderKind::OpenAiCompatible).unwrap(),
        "\"open_ai_compatible\""
    );
    assert_eq!(
        serde_json::to_string(&Permission::MemoryWrite).unwrap(),
        "\"memory_write\""
    );
    assert_eq!(
        serde_json::to_string(&Permission::SimulationAct).unwrap(),
        "\"simulation_act\""
    );
    assert!(serde_json::from_str::<ProviderKind>("\"bionic\"").is_err());
}
