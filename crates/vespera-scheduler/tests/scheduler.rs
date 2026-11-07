//! Deterministic scheduler integration tests; no live provider is required.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    load_mode: AtomicUsize,
    unload_mode: AtomicUsize,
    loads: AtomicUsize,
    unloads: AtomicUsize,
    entered: tokio::sync::Notify,
    proceed: tokio::sync::Notify,
}

impl MockProvider {
    fn new(id: ProviderId) -> Self {
        Self {
            id,
            load_mode: AtomicUsize::new(1),
            unload_mode: AtomicUsize::new(1),
            loads: AtomicUsize::new(0),
            unloads: AtomicUsize::new(0),
            entered: tokio::sync::Notify::new(),
            proceed: tokio::sync::Notify::new(),
        }
    }
    async fn lifecycle(
        &self,
        name: &str,
        mode: usize,
        cancel: CancellationToken,
    ) -> Result<ModelLifecycleOutcome, vespera_providers::ProviderError> {
        if mode == 4 {
            self.entered.notify_one();
            tokio::select! {
                () = self.proceed.notified() => (),
                () = cancel.cancelled() => return Err(vespera_providers::ProviderError::Cancelled { provider_id: self.id, phase: "fixture" }),
            }
        }
        if mode == 3 {
            return Err(vespera_providers::ProviderError::ConnectionFailed {
                provider_id: self.id,
                sanitized_message: "fixture failure".into(),
            });
        }
        Ok(ModelLifecycleOutcome {
            provider_id: self.id,
            provider_model_name: name.into(),
            acknowledged: mode != 2,
            elapsed_ms: 0,
        })
    }
}

#[tokio::test]
async fn unknown_estimates_are_rejected_without_changing_accounting() {
    use std::time::Duration;
    use vespera_core::ResourceBudget;
    use vespera_scheduler::{ResourceScheduler, SchedulerError, SchedulingPolicy};
    let registry = Arc::new(ModelRegistry::new());
    let provider_id = ProviderId::new();
    registry
        .register_provider(Arc::new(MockProvider::new(provider_id)))
        .await;
    let scheduler = ResourceScheduler::new(
        registry.clone(),
        SchedulingPolicy {
            queue: vespera_scheduler::QueuePolicy::default(),
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
            explicit_model_load: if self.load_mode.load(Ordering::SeqCst) == 0 {
                LifecycleCapabilityState::Unsupported
            } else {
                LifecycleCapabilityState::Supported
            },
            explicit_model_unload: if self.unload_mode.load(Ordering::SeqCst) == 0 {
                LifecycleCapabilityState::Unsupported
            } else {
                LifecycleCapabilityState::Supported
            },
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
        model_name: &'a str,
        _keep_alive: Option<std::time::Duration>,
        cancel: CancellationToken,
    ) -> vespera_providers::BoxFuture<
        'a,
        Result<ModelLifecycleOutcome, vespera_providers::ProviderError>,
    > {
        Box::pin(async move {
            self.loads.fetch_add(1, Ordering::SeqCst);
            self.lifecycle(model_name, self.load_mode.load(Ordering::SeqCst), cancel)
                .await
        })
    }
    fn unload_model<'a>(
        &'a self,
        model_name: &'a str,
        cancel: CancellationToken,
    ) -> vespera_providers::BoxFuture<
        'a,
        Result<ModelLifecycleOutcome, vespera_providers::ProviderError>,
    > {
        Box::pin(async move {
            self.unloads.fetch_add(1, Ordering::SeqCst);
            self.lifecycle(model_name, self.unload_mode.load(Ordering::SeqCst), cancel)
                .await
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

async fn fixture() -> (
    Arc<vespera_scheduler::ResourceScheduler>,
    Arc<MockProvider>,
    ModelId,
    ModelId,
) {
    use std::time::Duration;
    use vespera_core::ResourceBudget;
    use vespera_scheduler::{ResourceScheduler, SchedulingPolicy};
    let registry = Arc::new(ModelRegistry::new());
    let provider = Arc::new(MockProvider::new(ProviderId::new()));
    registry.register_provider(provider.clone()).await;
    let a = ModelId::new();
    let b = ModelId::new();
    for model_id in [a, b] {
        registry
            .register_model(ModelRegistration {
                model_id,
                provider_id: provider.id,
                name: model_id.to_string(),
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
            })
            .await
            .unwrap();
    }
    (
        Arc::new(ResourceScheduler::new(
            registry,
            SchedulingPolicy {
                queue: vespera_scheduler::QueuePolicy::default(),
                budget: ResourceBudget {
                    ram_bytes: 100,
                    vram_bytes: 0,
                    max_concurrent_inferences: 2,
                    max_context_tokens: 512,
                },
                default_keep_alive: Duration::from_secs(30),
                evict_idle_on_pressure: true,
            },
        )),
        provider,
        a,
        b,
    )
}

#[tokio::test]
async fn failed_or_unsupported_unload_never_frees_memory() {
    use vespera_scheduler::ResidencyState;
    for mode in [0, 2, 3] {
        let (scheduler, provider, a, b) = fixture().await;
        scheduler
            .admit_and_prepare(a, None, CancellationToken::new())
            .await
            .unwrap();

        provider.unload_mode.store(mode, Ordering::SeqCst);
        assert!(
            scheduler
                .admit_and_prepare(b, None, CancellationToken::new())
                .await
                .is_err()
        );
        assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 100);
        let entries = scheduler.resident_models().await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].model_id, a);
        assert_eq!(entries[0].active_inferences, 0);
        assert_eq!(
            entries[0].residency,
            if mode == 0 {
                ResidencyState::Loaded
            } else {
                ResidencyState::Unknown
            }
        );
        provider.unload_mode.store(1, Ordering::SeqCst);
        assert!(
            scheduler
                .evict_model(a, CancellationToken::new())
                .await
                .unwrap()
        );
        assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 0);
    }
}

