//! Deterministic scheduler integration tests; no live provider is required.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use vespera_core::{InferenceOverhead, ResourceEstimate};
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
    last_load_context: AtomicUsize,
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
            last_load_context: AtomicUsize::new(0),
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
        .await
        .unwrap();
    registry.health_check_all(CancellationToken::new()).await;
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
            residency_telemetry: LifecycleCapabilityState::Supported,
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
            Ok(ProviderTelemetrySnapshot {
                provider_id: self.id,
                resident_models: vec![],
                total_provider_ram_bytes: 0,
                total_provider_vram_bytes: 0,
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
    fn load_model_with_context<'a>(
        &'a self,
        model_name: &'a str,
        context_tokens: u32,
        keep_alive: Option<std::time::Duration>,
        cancel: CancellationToken,
    ) -> vespera_providers::BoxFuture<
        'a,
        Result<ModelLifecycleOutcome, vespera_providers::ProviderError>,
    > {
        self.last_load_context
            .store(context_tokens as usize, Ordering::SeqCst);
        self.load_model(model_name, keep_alive, cancel)
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
    fixture_with_limits(100, 0, InferenceOverhead::default()).await
}

async fn fixture_with_limits(
    ram_bytes: u64,
    vram_bytes: u64,
    overhead: InferenceOverhead,
) -> (
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
    registry.register_provider(provider.clone()).await.unwrap();
    registry.health_check_all(CancellationToken::new()).await;
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
        Arc::new(ResourceScheduler::new_with_overhead(
            registry,
            SchedulingPolicy {
                queue: vespera_scheduler::QueuePolicy::default(),
                budget: ResourceBudget {
                    ram_bytes,
                    vram_bytes,
                    max_concurrent_inferences: 2,
                    max_context_tokens: 512,
                },
                default_keep_alive: Duration::from_secs(30),
                evict_idle_on_pressure: true,
            },
            overhead,
        )),
        provider,
        a,
        b,
    )
}

#[tokio::test]
async fn validated_request_binding_and_context_are_checked_before_load() {
    use vespera_providers::{BoundedInferenceRequest, ChatMessage, ChatRole};
    use vespera_scheduler::SchedulerError;
    let (scheduler, provider, model, other) = fixture().await;
    for (name, context) in [(other.to_string(), 512), (model.to_string(), 1024)] {
        let request = BoundedInferenceRequest::new(
            name,
            vec![ChatMessage {
                role: ChatRole::User,
                content: "fixture".into(),
            }],
            8,
            context,
            None,
            None,
        )
        .unwrap();
        let result = scheduler
            .admit_request(model, &request, CancellationToken::new())
            .await;
        assert!(matches!(
            result,
            Err(SchedulerError::RegistrationChanged { .. })
                | Err(SchedulerError::BudgetExceeded(_))
        ));
        assert_eq!(provider.loads.load(Ordering::SeqCst), 0);
        assert!(scheduler.resident_models().await.is_empty());
        assert_eq!(scheduler.queue_snapshot().active, 0);
    }
}

