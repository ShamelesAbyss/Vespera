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

use std::time::Duration;
use vespera_agents::{AgentRuntime, AgentRuntimeError};
use vespera_core::{
    AgentDefinition, AgentId, AgentIdentity, AgentStatus, ModelSelectionPolicy, Permission,
    ResourceBudget, Task, TaskId, TaskState,
};
use vespera_scheduler::{ResourceScheduler, SchedulingPolicy};

fn task(id: TaskId) -> Task {
    Task {
        id,
        prompt: "bounded fixture task".into(),
        required_capabilities: BTreeSet::new(),
        state: TaskState::Queued,
    }
}

async fn runtime_fixture() -> (
    Arc<AgentRuntime>,
    Arc<ResourceScheduler>,
    Arc<MockProvider>,
    AgentId,
    AgentId,
    ModelId,
) {
    let registry = Arc::new(ModelRegistry::new());
    let provider = Arc::new(MockProvider::new(ProviderId::new()));
    registry.register_provider(provider.clone()).await.unwrap();
    registry.health_check_all(CancellationToken::new()).await;
    let model_id = ModelId::new();
    registry
        .register_model(ModelRegistration {
            model_id,
            provider_id: provider.id,
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
        })
        .await
        .unwrap();
    let scheduler = Arc::new(ResourceScheduler::new(
        registry.clone(),
        SchedulingPolicy {
            queue: vespera_scheduler::QueuePolicy::default(),
            budget: ResourceBudget {
                ram_bytes: 100,
                vram_bytes: 0,
                max_concurrent_inferences: 4,
                max_context_tokens: 512,
            },
            default_keep_alive: Duration::from_secs(30),
            evict_idle_on_pressure: true,
        },
    ));
    let runtime = Arc::new(AgentRuntime::new(registry, scheduler.clone()));
    let a = AgentId::new();
    let b = AgentId::new();
    for id in [a, b] {
        runtime
            .register_agent(AgentDefinition {
                identity: AgentIdentity {
                    id,
                    name: id.to_string(),
                    role: "tester".into(),
                    description: String::new(),
                    personality: String::new(),
                    goals: vec![],
                },
                model_policy: ModelSelectionPolicy::Fixed { model_id },
                permissions: BTreeSet::from([Permission::Delegate]),
                status: AgentStatus::Idle,
            })
            .await
            .unwrap();
    }
    (runtime, scheduler, provider, a, b, model_id)
}

#[tokio::test]
async fn task_completion_uses_recorded_lease_and_rejects_wrong_or_duplicate_release() {
    let (runtime, scheduler, _, a, b, model) = runtime_fixture().await;
    let task_a = TaskId::new();
    let task_b = TaskId::new();
    let execution_a = runtime
        .start_task_execution(a, task(task_a), None, CancellationToken::new())
        .await
        .unwrap();
    let execution_b = runtime
        .start_task_execution(b, task(task_b), None, CancellationToken::new())
        .await
        .unwrap();
    assert_ne!(
        execution_a.inference.lease_id(),
        execution_b.inference.lease_id()
    );
    assert!(matches!(
        runtime
            .finish_task_execution(task_a, ModelId::new(), TaskState::Completed)
            .await,
        Err(AgentRuntimeError::ExecutionMismatch { .. })
    ));
    assert!(
        runtime
            .finish_task_execution(TaskId::new(), model, TaskState::Completed)
            .await
            .is_err()
    );
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 2);
    runtime
        .finish_task_execution(task_a, model, TaskState::Completed)
        .await
        .unwrap();
    assert!(
        runtime
            .finish_task_execution(task_a, model, TaskState::Completed)
            .await
            .is_err()
    );
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 1);
    drop(execution_a);
    drop(execution_b);
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 1);
    runtime
        .finish_task_execution(task_b, model, TaskState::Completed)
        .await
        .unwrap();
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 0);
    assert_eq!(
        runtime.get_agent(a).await.unwrap().status,
        AgentStatus::Idle
    );
    assert_eq!(
        runtime.get_task(task_b).await.unwrap().state,
        TaskState::Completed
    );
}

#[tokio::test]
async fn preparation_is_exclusive_and_abort_restores_agent_without_losing_history() {
    let (runtime, scheduler, provider, a, b, _) = runtime_fixture().await;
    provider.load_mode.store(4, Ordering::SeqCst);
    let id = TaskId::new();
    let r = runtime.clone();
    let starting = tokio::spawn(async move {
        r.start_task_execution(a, task(id), None, CancellationToken::new())
            .await
    });
    provider.entered.notified().await;
    assert_eq!(
        runtime.get_agent(a).await.unwrap().status,
        AgentStatus::Running
    );
    assert!(matches!(
        runtime
            .start_task_execution(a, task(TaskId::new()), None, CancellationToken::new())
            .await,
        Err(AgentRuntimeError::InvalidAgentState { .. })
    ));
    assert!(matches!(
        runtime
            .start_task_execution(b, task(id), None, CancellationToken::new())
            .await,
        Err(AgentRuntimeError::TaskConflict { .. })
    ));
    assert!(
        runtime
            .set_agent_status(a, AgentStatus::Idle)
            .await
            .is_err()
    );
    starting.abort();
    assert!(starting.await.unwrap_err().is_cancelled());
    assert_eq!(
        runtime.get_agent(a).await.unwrap().status,
        AgentStatus::Idle
    );
    assert_eq!(
        runtime.get_task(id).await.unwrap().state,
        TaskState::Cancelled
    );
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 0);
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 100);
}

#[tokio::test]
async fn delegated_tasks_execute_once_for_the_assigned_recipient() {
    let (runtime, scheduler, _, a, b, model) = runtime_fixture().await;
    let id = TaskId::new();
    runtime.delegate_task(a, b, task(id)).await.unwrap();
    assert!(matches!(
        runtime.delegate_task(a, b, task(id)).await,
        Err(AgentRuntimeError::TaskConflict { .. })
    ));
    assert!(matches!(
        runtime
            .start_task_execution(a, task(id), None, CancellationToken::new())
            .await,
        Err(AgentRuntimeError::TaskConflict { .. })
    ));
    let execution = runtime
        .start_task_execution(b, task(id), None, CancellationToken::new())
        .await
        .unwrap();
    runtime
        .finish_task_execution(id, model, TaskState::Completed)
        .await
        .unwrap();
    drop(execution);
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 0);
    assert!(
        runtime
            .start_task_execution(b, task(id), None, CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(
        runtime.get_task(id).await.unwrap().state,
        TaskState::Completed
    );
}

#[tokio::test]
async fn preparation_failure_records_failed_status_and_keeps_uncertain_memory() {
    let (runtime, scheduler, provider, a, _, _) = runtime_fixture().await;
    provider.load_mode.store(3, Ordering::SeqCst);
    let id = TaskId::new();
    assert!(
        runtime
            .start_task_execution(a, task(id), None, CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(
        runtime.get_agent(a).await.unwrap().status,
        AgentStatus::Failed
    );
    assert_eq!(runtime.get_task(id).await.unwrap().state, TaskState::Failed);
    assert_eq!(scheduler.usage_snapshot().await.active_inferences, 0);
    assert_eq!(scheduler.usage_snapshot().await.ram_bytes, 100);
}

#[tokio::test]
async fn inbox_read_requires_communication_permission() {
    let (runtime, _, _, a, _, _) = runtime_fixture().await;
    assert!(matches!(
        runtime.inbox_messages(a).await,
        Err(AgentRuntimeError::PermissionDenied {
            permission: Permission::Communicate,
            ..
        })
    ));
}