#[tokio::test]
async fn successful_pressure_eviction_and_reuse_account_once() {
    let (scheduler, provider, a, b) = fixture().await;
    let first = scheduler
        .admit_and_prepare(a, None, CancellationToken::new())
        .await
        .unwrap();
    assert!(first.loaded_on_demand);
    let second = scheduler
        .admit_and_prepare(a, None, CancellationToken::new())
        .await
        .unwrap();
    assert!(!second.loaded_on_demand);
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 2);
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 100);
    assert!(
        !scheduler
            .evict_model(a, CancellationToken::new())
            .await
            .unwrap()
    );
    first.release();
    second.release();
    scheduler
        .admit_and_prepare(b, None, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(provider.loads.load(Ordering::SeqCst), 2);
    assert_eq!(provider.unloads.load(Ordering::SeqCst), 1);
    assert_eq!(scheduler.resident_models().await[0].model_id, b);
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 100);
}

#[tokio::test]
async fn concurrent_requests_wait_for_one_load_without_blocking_accounting() {
    let (scheduler, provider, a, _) = fixture().await;
    provider.load_mode.store(4, Ordering::SeqCst);
    let s = scheduler.clone();
    let first =
        tokio::spawn(async move { s.admit_and_prepare(a, None, CancellationToken::new()).await });
    provider.entered.notified().await;
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 1);
    let s = scheduler.clone();
    let second =
        tokio::spawn(async move { s.admit_and_prepare(a, None, CancellationToken::new()).await });
    tokio::task::yield_now().await;
    assert!(!second.is_finished());
    assert_eq!(provider.loads.load(Ordering::SeqCst), 1);
    provider.proceed.notify_one();
    let _first_lease = first.await.unwrap().unwrap();
    let _second_lease = second.await.unwrap().unwrap();
    assert_eq!(provider.loads.load(Ordering::SeqCst), 1);
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 2);
}

