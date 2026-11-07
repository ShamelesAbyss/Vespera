//! Inference request invariants apply equally to constructors and deserialization.
use vespera_providers::{BoundedInferenceRequest, ChatMessage, ChatRole};
fn valid() -> serde_json::Value {
    serde_json::json!({"model_name":"fixture","messages":[{"role":"user","content":"hello"}],"max_output_tokens":32,"context_window_limit":512,"temperature":0.5,"keep_alive_secs":null})
}
#[test]
fn deserialization_revalidates_token_context_temperature_and_field_contracts() {
    let request: BoundedInferenceRequest = serde_json::from_value(valid()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), valid());
    for (field, value) in [
        ("max_output_tokens", serde_json::json!(0)),
        ("max_output_tokens", serde_json::json!(513)),
        ("context_window_limit", serde_json::json!(0)),
        ("temperature", serde_json::json!(9)),
        ("model_name", serde_json::json!("")),
        ("messages", serde_json::json!([])),
        ("unexpected", serde_json::json!(true)),
    ] {
        let mut json = valid();
        json[field] = value;
        assert!(serde_json::from_value::<BoundedInferenceRequest>(json).is_err());
    }
}
#[test]
fn request_byte_and_message_count_bounds_are_enforced() {
    let message = |content: String| ChatMessage {
        role: ChatRole::User,
        content,
    };
    assert!(
        BoundedInferenceRequest::new(
            "x".repeat(513),
            vec![message("hello".into())],
            32,
            512,
            None,
            None
        )
        .is_err()
    );
    assert!(
        BoundedInferenceRequest::new(
            "fixture",
            vec![message("x".repeat(1024 * 1024))],
            32,
            512,
            None,
            None
        )
        .is_ok()
    );
    assert!(
        BoundedInferenceRequest::new(
            "fixture",
            vec![message("x".repeat(1024 * 1024 + 1))],
            32,
            512,
            None,
            None
        )
        .is_err()
    );
    assert!(
        BoundedInferenceRequest::new(
            "fixture",
            vec![message("hello".into()); 257],
            32,
            512,
            None,
            None
        )
        .is_err()
    );
    assert!(
        BoundedInferenceRequest::new(
            "fixture",
            vec![message("hello".into())],
            32,
            512,
            Some(f32::NAN),
            None
        )
        .is_err()
    );
}

#[test]
fn endpoint_port_and_credential_reference_validation_match_configuration_contracts() {
    use vespera_core::ProviderId;
    use vespera_providers::{EndpointPolicy, SecretRef, ValidatedEndpoint};
    let id = ProviderId::new();
    assert_eq!(
        SecretRef::new(id, "mixed_Case9").unwrap().env_var_name(),
        "mixed_Case9"
    );
    assert!(SecretRef::new(id, "9bad").is_err());
    assert!(ValidatedEndpoint::parse(id, "http://127.0.0.1:0", EndpointPolicy::default()).is_err());
    assert!(
        ValidatedEndpoint::parse(
            id,
            "http://127.0.0.1:11434/path:0/",
            EndpointPolicy::default()
        )
        .is_ok()
    );
}