#[tokio::test]
async fn failed_or_unsupported_unload_never_frees_memory() {
    use vespera_core::EventKind;
    use vespera_scheduler::ResidencyState;
    for mode in [0, 2, 3] {
        let (scheduler, provider, a, b) = fixture().await;
        let mut events = scheduler.observe_lifecycle();
        scheduler
            .admit_and_prepare(a, None, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            events.receiver.try_recv().unwrap().kind,
            EventKind::ModelActivated { model_id: a }
        );

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
        assert!(events.receiver.try_recv().is_err());
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
        assert_eq!(
            events.receiver.try_recv().unwrap().kind,
            EventKind::ModelDeactivated { model_id: a }
        );
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
async fn context_change_fences_active_leases_and_reloads_only_after_acknowledged_unload() {
    use vespera_scheduler::{ResidencyState, SchedulerError};

    let (scheduler, provider, model, _) = fixture().await;
    let admit =
        |context| scheduler.admit_and_prepare(model, Some(context), CancellationToken::new());
    let lease = admit(256).await.unwrap();
    assert_eq!(provider.last_load_context.load(Ordering::SeqCst), 256);
    assert!(matches!(
        admit(512).await,
        Err(SchedulerError::ContextInUse {
            loaded: 256,
            requested: 512,
            ..
        })
    ));
    assert_eq!(provider.unloads.load(Ordering::SeqCst), 0);
    lease.release();
    let replacement = admit(512).await.unwrap();
    assert!(replacement.loaded_on_demand);
    assert_eq!(provider.unloads.load(Ordering::SeqCst), 1);
    assert_eq!(provider.loads.load(Ordering::SeqCst), 2);
    assert_eq!(provider.last_load_context.load(Ordering::SeqCst), 512);
    replacement.release();

    provider.unload_mode.store(2, Ordering::SeqCst);
    assert!(matches!(
        admit(256).await,
        Err(SchedulerError::UnacknowledgedLifecycle { .. })
    ));
    assert_eq!(provider.loads.load(Ordering::SeqCst), 2);
    let usage = scheduler.usage_snapshot().await;
    assert_eq!(usage.active_inferences, 0);
    assert_eq!(usage.ram_bytes, 100);
    assert_eq!(
        scheduler.resident_models().await[0].residency,
        ResidencyState::Unknown
    );
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
async fn each_lease_charges_overhead_and_reuse_obeys_both_memory_budgets() {
    use vespera_core::{DomainError, ResourceKind};
    use vespera_scheduler::SchedulerError;

    let overhead = InferenceOverhead {
        ram_bytes: 10,
        vram_bytes_per_context_token: 1,
        ..InferenceOverhead::default()
    };
    for (ram, vram, resource) in [
        (115, 512, ResourceKind::Ram),
        (120, 511, ResourceKind::Vram),
    ] {
        let (scheduler, provider, model, _) = fixture_with_limits(ram, vram, overhead).await;
        let mut events = scheduler.observe_lifecycle();
        let admit = || scheduler.admit_and_prepare(model, Some(256), CancellationToken::new());
        let first = admit().await.unwrap();
        assert!(matches!(
            events.receiver.try_recv().unwrap().kind,
            vespera_core::EventKind::ModelActivated { .. }
        ));
        assert!(matches!(
            admit().await,
            Err(SchedulerError::BudgetExceeded(DomainError::ResourceLimitExceeded {
                resource: actual,
                ..
            })) if actual == resource
        ));
        assert!(matches!(events.receiver.try_recv().unwrap().kind,
            vespera_core::EventKind::ResourceLimitReached { resource: actual, requested, limit, .. }
            if actual == resource && requested > limit));
        let usage = scheduler.usage_snapshot().await;
        assert_eq!(
            (usage.ram_bytes, usage.vram_bytes, usage.active_inferences),
            (110, 256, 1)
        );

        let clone = first.clone();
        assert!(first.release());
        assert!(!clone.release());
        let second = admit().await.unwrap();
        assert!(!second.loaded_on_demand);
        drop(second);
        let usage = scheduler.usage_snapshot().await;
        assert_eq!(
            (usage.ram_bytes, usage.vram_bytes, usage.active_inferences),
            (100, 0, 0)
        );
        assert_eq!(provider.loads.load(Ordering::SeqCst), 1);
        assert_eq!(provider.unloads.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn reconciliation_retains_uncertain_reservations_without_unloading_them() {
    use vespera_scheduler::SchedulerError;

    let (scheduler, provider, model, other) = fixture().await;
    scheduler
        .reconcile_startup(CancellationToken::new())
        .await
        .unwrap();
    provider.load_mode.store(2, Ordering::SeqCst);
    assert!(matches!(
        scheduler
            .admit_and_prepare(model, None, CancellationToken::new())
            .await,
        Err(SchedulerError::UnacknowledgedLifecycle { .. })
    ));
    assert!(matches!(
        scheduler.evict_model(model, CancellationToken::new()).await,
        Err(SchedulerError::UncertainResidency { .. })
    ));
    assert!(matches!(
        scheduler
            .admit_and_prepare(other, None, CancellationToken::new())
            .await,
        Err(SchedulerError::BudgetExceeded(_))
    ));
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 100);
    assert_eq!(provider.unloads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn interrupted_load_retains_unknown_memory_and_releases_slot() {
    use vespera_scheduler::{ResidencyState, SchedulerError};
    let overhead = InferenceOverhead {
        ram_bytes: 20,
        ..InferenceOverhead::default()
    };
    let (scheduler, provider, a, _) = fixture_with_limits(120, 0, overhead).await;
    provider.load_mode.store(4, Ordering::SeqCst);
    let s = scheduler.clone();
    let loading =
        tokio::spawn(async move { s.admit_and_prepare(a, None, CancellationToken::new()).await });
    provider.entered.notified().await;
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 120);
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
        let mut events = scheduler.observe_lifecycle();
        provider.load_mode.store(mode, Ordering::SeqCst);
        assert!(
            scheduler
                .admit_and_prepare(a, None, CancellationToken::new())
                .await
                .is_err()
        );
        let usage = scheduler.usage_snapshot().await;
        assert_eq!((usage.ram_bytes, usage.active_inferences), (100, 0));
        assert!(events.receiver.try_recv().is_err());
    }
}

#[tokio::test]
async fn lifecycle_observer_bounds_backlog_and_reports_omissions() {
    use vespera_core::{AgentId, TaskId};
    let (scheduler, _, model, _) = fixture().await;
    let mut events = scheduler.observe_lifecycle();
    let task = TaskId::new();
    let agent = AgentId::new();
    for _ in 0..257 {
        scheduler.observe_inference_started(task, model, agent);
    }
    scheduler.stop_lifecycle_observation();
    let mut drained = 0;
    while let Some(event) = events.receiver.recv().await {
        assert_eq!(event.correlation_task_id, Some(task));
        drained += 1;
    }
    assert_eq!(drained, 256);
    assert_eq!(events.omitted.load(Ordering::Relaxed), 1);
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

#[tokio::test]
async fn registry_removal_and_rebinding_do_not_redirect_reserved_model_unload() {
    use std::time::Duration;
    use vespera_core::ResourceBudget;
    use vespera_scheduler::{ResourceScheduler, SchedulerError, SchedulingPolicy};
    let registry = Arc::new(ModelRegistry::new());
    let original = Arc::new(MockProvider::new(ProviderId::new()));
    registry.register_provider(original.clone()).await.unwrap();
    registry.health_check_all(CancellationToken::new()).await;
    let model_id = ModelId::new();
    let mut registration = ModelRegistration {
        model_id,
        provider_id: original.id,
        name: "fixture".into(),
        provider_model_identifier: "original".into(),
        parameter_size: None,
        quantization: None,
        context_window: Some(512),
        capabilities: BTreeSet::new(),
        estimated_resources: ResourceEstimate {
            ram_bytes: Some(100),
            vram_bytes: Some(0),
        },
        available: true,
    };
    registry.register_model(registration.clone()).await.unwrap();
    let scheduler = ResourceScheduler::new(
        registry.clone(),
        SchedulingPolicy {
            queue: Default::default(),
            budget: ResourceBudget::default(),
            default_keep_alive: Duration::from_secs(30),
            evict_idle_on_pressure: true,
        },
    );
    assert!(matches!(
        scheduler
            .admit_and_prepare(model_id, Some(1024), CancellationToken::new())
            .await,
        Err(SchedulerError::BudgetExceeded(_))
    ));
    let admission = scheduler
        .admit_and_prepare(model_id, None, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(admission.context_tokens, 512);
    drop(admission);
    registration.estimated_resources.ram_bytes = Some(200);
    registry.update_model(registration.clone()).await.unwrap();
    assert!(matches!(
        scheduler
            .admit_and_prepare(model_id, None, CancellationToken::new())
            .await,
        Err(SchedulerError::RegistrationChanged { .. })
    ));
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 100);
    registry.remove_model(model_id).await.unwrap();
    registry.remove_provider(original.id).await.unwrap();
    let replacement = Arc::new(MockProvider::new(original.id));
    registry
        .register_provider(replacement.clone())
        .await
        .unwrap();
    registry.health_check_all(CancellationToken::new()).await;
    registration.provider_model_identifier = "replacement".into();
    registry.register_model(registration).await.unwrap();
    assert!(matches!(
        scheduler
            .admit_and_prepare(model_id, None, CancellationToken::new())
            .await,
        Err(SchedulerError::RegistrationChanged { .. })
    ));
    assert!(
        scheduler
            .evict_model(model_id, CancellationToken::new())
            .await
            .unwrap()
    );
    assert_eq!(original.unloads.load(Ordering::SeqCst), 1);
    assert_eq!(replacement.unloads.load(Ordering::SeqCst), 0);
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 0);
    let _new = scheduler
        .admit_and_prepare(model_id, None, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(replacement.loads.load(Ordering::SeqCst), 1);
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 200);
}
