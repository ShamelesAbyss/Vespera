//! Strict configuration boundary for VESPERA.
//!
//! Parsing never opens a provider connection or reads a credential value. A
//! validated configuration is not proof of provider availability or model fit.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::{Host, Url};
use vespera_core::{
    AgentDefinition, AgentId, ModelDescriptor, ModelId, ModelSelectionPolicy, ProviderId,
    ProviderKind, ResourceBudget,
};

/// Maximum configuration input size, independent of the number of models.
pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
/// The configuration format supported by this foundation.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// Default-deny network declarations. Enforcement by transport is a later milestone.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkPolicy {
    #[serde(default)]
    pub allow_remote: bool,
    #[serde(default)]
    pub allow_insecure_http: bool,
}

fn default_timeout_seconds() -> u32 {
    30
}

/// A provider declaration, not an active provider connection.
///
/// Credentials are referenced by environment-variable name, never embedded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub id: ProviderId,
    pub name: String,
    pub kind: ProviderKind,
    pub endpoint: String,
    #[serde(default)]
    pub credential_env: Option<String>,
    #[serde(default = "default_timeout_seconds")]
    pub request_timeout_seconds: u32,
}

/// Complete, versioned local-first configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub network: NetworkPolicy,
    #[serde(default)]
    pub resources: ResourceBudget,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub models: Vec<ModelDescriptor>,
    #[serde(default)]
    pub agents: Vec<AgentDefinition>,
}

/// Errors deliberately exclude TOML excerpts, endpoint strings, and secrets.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not read configuration: {0}")]
    Io(#[from] io::Error),
    #[error("configuration exceeds the {MAX_CONFIG_BYTES}-byte input limit")]
    TooLarge,
    #[error("invalid TOML or configuration shape; input excerpts are suppressed")]
    Parse,
    #[error("unsupported configuration schema version {0}")]
    UnsupportedVersion(u32),
    #[error("invalid configuration field: {0}")]
    InvalidField(&'static str),
    #[error("duplicate provider ID {0}")]
    DuplicateProvider(ProviderId),
    #[error("duplicate model ID {0}")]
    DuplicateModel(ModelId),
    #[error("duplicate agent ID {0}")]
    DuplicateAgent(AgentId),
    #[error("provider {provider_id}: {issue}")]
    InvalidProvider {
        provider_id: ProviderId,
        issue: &'static str,
    },
    #[error("model {model_id}: {issue}")]
    InvalidModel {
        model_id: ModelId,
        issue: &'static str,
    },
    #[error("agent {agent_id}: {issue}")]
    InvalidAgent {
        agent_id: AgentId,
        issue: &'static str,
    },
}

impl AppConfig {
    /// Parse and validate in-memory configuration without any side effects.
    pub fn parse(input: &str) -> Result<Self, ConfigError> {
        if input.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::TooLarge);
        }
        // toml::de::Error can contain literal input. Never forward it to logs.
        let config: Self = toml::from_str(input).map_err(|_| ConfigError::Parse)?;
        config.validate()?;
        Ok(config)
    }

    /// Read at most the input-size limit plus one byte, then validate.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let file = File::open(path)?;
        let mut input = String::new();
        file.take((MAX_CONFIG_BYTES + 1) as u64)
            .read_to_string(&mut input)?;
        Self::parse(&input)
    }

    /// Validate domain invariants, networking declarations, and all references.
    /// This is required again after trusted code mutates public configuration fields.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedVersion(self.schema_version));
        }
        if self.network.allow_insecure_http && !self.network.allow_remote {
            return Err(ConfigError::InvalidField(
                "network.allow_insecure_http requires network.allow_remote",
            ));
        }
        self.resources
            .validate()
            .map_err(|_| ConfigError::InvalidField("resources"))?;

        let mut provider_ids = BTreeSet::new();
        for provider in &self.providers {
            if !provider_ids.insert(provider.id) {
                return Err(ConfigError::DuplicateProvider(provider.id));
            }
            validate_provider(provider, &self.network)?;
        }
        let mut model_ids = BTreeSet::new();
        for model in &self.models {
            if !model_ids.insert(model.id) {
                return Err(ConfigError::DuplicateModel(model.id));
            }
            let error = |issue| ConfigError::InvalidModel {
                model_id: model.id,
                issue,
            };
            model
                .validate()
                .map_err(|_| error("invalid domain metadata"))?;
            if !provider_ids.contains(&model.provider_id) {
                return Err(error("provider reference does not exist"));
            }
        }
        let mut agent_ids = BTreeSet::new();
        for agent in &self.agents {
            let agent_id = agent.identity.id;
            if !agent_ids.insert(agent_id) {
                return Err(ConfigError::DuplicateAgent(agent_id));
            }
            let error = |issue| ConfigError::InvalidAgent { agent_id, issue };
            agent
                .validate()
                .map_err(|_| error("invalid domain definition"))?;
            match &agent.model_policy {
                ModelSelectionPolicy::Fixed { model_id } => {
                    if !model_ids.contains(model_id) {
                        return Err(error("fixed model reference does not exist"));
                    }
                }
                ModelSelectionPolicy::Capability { required } => {
                    if !self
                        .models
                        .iter()
                        .any(|model| required.is_subset(&model.capabilities))
                    {
                        return Err(error("no registered model satisfies required capabilities"));
                    }
                }
            }
        }
        Ok(())
    }
}

