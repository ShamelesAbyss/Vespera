//! Deterministic scheduler integration tests; no live provider is required.

use std::collections::BTreeSet;
use std::sync::Arc;

use vespera_core::ResourceEstimate;
use vespera_core::{ModelId, ProviderId};
use vespera_model_registry::{ModelRegistration, ModelRegistry};
use vespera_providers::{
    CancellationToken, DiscoveredModel, DiscoveredModelMetadata, InferenceProvider,
    InferenceResponse, InferenceStream, LifecycleCapabilityState, ModelLifecycleOutcome,
    ProviderHealthReport, ProviderHealthStatus, ProviderKind, ProviderLifecycleSupport,
    ProviderTelemetrySnapshot,
};

struct MockProvider {
    id: ProviderId,
}

#[tokio::test]
async fn unknown_estimates_are_rejected_without_changing_accounting() {
    use std::time::Duration;
    use vespera_core::ResourceBudget;
    use vespera_scheduler::{ResourceScheduler, SchedulerError, SchedulingPolicy};
    let registry = Arc::new(ModelRegistry::new());
    let provider_id = ProviderId::new();
    registry
        .register_provider(Arc::new(MockProvider { id: provider_id }))
        .await;
    let scheduler = ResourceScheduler::new(
        registry.clone(),
        SchedulingPolicy {
            budget: ResourceBudget::default(),
            default_keep_alive: Duration::from_secs(30),
            evict_idle_on_pressure: true,
        },
    );
    for (ram_bytes, vram_bytes) in [(None, None), (Some(1), None), (None, Some(0))] {
        let model_id = ModelId::new();
        registry
            .register_model(ModelRegistration {
                model_id,
                provider_id,
                name: "test".into(),
                provider_model_identifier: model_id.to_string(),
                parameter_size: None,
                quantization: None,
                context_window: Some(512),
                capabilities: BTreeSet::new(),
                estimated_resources: ResourceEstimate {
                    ram_bytes,
                    vram_bytes,
                },
                available: true,
            })
            .await
            .unwrap();
        assert!(matches!(
            scheduler
                .admit_and_prepare(model_id, None, CancellationToken::new())
                .await,
            Err(SchedulerError::UnknownResources { .. })
        ));
        let usage = scheduler.usage_snapshot().await;
        assert_eq!(
            (usage.ram_bytes, usage.vram_bytes, usage.active_inferences),
            (0, 0, 0)
        );
        assert!(scheduler.resident_models().await.is_empty());
    }
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