#[tokio::test]
async fn interrupted_load_retains_unknown_memory_and_releases_slot() {
    use vespera_scheduler::{ResidencyState, SchedulerError};
    let (scheduler, provider, a, _) = fixture().await;
    provider.load_mode.store(4, Ordering::SeqCst);
    let s = scheduler.clone();
    let loading =
        tokio::spawn(async move { s.admit_and_prepare(a, None, CancellationToken::new()).await });
    provider.entered.notified().await;
    loading.abort();
    assert!(loading.await.unwrap_err().is_cancelled());
    let usage = scheduler.usage_snapshot().await;
    assert_eq!((usage.ram_bytes, usage.active_inferences), (100, 0));
    assert_eq!(
        scheduler.resident_models().await[0].residency,
        ResidencyState::Unknown
    );
    assert!(matches!(
        scheduler
            .admit_and_prepare(a, None, CancellationToken::new())
            .await,
        Err(SchedulerError::UncertainResidency { .. })
    ));
    assert!(
        scheduler
            .evict_model(a, CancellationToken::new())
            .await
            .unwrap()
    );
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 0);
}

#[tokio::test]
async fn interrupted_unload_retains_memory_and_cannot_admit_replacement() {
    use vespera_scheduler::ResidencyState;
    let (scheduler, provider, a, _) = fixture().await;
    scheduler
        .admit_and_prepare(a, None, CancellationToken::new())
        .await
        .unwrap();

    provider.unload_mode.store(4, Ordering::SeqCst);
    let s = scheduler.clone();
    let unloading = tokio::spawn(async move { s.evict_model(a, CancellationToken::new()).await });
    provider.entered.notified().await;
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 100);
    unloading.abort();
    assert!(unloading.await.unwrap_err().is_cancelled());
    assert_eq!(
        scheduler.resident_models().await[0].residency,
        ResidencyState::Unknown
    );
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 100);
}

#[tokio::test]
async fn failed_load_retains_uncertain_memory_but_not_an_inference_slot() {
    for mode in [2, 3] {
        let (scheduler, provider, a, _) = fixture().await;
        provider.load_mode.store(mode, Ordering::SeqCst);
        assert!(
            scheduler
                .admit_and_prepare(a, None, CancellationToken::new())
                .await
                .is_err()
        );
        let usage = scheduler.usage_snapshot().await;
        assert_eq!((usage.ram_bytes, usage.active_inferences), (100, 0));
    }
}

#[tokio::test]
async fn lease_clones_release_exactly_once_and_last_drop_returns_slot() {
    let (scheduler, _, a, _) = fixture().await;
    let first = scheduler
        .admit_and_prepare(a, None, CancellationToken::new())
        .await
        .unwrap();
    let clone = first.clone();
    let second = scheduler
        .admit_and_prepare(a, None, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(first.lease_id(), clone.lease_id());
    assert_ne!(first.lease_id(), second.lease_id());
    drop(first);
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 2);
    assert!(clone.release());
    assert!(!clone.release());
    drop(clone);
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 1);
    drop(second);
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 0);
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 100);
}

#[tokio::test]
async fn scheduler_shutdown_closes_queued_work_and_preserves_charged_models() {
    use std::time::Duration;
    use vespera_scheduler::{QueueError, SchedulerError};
    let (scheduler, _, a, _) = fixture().await;
    let first = scheduler
        .admit_and_prepare(a, None, CancellationToken::new())
        .await
        .unwrap();
    let second = scheduler
        .admit_and_prepare(a, None, CancellationToken::new())
        .await
        .unwrap();
    let s = scheduler.clone();
    let queued =
        tokio::spawn(async move { s.admit_and_prepare(a, None, CancellationToken::new()).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while scheduler.queue_snapshot().waiting != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!scheduler.shutdown(Duration::ZERO).await);
    assert!(matches!(
        queued.await.unwrap(),
        Err(SchedulerError::Queue(QueueError::Closed))
    ));
    assert_eq!(scheduler.queue_snapshot().active, 2);
    first.release();
    second.release();
    assert!(scheduler.shutdown(Duration::from_secs(1)).await);
    assert_eq!(scheduler.queue_snapshot().active, 0);
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 0);
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 100);
    assert!(
        scheduler
            .evict_model(a, CancellationToken::new())
            .await
            .unwrap()
    );
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 0);
}