fn validate_provider(
    provider: &ProviderConfig,
    network: &NetworkPolicy,
) -> Result<(), ConfigError> {
    let error = |issue| ConfigError::InvalidProvider {
        provider_id: provider.id,
        issue,
    };
    if provider.name.trim().is_empty() {
        return Err(error("name must not be empty"));
    }
    if !(1..=3600).contains(&provider.request_timeout_seconds) {
        return Err(error("request timeout must be between 1 and 3600 seconds"));
    }
    if let Some(name) = &provider.credential_env {
        let mut chars = name.chars();
        let first_valid = chars
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_');
        if !first_valid
            || name.len() > 128
            || !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            return Err(error("credential_env must name an environment variable"));
        }
    }
    let endpoint =
        Url::parse(&provider.endpoint).map_err(|_| error("endpoint is not a valid URL"))?;
    if !matches!(endpoint.scheme(), "http" | "https") {
        return Err(error("endpoint must use http or https"));
    }
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err(error("embedded URL credentials are forbidden"));
    }
    if endpoint.query().is_some() || endpoint.fragment().is_some() {
        return Err(error("endpoint query strings and fragments are forbidden"));
    }
    if endpoint.port() == Some(0) {
        return Err(error("endpoint port must not be zero"));
    }
    let loopback = match endpoint.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        None => return Err(error("endpoint must have a host")),
    };
    if !loopback && !network.allow_remote {
        return Err(error(
            "non-loopback endpoint denied by local-first network policy",
        ));
    }
    if !loopback && endpoint.scheme() == "http" && !network.allow_insecure_http {
        return Err(error(
            "remote plaintext HTTP requires explicit insecure-network opt-in",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../../../config/local.toml");

    #[test]
    fn sample_is_valid() {
        let config = AppConfig::parse(SAMPLE).expect("sample must validate");
        assert_eq!(config.providers.len(), 1);
        assert_eq!(config.models.len(), 1);
        assert_eq!(config.agents.len(), 1);
        assert_eq!(config.resources.max_concurrent_inferences, 1);
    }

    #[test]
    fn empty_inventory_is_allowed_without_any_activation() {
        let config = AppConfig::parse("schema_version = 1").expect("empty inventory is valid");
        assert!(config.models.is_empty());
        assert!(!config.network.allow_remote);
        assert_eq!(config.resources.vram_bytes, 0);
    }

    #[test]
    fn unknown_root_and_nested_fields_are_rejected() {
        assert!(AppConfig::parse("schema_version = 1\nmystery = true").is_err());
        assert!(
            AppConfig::parse(
                &SAMPLE.replace("kind = \"ollama\"", "kind = \"ollama\"\nmystery = true")
            )
            .is_err()
        );
        assert!(
            AppConfig::parse(&SAMPLE.replace(
                "allow_remote = false",
                "allow_remote = false\nmystery = true"
            ))
            .is_err()
        );
        assert!(
            AppConfig::parse(&SAMPLE.replace(
                "identifier = \"example-local-model\"",
                "identifier = \"example-local-model\"\nmystery = true"
            ))
            .is_err()
        );
    }

    #[test]
    fn unsupported_schema_is_rejected() {
        assert!(matches!(
            AppConfig::parse("schema_version = 2"),
            Err(ConfigError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn input_size_is_bounded() {
        assert!(matches!(
            AppConfig::parse(&" ".repeat(MAX_CONFIG_BYTES + 1)),
            Err(ConfigError::TooLarge)
        ));
    }

    #[test]
    fn duplicate_inventory_ids_are_rejected() {
        let mut config = AppConfig::parse(SAMPLE).expect("sample");
        config.providers.push(config.providers[0].clone());
        assert!(matches!(
            config.validate(),
            Err(ConfigError::DuplicateProvider(_))
        ));
        config.providers.pop();
        config.models.push(config.models[0].clone());
        assert!(matches!(
            config.validate(),
            Err(ConfigError::DuplicateModel(_))
        ));
        config.models.pop();
        config.agents.push(config.agents[0].clone());
        assert!(matches!(
            config.validate(),
            Err(ConfigError::DuplicateAgent(_))
        ));
    }

    #[test]
    fn broken_model_and_provider_references_are_rejected() {
        let mut config = AppConfig::parse(SAMPLE).expect("sample");
        config.models[0].provider_id = ProviderId::new();
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidModel { .. })
        ));
        let mut config = AppConfig::parse(SAMPLE).expect("sample");
        config.agents[0].model_policy = ModelSelectionPolicy::Fixed {
            model_id: ModelId::new(),
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidAgent { .. })
        ));
    }

    #[test]
    fn capability_policy_requires_a_matching_registered_model() {
        let mut config = AppConfig::parse(SAMPLE).expect("sample");
        config.agents[0].model_policy = ModelSelectionPolicy::Capability {
            required: ["coding".parse().expect("capability")]
                .into_iter()
                .collect(),
        };
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidAgent { .. })
        ));
        config.agents[0].model_policy = ModelSelectionPolicy::Capability {
            required: ["text-generation".parse().expect("capability")]
                .into_iter()
                .collect(),
        };
        config.validate().expect("matching declared capability");
    }

    #[test]
    fn invalid_budget_and_timeout_are_rejected() {
        let mut config = AppConfig::parse(SAMPLE).expect("sample");
        config.resources.max_concurrent_inferences = 0;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidField("resources"))
        ));
        let mut config = AppConfig::parse(SAMPLE).expect("sample");
        config.providers[0].request_timeout_seconds = 0;
        assert!(matches!(
            config.validate(),
            Err(ConfigError::InvalidProvider { .. })
        ));
    }

    #[test]
    fn local_ipv4_ipv6_and_hostname_are_allowed() {
        for endpoint in [
            "http://127.0.0.1:11434",
            "http://[::1]:11434",
            "https://localhost:1234/v1",
            "http://127.1.2.3:8080",
        ] {
            let mut config = AppConfig::parse(SAMPLE).expect("sample");
            config.providers[0].endpoint = endpoint.into();
            config.validate().expect("loopback endpoint");
        }
    }

    #[test]
    fn remote_hosts_require_explicit_opt_in() {
        for endpoint in [
            "https://example.com/v1",
            "http://192.168.1.2:1234/v1",
            "http://localhost.example.com:1234",
            "http://0.0.0.0:11434",
        ] {
            let mut config = AppConfig::parse(SAMPLE).expect("sample");
            config.providers[0].endpoint = endpoint.into();
            assert!(
                config.validate().is_err(),
                "must deny non-loopback endpoint"
            );
        }
        let mut config = AppConfig::parse(SAMPLE).expect("sample");
        config.providers[0].endpoint = "https://example.com/v1".into();
        config.network.allow_remote = true;
        config.validate().expect("explicit secure remote endpoint");
        config.providers[0].endpoint = "http://example.com/v1".into();
        assert!(config.validate().is_err());
        config.network.allow_insecure_http = true;
        config.validate().expect("explicit plaintext opt-in");
        config.network.allow_remote = false;
        assert!(config.validate().is_err());
    }

    #[test]
    fn url_secrets_and_unsafe_shapes_are_rejected_without_echoing_them() {
        for endpoint in [
            "http://user:super-secret@localhost:11434",
            "http://localhost:11434?key=super-secret",
            "http://localhost:11434#super-secret",
            "file:///tmp/provider",
            "not-a-url-super-secret",
            "http://localhost:0",
        ] {
            let mut config = AppConfig::parse(SAMPLE).expect("sample");
            config.providers[0].endpoint = endpoint.into();
            let error = config.validate().expect_err("unsafe endpoint").to_string();
            assert!(!error.contains("super-secret"));
            assert!(!error.contains(endpoint));
        }
    }

    #[test]
    fn malformed_toml_does_not_leak_credentials_in_error() {
        let error = AppConfig::parse("schema_version = 1\ncredential = \"super-secret\"")
            .expect_err("unknown inline credential")
            .to_string();
        assert!(!error.contains("super-secret"));
    }

    #[test]
    fn credential_references_are_names_not_values() {
        let mut config = AppConfig::parse(SAMPLE).expect("sample");
        for name in ["VESPERA_PROVIDER_TOKEN", "_LOCAL_KEY", "Key2"] {
            config.providers[0].credential_env = Some(name.into());
            config.validate().expect("valid environment reference");
        }
        for value in ["", "123KEY", "token-with-dashes", "key=value", "with space"] {
            config.providers[0].credential_env = Some(value.into());
            assert!(config.validate().is_err());
        }
    }

    #[test]
    fn model_replacement_does_not_change_agent_identity_or_memory_namespace() {
        use vespera_core::MemoryNamespace;
        let mut config = AppConfig::parse(SAMPLE).expect("sample");
        let id = config.agents[0].identity.id;
        let namespace = MemoryNamespace::for_agent(id).relative_path();
        let mut replacement = config.models[0].clone();
        replacement.id = ModelId::new();
        replacement.identifier = "replacement-model".into();
        config.models.push(replacement.clone());
        config.agents[0].model_policy = ModelSelectionPolicy::Fixed {
            model_id: replacement.id,
        };
        config.validate().expect("model replaced");
        assert_eq!(id, config.agents[0].identity.id);
        assert_eq!(namespace, MemoryNamespace::for_agent(id).relative_path());
    }

    #[test]
    fn many_registered_models_do_not_change_resource_budget() {
        let mut config = AppConfig::parse(SAMPLE).expect("sample");
        for _ in 0..100 {
            let mut model = config.models[0].clone();
            model.id = ModelId::new();
            config.models.push(model);
        }
        config.validate().expect("no fixed model-count limit");
        assert_eq!(config.models.len(), 101);
        assert_eq!(config.resources.max_concurrent_inferences, 1);
    }
}
