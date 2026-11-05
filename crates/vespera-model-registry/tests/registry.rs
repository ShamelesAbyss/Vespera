//! Integration tests for `vespera-model-registry`.

use std::collections::BTreeSet;
use std::sync::Arc;

use vespera_core::Capability;
use vespera_core::ResourceEstimate;
use vespera_core::{ModelId, ProviderId};
use vespera_model_registry::{ModelRegistration, ModelRegistry, RegistryError};
use vespera_providers::{
    CancellationToken, DiscoveredModel, DiscoveredModelMetadata, InferenceProvider,
    InferenceResponse, InferenceStream, LifecycleCapabilityState, ModelLifecycleOutcome,
    ProviderHealthReport, ProviderHealthStatus, ProviderKind, ProviderLifecycleSupport,
    ProviderTelemetrySnapshot,
};

struct MockProvider {
    id: ProviderId,
}

impl InferenceProvider for MockProvider {
    fn provider_id(&self) -> ProviderId {
        self.id
    }
    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::Custom
    }
    fn lifecycle_support(&self) -> ProviderLifecycleSupport {
        ProviderLifecycleSupport {
            model_enumeration: LifecycleCapabilityState::Unsupported,
            model_metadata: LifecycleCapabilityState::Unsupported,
            explicit_model_load: LifecycleCapabilityState::Unsupported,
            explicit_model_unload: LifecycleCapabilityState::Unsupported,
            residency_telemetry: LifecycleCapabilityState::Unsupported,
        }
    }
    fn health_check<'a>(
        &'a self,
        _cancel: CancellationToken,
    ) -> vespera_providers::BoxFuture<
        'a,
        Result<ProviderHealthReport, vespera_providers::ProviderError>,
    > {
        Box::pin(async move {
            Ok(ProviderHealthReport {
                provider_id: self.id,
                status: ProviderHealthStatus::Healthy,
                version: Some("mock-v1".to_string()),
                latency_ms: 1,
            })
        })
    }
    fn enumerate_models<'a>(
        &'a self,
        _cancel: CancellationToken,
    ) -> vespera_providers::BoxFuture<
        'a,
        Result<Vec<DiscoveredModel>, vespera_providers::ProviderError>,
    > {
        Box::pin(async move { Ok(vec![]) })
    }
    fn model_metadata<'a>(
        &'a self,
        _model_name: &'a str,
        _cancel: CancellationToken,
    ) -> vespera_providers::BoxFuture<
        'a,
        Result<DiscoveredModelMetadata, vespera_providers::ProviderError>,
    > {
        Box::pin(async move {
            Err(vespera_providers::ProviderError::UnsupportedOperation {
                provider_id: self.id,
                operation: "model_metadata",
            })
        })
    }
    fn query_residency<'a>(
        &'a self,
        _cancel: CancellationToken,
    ) -> vespera_providers::BoxFuture<
        'a,
        Result<ProviderTelemetrySnapshot, vespera_providers::ProviderError>,
    > {
        Box::pin(async move {
            Err(vespera_providers::ProviderError::UnsupportedOperation {
                provider_id: self.id,
                operation: "query_residency",
            })
        })
    }
    fn load_model<'a>(
        &'a self,
        _model_name: &'a str,
        _keep_alive: Option<std::time::Duration>,
        _cancel: CancellationToken,
    ) -> vespera_providers::BoxFuture<
        'a,
        Result<ModelLifecycleOutcome, vespera_providers::ProviderError>,
    > {
        Box::pin(async move {
            Err(vespera_providers::ProviderError::UnsupportedOperation {
                provider_id: self.id,
                operation: "load_model",
            })
        })
    }
    fn unload_model<'a>(
        &'a self,
        _model_name: &'a str,
        _cancel: CancellationToken,
    ) -> vespera_providers::BoxFuture<
        'a,
        Result<ModelLifecycleOutcome, vespera_providers::ProviderError>,
    > {
        Box::pin(async move {
            Err(vespera_providers::ProviderError::UnsupportedOperation {
                provider_id: self.id,
                operation: "unload_model",
            })
        })
    }
    fn infer<'a>(
        &'a self,
        _request: vespera_providers::BoundedInferenceRequest,
        _cancel: CancellationToken,
    ) -> vespera_providers::BoxFuture<'a, Result<InferenceResponse, vespera_providers::ProviderError>>
    {
        Box::pin(async move {
            Err(vespera_providers::ProviderError::UnsupportedOperation {
                provider_id: self.id,
                operation: "infer",
            })
        })
    }
    fn infer_stream<'a>(
        &'a self,
        _request: vespera_providers::BoundedInferenceRequest,
        _cancel: CancellationToken,
    ) -> vespera_providers::BoxFuture<
        'a,
        Result<InferenceStream<'a>, vespera_providers::ProviderError>,
    > {
        Box::pin(async move {
            Err(vespera_providers::ProviderError::UnsupportedOperation {
                provider_id: self.id,
                operation: "load_model",
            })
        })
    }
}

#[tokio::test]
async fn registry_manages_models_and_checks_health() {
    let registry = ModelRegistry::new();
    let provider_id: ProviderId = "0195f000-0000-7000-8000-000000000001"
        .parse()
        .expect("valid provider id");
    let model_id: ModelId = "0195f000-0000-7000-8000-000000000002"
        .parse()
        .expect("valid model id");

    let provider = Arc::new(MockProvider { id: provider_id });
    registry.register_provider(provider).await;

    let mut caps = BTreeSet::new();
    caps.insert("text-generation".parse::<Capability>().unwrap());

    let reg = ModelRegistration {
        model_id,
        provider_id,
        name: "test-model".to_string(),
        provider_model_identifier: "mock:test-model".to_string(),
        parameter_size: Some("3B".to_string()),
        quantization: Some("Q4_K_M".to_string()),
        context_window: Some(4096),
        capabilities: caps.clone(),
        estimated_resources: ResourceEstimate {
            ram_bytes: Some(2048),
            vram_bytes: None,
        },
        available: true,
    };

    registry
        .register_model(reg)
        .await
        .expect("registration succeeds");

    let fetched = registry.get_model(model_id).await.expect("model found");
    assert_eq!(fetched.name, "test-model");

    let found = registry.find_by_capability(&caps).await;
    assert_eq!(found.len(), 1);

    let health = registry.health_check_all(CancellationToken::new()).await;
    assert_eq!(
        health.get(&provider_id),
        Some(&ProviderHealthStatus::Healthy)
    );
}

#[tokio::test]
async fn registering_model_without_provider_fails() {
    let registry = ModelRegistry::new();
    let provider_id: ProviderId = "0195f000-0000-7000-8000-000000000001"
        .parse()
        .expect("valid provider id");
    let model_id: ModelId = "0195f000-0000-7000-8000-000000000002"
        .parse()
        .expect("valid model id");

    let reg = ModelRegistration {
        model_id,
        provider_id,
        name: "orphan-model".to_string(),
        provider_model_identifier: "mock:orphan".to_string(),
        parameter_size: None,
        quantization: None,
        context_window: None,
        capabilities: BTreeSet::new(),
        estimated_resources: ResourceEstimate {
            ram_bytes: None,
            vram_bytes: None,
        },
        available: true,
    };

    let err = registry.register_model(reg).await.expect_err("should fail");
    assert!(matches!(err, RegistryError::ProviderNotRegistered { .. }));
}
