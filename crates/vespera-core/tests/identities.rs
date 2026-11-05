use std::{collections::BTreeSet, fmt::Debug, str::FromStr};

use serde::{Serialize, de::DeserializeOwned};
use vespera_core::{AgentId, Capability, EventId, MemoryId, ModelId, ProviderId, TaskId};

const ID: &str = "12345678-1234-4234-8234-123456789abc";
const NIL: &str = "00000000-0000-0000-0000-000000000000";

fn check_id<T>()
where
    T: FromStr + Serialize + DeserializeOwned + PartialEq + Debug,
    T::Err: Debug,
{
    let parsed: T = ID.parse().unwrap();
    let encoded = serde_json::to_string(&parsed).unwrap();
    assert_eq!(encoded, format!("\"{ID}\""));
    assert_eq!(serde_json::from_str::<T>(&encoded).unwrap(), parsed);
    for malformed in [
        NIL,
        "",
        "invalid",
        "12345678-1234",
        " 12345678-1234-4234-8234-123456789abc",
    ] {
        assert!(malformed.parse::<T>().is_err(), "accepted {malformed}");
        assert!(serde_json::from_str::<T>(&format!("\"{malformed}\"")).is_err());
    }
    for wrong_type in ["null", "123", "{}", "[]"] {
        assert!(serde_json::from_str::<T>(wrong_type).is_err());
    }
}

#[test]
fn all_domain_ids_are_nonnil_uuid_strings() {
    check_id::<AgentId>();
    check_id::<ModelId>();
    check_id::<ProviderId>();
    check_id::<TaskId>();
    check_id::<EventId>();
    check_id::<MemoryId>();
}

#[test]
fn freshly_generated_ids_satisfy_invariant() {
    assert!(AgentId::new().validate().is_ok());
    assert!(ModelId::new().validate().is_ok());
    assert!(ProviderId::new().validate().is_ok());
    assert!(TaskId::new().validate().is_ok());
    assert!(EventId::new().validate().is_ok());
    assert!(MemoryId::new().validate().is_ok());
}

#[test]
fn capability_grammar_and_boundary_lengths_are_exact() {
    for valid in ["a", "reasoning", "code.rust-v2_0", "a..__--99"] {
        let capability = Capability::new(valid).unwrap();
        assert_eq!(capability.as_str(), valid);
        assert_eq!(capability.to_string(), valid);
        assert_eq!(valid.parse::<Capability>().unwrap(), capability);
        let encoded = serde_json::to_string(&capability).unwrap();
        assert_eq!(
            serde_json::from_str::<Capability>(&encoded).unwrap(),
            capability
        );
    }
    assert!(Capability::new("a".repeat(64)).is_ok());
    assert!(Capability::new("a".repeat(65)).is_err());
    for invalid in [
        "",
        "1abc",
        "_abc",
        "-abc",
        ".abc",
        "A",
        "mixedCase",
        "a/b",
        "a b",
        " a",
        "a ",
        "a\n",
        "é",
        "aé",
    ] {
        assert!(Capability::new(invalid).is_err(), "accepted {invalid:?}");
        assert!(serde_json::from_value::<Capability>(serde_json::json!(invalid)).is_err());
    }
    assert!(serde_json::from_str::<Capability>("null").is_err());
}

#[test]
fn capability_sets_have_deterministic_lexical_order() {
    let capabilities: BTreeSet<Capability> = ["reasoning", "coding", "coding"]
        .into_iter()
        .map(|value| Capability::new(value).unwrap())
        .collect();
    assert_eq!(
        serde_json::to_string(&capabilities).unwrap(),
        "[\"coding\",\"reasoning\"]"
    );
}
