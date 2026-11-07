//! Integration tests for `vespera-model-registry`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    mode: AtomicUsize,
    entered: tokio::sync::Notify,
    proceed: tokio::sync::Notify,
}
impl MockProvider {
    fn new(id: ProviderId) -> Self {
        Self {
            id,
            mode: AtomicUsize::new(0),
            entered: tokio::sync::Notify::new(),
            proceed: tokio::sync::Notify::new(),
        }
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
            let mode = self.mode.load(Ordering::SeqCst);
            if mode == 3 {
                self.entered.notify_one();
                self.proceed.notified().await;
            }
            if mode == 1 {
                return Err(vespera_providers::ProviderError::ConnectionFailed {
                    provider_id: self.id,
                    sanitized_message: "fixture".into(),
                });
            }
            Ok(ProviderHealthReport {
                provider_id: if mode == 4 {
                    ProviderId::new()
                } else {
                    self.id
                },
                status: if mode == 2 {
                    ProviderHealthStatus::Degraded
                } else {
                    ProviderHealthStatus::Healthy
                },
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

    let provider = Arc::new(MockProvider::new(provider_id));
    registry.register_provider(provider).await.unwrap();
    registry.health_check_all(CancellationToken::new()).await;

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

fn model(provider_id: ProviderId, model_id: ModelId) -> ModelRegistration {
    ModelRegistration {
        model_id,
        provider_id,
        name: "fixture".into(),
        provider_model_identifier: model_id.to_string(),
        parameter_size: None,
        quantization: None,
        context_window: Some(512),
        capabilities: BTreeSet::new(),
        estimated_resources: ResourceEstimate {
            ram_bytes: Some(100),
            vram_bytes: Some(0),
        },
        available: true,
    }
}

#[tokio::test]
async fn duplicate_registration_is_atomic_and_binding_changes_are_explicit() {
    let registry = ModelRegistry::new();
    let provider = Arc::new(MockProvider::new(ProviderId::new()));
    registry.register_provider(provider.clone()).await.unwrap();
    assert!(matches!(
        registry.register_provider(provider.clone()).await,
        Err(RegistryError::Conflict { .. })
    ));
    let original = model(provider.id, ModelId::new());
    let (a, b) = tokio::join!(
        registry.register_model(original.clone()),
        registry.register_model(original.clone())
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let mut alias = original.clone();
    alias.model_id = ModelId::new();
    assert!(matches!(
        registry.register_model(alias).await,
        Err(RegistryError::Conflict { .. })
    ));
    let mut update = original.clone();
    update.name = "updated".into();
    registry.update_model(update.clone()).await.unwrap();
    assert_eq!(
        registry
            .get_registration(original.model_id)
            .await
            .unwrap()
            .name,
        "updated"
    );
    update.provider_model_identifier = "other-model".into();
    assert!(matches!(
        registry.update_model(update).await,
        Err(RegistryError::Conflict { .. })
    ));
    assert_eq!(
        registry
            .get_registration(original.model_id)
            .await
            .unwrap()
            .provider_model_identifier,
        original.provider_model_identifier
    );
    assert!(registry.remove_provider(provider.id).await.is_err());
    assert_eq!(
        registry.remove_model(original.model_id).await.unwrap().name,
        "updated"
    );
    assert!(registry.get_model(original.model_id).await.is_err());
    registry.remove_provider(provider.id).await.unwrap();
    assert!(registry.get_provider(provider.id).await.is_err());
}

#[tokio::test(start_paused = true)]
async fn effective_availability_expires_and_recovers_without_overriding_operator_disable() {
    use std::time::Duration;
    use vespera_model_registry::ProviderHealthState;
    let registry = ModelRegistry::new();
    let provider = Arc::new(MockProvider::new(ProviderId::new()));
    registry.register_provider(provider.clone()).await.unwrap();
    let mut enabled = model(provider.id, ModelId::new());
    let disabled_id = ModelId::new();
    let mut disabled = model(provider.id, disabled_id);
    disabled.available = false;
    registry.register_model(enabled.clone()).await.unwrap();
    registry.register_model(disabled).await.unwrap();
    assert_eq!(
        registry.provider_health(provider.id).await.unwrap().state,
        ProviderHealthState::Unknown
    );
    assert!(
        !registry
            .get_model(enabled.model_id)
            .await
            .unwrap()
            .available
    );
    assert!(
        registry
            .get_registration(enabled.model_id)
            .await
            .unwrap()
            .available
    );
    registry.health_check_all(CancellationToken::new()).await;
    assert!(
        registry
            .get_model(enabled.model_id)
            .await
            .unwrap()
            .available
    );
    assert!(!registry.get_model(disabled_id).await.unwrap().available);
    tokio::time::advance(Duration::from_secs(31)).await;
    assert_eq!(
        registry.provider_health(provider.id).await.unwrap().state,
        ProviderHealthState::Stale
    );
    assert!(
        registry
            .find_by_capability(&BTreeSet::new())
            .await
            .is_empty()
    );
    for (mode, expected) in [
        (1, ProviderHealthState::Unreachable),
        (2, ProviderHealthState::Degraded),
        (4, ProviderHealthState::Unreachable),
        (0, ProviderHealthState::Healthy),
    ] {
        provider.mode.store(mode, Ordering::SeqCst);
        registry.health_check_all(CancellationToken::new()).await;
        let observation = registry.provider_health(provider.id).await.unwrap();
        assert_eq!(observation.state, expected);
        assert!(observation.checked_at.is_some());
        assert_eq!(
            registry
                .get_model(enabled.model_id)
                .await
                .unwrap()
                .available,
            mode == 0
        );
        assert!(!registry.get_model(disabled_id).await.unwrap().available);
    }
    enabled.available = false;
    registry.update_model(enabled.clone()).await.unwrap();
    registry.health_check_all(CancellationToken::new()).await;
    assert!(
        !registry
            .get_model(enabled.model_id)
            .await
            .unwrap()
            .available
    );
}

#[tokio::test(start_paused = true)]
async fn stalled_health_does_not_lock_inventory_and_old_results_are_fenced() {
    use vespera_model_registry::ProviderHealthState;
    let registry = Arc::new(ModelRegistry::new());
    let provider = Arc::new(MockProvider::new(ProviderId::new()));
    registry.register_provider(provider.clone()).await.unwrap();
    provider.mode.store(3, Ordering::SeqCst);
    let r = registry.clone();
    let old_poll = tokio::spawn(async move { r.health_check_all(CancellationToken::new()).await });
    provider.entered.notified().await;
    let registration = model(provider.id, ModelId::new());
    registry.register_model(registration.clone()).await.unwrap();
    assert_eq!(registry.list_models().await.len(), 1);
    provider.mode.store(1, Ordering::SeqCst);
    registry.health_check_all(CancellationToken::new()).await;
    provider.proceed.notify_one();
    assert!(old_poll.await.unwrap().is_empty());
    assert_eq!(
        registry.provider_health(provider.id).await.unwrap().state,
        ProviderHealthState::Unreachable
    );
    registry.remove_model(registration.model_id).await.unwrap();
    provider.mode.store(3, Ordering::SeqCst);
    let r = registry.clone();
    let old_instance =
        tokio::spawn(async move { r.health_check_all(CancellationToken::new()).await });
    provider.entered.notified().await;
    registry.remove_provider(provider.id).await.unwrap();
    registry
        .register_provider(Arc::new(MockProvider::new(provider.id)))
        .await
        .unwrap();
    provider.proceed.notify_one();
    assert!(old_instance.await.unwrap().is_empty());
    assert_eq!(
        registry.provider_health(provider.id).await.unwrap().state,
        ProviderHealthState::Unknown
    );
}

#[tokio::test(start_paused = true)]
async fn poll_timeout_is_bounded_and_cancellation_does_not_fabricate_health() {
    use std::time::Duration;
    use vespera_model_registry::ProviderHealthState;
    let registry = Arc::new(
        ModelRegistry::with_health_policy(Duration::from_secs(30), Duration::from_secs(1)).unwrap(),
    );
    let provider = Arc::new(MockProvider::new(ProviderId::new()));
    provider.mode.store(3, Ordering::SeqCst);
    registry.register_provider(provider.clone()).await.unwrap();
    let r = registry.clone();
    let poll = tokio::spawn(async move { r.health_check_all(CancellationToken::new()).await });
    provider.entered.notified().await;
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(
        poll.await.unwrap()[&provider.id],
        ProviderHealthStatus::Unreachable
    );
    let cancel = CancellationToken::new();
    let r = registry.clone();
    let token = cancel.clone();
    let poll = tokio::spawn(async move { r.health_check_all(token).await });
    provider.entered.notified().await;
    cancel.cancel();
    assert!(poll.await.unwrap().is_empty());
    assert_eq!(
        registry.provider_health(provider.id).await.unwrap().state,
        ProviderHealthState::Unreachable
    );
}

#[tokio::test]
async fn invalid_registration_does_not_mutate_inventory_or_expose_payload() {
    let registry = ModelRegistry::new();
    let provider = Arc::new(MockProvider::new(ProviderId::new()));
    registry.register_provider(provider.clone()).await.unwrap();
    let mut registration = model(provider.id, ModelId::new());
    registration.context_window = Some(0);
    assert!(matches!(
        registry.register_model(registration.clone()).await,
        Err(RegistryError::InvalidField {
            field: "context_window"
        })
    ));
    registration.context_window = None;
    registration.name = "  ".into();
    assert!(registry.register_model(registration).await.is_err());
    assert!(registry.list_models().await.is_empty());
}
