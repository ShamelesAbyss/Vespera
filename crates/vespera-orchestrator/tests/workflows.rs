//! Actual agent/scheduler/native-HTTP workflow integration; no installed model required.
use serde_json::{Value, json};
use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use vespera_agents::AgentRuntime;
use vespera_core::{
    AgentDefinition, AgentId, AgentIdentity, AgentStatus, Capability, ModelId,
    ModelSelectionPolicy, Permission, ProviderId, ResourceBudget, ResourceEstimate, Task, TaskId,
    TaskState,
};
use vespera_model_registry::{ModelRegistration, ModelRegistry};
use vespera_orchestrator::{
    ConflictPolicy, Failure, Orchestrator, OrchestratorError, OutputPolicy, Step, Workflow,
};
use vespera_providers::{CancellationToken, OllamaConfig, OllamaProvider};
use vespera_scheduler::{ResourceScheduler, SchedulingPolicy};

struct Fixture {
    orchestrator: Orchestrator,
    agents: AgentRuntime,
    scheduler: Arc<ResourceScheduler>,
    coordinator: AgentId,
    workers: [AgentId; 2],
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    entered: Arc<tokio::sync::Notify>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
fn caps(name: &str) -> BTreeSet<Capability> {
    [Capability::new(name).unwrap()].into()
}
impl Fixture {
    async fn new(replies: Vec<(u16, &str)>, delay: Duration, budget: u64) -> Self {
        Self::with_health_age(replies, delay, budget, Duration::from_secs(30)).await
    }
    async fn with_health_age(
        replies: Vec<(u16, &str)>,
        delay: Duration,
        budget: u64,
        health_age: Duration,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let entered = Arc::new(tokio::sync::Notify::new());
        let signal = entered.clone();
        let mut replies: VecDeque<_> = replies
            .into_iter()
            .map(|(status, body)| (status, body.to_owned()))
            .collect();
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let (header_len, content_len) = loop {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0 && bytes.len() < 128 * 1024);
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        break (end + 4, length);
                    }
                };
                while bytes.len() < header_len + content_len {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0 && bytes.len() < 128 * 1024);
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let headers = String::from_utf8_lossy(&bytes[..header_len]);
                let path = headers.split_whitespace().nth(1).unwrap().to_owned();
                let payload = if content_len == 0 {
                    Value::Null
                } else {
                    serde_json::from_slice(&bytes[header_len..header_len + content_len]).unwrap()
                };
                captured.lock().unwrap().push((path.clone(), payload));
                let (status, body) = match path.as_str() {
                    "/api/version" => (200, json!({"version":"fixture"}).to_string()),
                    "/api/generate" => (200, json!({"done":true}).to_string()),
                    "/api/chat" => {
                        signal.notify_one();
                        tokio::time::sleep(delay).await;
                        let (status, content) =
                            replies.pop_front().unwrap_or((200, "answer".into()));
                        (
                            status,
                            if status == 200 {
                                json!({"message":{"role":"assistant","content":content},"done":true,"done_reason":"stop","eval_count":2}).to_string()
                            } else {
                                "private fixture failure".into()
                            },
                        )
                    }
                    _ => (404, "{}".into()),
                };
                let response = format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        let registry = Arc::new(
            ModelRegistry::with_health_policy(health_age, Duration::from_secs(1)).unwrap(),
        );
        let provider_id = ProviderId::new();
        registry
            .register_provider(Arc::new(
                OllamaProvider::new(
                    OllamaConfig::loopback_default(provider_id, &endpoint).unwrap(),
                )
                .unwrap(),
            ))
            .await
            .unwrap();
        registry.health_check_all(CancellationToken::new()).await;
        let models = [ModelId::new(), ModelId::new()];
        for (i, name) in ["research", "coding"].into_iter().enumerate() {
            registry
                .register_model(ModelRegistration {
                    model_id: models[i],
                    provider_id,
                    name: name.into(),
                    provider_model_identifier: name.into(),
                    parameter_size: None,
                    quantization: None,
                    context_window: Some(512),
                    capabilities: caps(name),
                    estimated_resources: ResourceEstimate {
                        ram_bytes: Some(100),
                        vram_bytes: Some(0),
                    },
                    available: true,
                })
                .await
                .unwrap();
        }
        let scheduler = Arc::new(ResourceScheduler::new(
            registry.clone(),
            SchedulingPolicy {
                queue: Default::default(),
                budget: ResourceBudget {
                    ram_bytes: budget,
                    vram_bytes: 0,
                    max_concurrent_inferences: 1,
                    max_context_tokens: 512,
                },
                default_keep_alive: Duration::from_secs(30),
                evict_idle_on_pressure: true,
            },
        ));
        let agents = AgentRuntime::new(registry, scheduler.clone());
        let coordinator = AgentId::new();
        let workers = [AgentId::new(), AgentId::new()];
        for (id, model, permissions) in [
            (coordinator, models[0], [Permission::Delegate].into()),
            (workers[0], models[0], BTreeSet::new()),
            (workers[1], models[1], BTreeSet::new()),
        ] {
            agents
                .register_agent(AgentDefinition {
                    identity: AgentIdentity {
                        id,
                        name: "fixture".into(),
                        role: "fixture".into(),
                        description: String::new(),
                        personality: String::new(),
                        goals: Vec::new(),
                    },
                    model_policy: ModelSelectionPolicy::Fixed { model_id: model },
                    permissions,
                    status: AgentStatus::Idle,
                })
                .await
                .unwrap();
        }
        Self {
            orchestrator: Orchestrator::new(agents.clone()),
            agents,
            scheduler,
            coordinator,
            workers,
            requests,
            entered,
            server,
        }
    }
    fn plan(&self, multi: bool) -> Workflow {
        let step = |name: &str, capability: &str, worker| Step {
            name: name.into(),
            prompt: format!("Perform {name}"),
            required_capabilities: caps(capability),
            candidates: vec![worker],
            dependencies: Vec::new(),
            max_attempts: 1,
            max_output_tokens: 8,
            context_tokens: Some(512),
            timeout_ms: 2000,
            temperature: Some(0.0),
        };
        let mut steps = vec![step("investigate", "research", self.workers[0])];
        if multi {
            let mut next = step("implement", "coding", self.workers[1]);
            next.dependencies.push("investigate".into());
            steps.push(next);
        }
        Workflow {
            task: Task {
                id: TaskId::new(),
                prompt: "Operator request".into(),
                required_capabilities: caps("research"),
                state: TaskState::Queued,
            },
            coordinator: self.coordinator,
            output: OutputPolicy::Step {
                step: if multi { "implement" } else { "investigate" }.into(),
            },
            steps,
            max_total_attempts: 6,
            max_total_output_tokens: 48,
            timeout_ms: 5000,
        }
    }
    fn inference(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(path, _)| path == "/api/chat")
            .map(|(_, body)| body.clone())
            .collect()
    }
}

#[tokio::test]
async fn failed_result_checkpoint_stops_dependent_work() {
    use vespera_orchestrator::WorkflowStore;

    let fixture = Fixture::new(
        vec![(200, "observation"), (200, "unexpected")],
        Duration::ZERO,
        100,
    )
    .await;
    let plan = fixture.plan(true);
    let task_id = plan.task.id;
    let path = std::env::temp_dir().join(format!("vespera-workflow-{task_id}.sqlite3"));
    drop(WorkflowStore::open(&path).unwrap());
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TRIGGER reject_result_checkpoint BEFORE UPDATE ON workflows
             WHEN NEW.terminal=0 AND json_extract(CAST(NEW.report AS TEXT), '$.attempts[0].result') IS NOT NULL
             BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;",
        ).unwrap();
    }
    let store = WorkflowStore::open(&path).unwrap();
    let report = fixture
        .orchestrator
        .execute_checkpointed(plan, CancellationToken::new(), None, Some(&store))
        .await
        .unwrap();
    assert_eq!(report.task.state, TaskState::Failed);
    assert_eq!(report.failure, Some(Failure::Storage));
    assert_eq!(fixture.inference().len(), 1);
    drop(store);
    let saved = WorkflowStore::inspect(&path, task_id).unwrap();
    assert_eq!(saved.report.failure, Some(Failure::Storage));
    assert!(!saved.interrupted);
    std::fs::remove_file(&path).unwrap();
}

#[tokio::test]
async fn single_and_dependent_workflows_execute_only_selected_models_with_provenance() {
    for multi in [false, true] {
        let fixture = Fixture::new(
            vec![(200, "research observation"), (200, "code contribution")],
            Duration::ZERO,
            100,
        )
        .await;
        let plan = fixture.plan(multi);
        let report = fixture
            .orchestrator
            .execute(plan.clone(), CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(report.task.state, TaskState::Completed);
        assert_eq!(report.attempts.len(), if multi { 2 } else { 1 });
        for attempt in &report.attempts {
            let result = attempt.result.as_ref().unwrap();
            assert!(attempt.delegated);
            assert_eq!(attempt.task_state, Some(TaskState::Completed));
            assert_eq!(attempt.agent_id, result.agent_id);
            assert_eq!(attempt.task_id, result.task.id);
            assert_eq!(attempt.candidate_model_id, result.model_id);
            assert!(!result.lease_id.is_empty());
        }
        let requests = fixture.inference();
        assert_eq!(requests[0]["model"], "research");
        if multi {
            assert_eq!(requests[1]["model"], "coding");
            let prompt: Value =
                serde_json::from_str(requests[1]["messages"][0]["content"].as_str().unwrap())
                    .unwrap();
            assert_eq!(
                prompt["untrusted_dependency_outputs"][0]["content"],
                "research observation"
            );
            assert_eq!(report.selected_step.as_deref(), Some("implement"));
        }
        assert_eq!(
            fixture.scheduler.usage_snapshot().await.active_inferences,
            0
        );
        assert_eq!(
            fixture
                .orchestrator
                .report(plan.task.id)
                .unwrap()
                .task
                .state,
            TaskState::Completed
        );
        assert!(matches!(
            fixture
                .orchestrator
                .execute(plan.clone(), CancellationToken::new())
                .await,
            Err(OrchestratorError::Duplicate)
        ));
        assert!(fixture.orchestrator.forget(plan.task.id).unwrap());
    }
}

#[tokio::test]
async fn authorization_capability_and_resource_failures_do_not_activate_unrelated_models() {
    let fixture = Fixture::new(vec![], Duration::ZERO, 99).await;
    let plan = fixture.plan(false);
    let report = fixture
        .orchestrator
        .execute(plan, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report.failure, Some(Failure::AdmissionDenied));
    assert!(fixture.inference().is_empty());
    assert!(fixture.scheduler.resident_models().await.is_empty());
    // The failed worker stays failed. An idle but unsuitable candidate is not resumed/rebound.
    let mut plan = fixture.plan(false);
    plan.steps[0].candidates = vec![fixture.workers[1]];
    assert_eq!(
        fixture
            .orchestrator
            .execute(plan.clone(), CancellationToken::new())
            .await
            .unwrap()
            .failure,
        Some(Failure::NoCandidate)
    );
    plan.coordinator = fixture.workers[1];
    assert!(matches!(
        fixture
            .orchestrator
            .execute(plan, CancellationToken::new())
            .await,
        Err(OrchestratorError::CoordinatorDenied)
    ));
    assert!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|(path, _)| path == "/api/version")
    );
}

#[tokio::test]
async fn partial_failure_and_token_attempt_budgets_preserve_completed_contributions() {
    for failure in [
        Failure::ExecutionFailed,
        Failure::AttemptBudget,
        Failure::OutputBudget,
    ] {
        let fixture = Fixture::new(
            vec![(200, "first"), (500, "not disclosed")],
            Duration::ZERO,
            100,
        )
        .await;
        let mut plan = fixture.plan(true);
        if failure == Failure::AttemptBudget {
            plan.max_total_attempts = 1;
        }
        if failure == Failure::OutputBudget {
            plan.max_total_output_tokens = 8;
        }
        let report = fixture
            .orchestrator
            .execute(plan, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(report.task.state, TaskState::Failed);
        assert_eq!(report.failure, Some(failure));
        assert_eq!(
            report.attempts[0].result.as_ref().unwrap().response.content,
            "first"
        );
        assert!(report.selected_step.is_none());
        assert_eq!(
            report.attempts.len(),
            if failure == Failure::ExecutionFailed {
                2
            } else {
                1
            }
        );
    }
}

#[tokio::test]
async fn conflicts_are_preserved_and_only_explicit_operator_precedence_resolves_them() {
    for prefer in [false, true] {
        let fixture = Fixture::new(vec![(200, "A"), (200, "B")], Duration::ZERO, 100).await;
        let mut plan = fixture.plan(true);
        plan.output = OutputPolicy::Agreement {
            steps: vec!["investigate".into(), "implement".into()],
            on_conflict: if prefer {
                ConflictPolicy::Prefer {
                    step: "implement".into(),
                }
            } else {
                ConflictPolicy::Fail
            },
        };
        let report = fixture
            .orchestrator
            .execute(plan, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(report.conflicting_steps, ["investigate", "implement"]);
        assert_eq!(
            report.task.state,
            if prefer {
                TaskState::Completed
            } else {
                TaskState::Failed
            }
        );
        assert_eq!(
            report.failure,
            if prefer {
                None
            } else {
                Some(Failure::ConflictingOutputs)
            }
        );
        assert_eq!(
            report.selected_step.as_deref(),
            if prefer { Some("implement") } else { None }
        );
    }
}

#[tokio::test]
async fn transient_rejection_retries_are_bounded_and_never_resume_failed_agents() {
    let fixture = Fixture::new(
        vec![(429, "private"), (200, "recovered")],
        Duration::ZERO,
        100,
    )
    .await;
    // Coordinator is also a permitted idle research candidate, without new authority grants.
    let mut plan = fixture.plan(false);
    plan.steps[0].candidates.push(fixture.coordinator);
    plan.steps[0].max_attempts = 3;
    let report = fixture
        .orchestrator
        .execute(plan, CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report.task.state, TaskState::Completed);
    assert_eq!(report.attempts.len(), 2);
    assert_eq!(
        report.attempts[0].failure,
        Some(Failure::TransientRejection)
    );
    assert_eq!(report.attempts[1].number, 2);
    assert_eq!(report.reserved_output_tokens, 16);
    assert_eq!(
        fixture
            .agents
            .get_agent(fixture.workers[0])
            .await
            .unwrap()
            .status,
        AgentStatus::Failed
    );
}

#[tokio::test]
async fn cancellation_deadline_and_future_abort_record_terminal_history_and_release_leases() {
    for mode in [0, 1, 2, 3] {
        let fixture = Fixture::new(vec![], Duration::from_secs(30), 100).await;
        let mut plan = fixture.plan(false);
        if mode == 1 {
            plan.timeout_ms = 100;
        }
        if mode == 3 {
            plan.steps[0].timeout_ms = 100;
        }
        let id = plan.task.id;
        let orchestrator = fixture.orchestrator.clone();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let running = tokio::spawn(async move { orchestrator.execute(plan, token).await });
        fixture.entered.notified().await;
        assert!(matches!(
            fixture.orchestrator.forget(id),
            Err(OrchestratorError::Running)
        ));
        if mode == 0 {
            cancel.cancel();
        }
        if mode == 2 {
            running.abort();
        }
        if mode == 2 {
            assert!(running.await.unwrap_err().is_cancelled());
        } else {
            let report = running.await.unwrap().unwrap();
            assert_eq!(
                report.failure,
                Some(if mode == 0 {
                    Failure::Cancelled
                } else {
                    Failure::Deadline
                })
            );
        }
        let report = fixture.orchestrator.report(id).unwrap();
        assert_eq!(report.task.state, TaskState::Cancelled);
        assert_eq!(report.attempts[0].task_state, Some(TaskState::Cancelled));
        assert_eq!(
            fixture
                .agents
                .get_task(report.attempts[0].task_id)
                .await
                .unwrap()
                .state,
            TaskState::Cancelled
        );
        assert_eq!(
            fixture.scheduler.usage_snapshot().await.active_inferences,
            0
        );
        assert_eq!(fixture.scheduler.queue_snapshot().active, 0);
    }
}

#[tokio::test]
async fn invalid_plans_preflight_without_history_and_retained_history_is_bounded() {
    let fixture = Fixture::new(vec![], Duration::ZERO, 100).await;
    let mut plan = fixture.plan(true);
    plan.steps[0].dependencies.push("implement".into());
    let id = plan.task.id;
    assert!(matches!(
        fixture
            .orchestrator
            .execute(plan, CancellationToken::new())
            .await,
        Err(OrchestratorError::Plan(_))
    ));
    assert!(fixture.orchestrator.report(id).is_none());
    for _ in 0..8 {
        let mut plan = fixture.plan(false);
        plan.steps[0].candidates = vec![fixture.workers[1]];
        assert_eq!(
            fixture
                .orchestrator
                .execute(plan, CancellationToken::new())
                .await
                .unwrap()
                .failure,
            Some(Failure::NoCandidate)
        );
    }
    assert!(matches!(
        fixture
            .orchestrator
            .execute(fixture.plan(false), CancellationToken::new())
            .await,
        Err(OrchestratorError::Capacity)
    ));
    assert!(fixture.inference().is_empty());
}

#[tokio::test]
async fn exact_agreement_selects_a_contribution_and_empty_output_is_rejected() {
    let fixture = Fixture::new(vec![(200, "same"), (200, "same")], Duration::ZERO, 100).await;
    let mut plan = fixture.plan(true);
    plan.output = OutputPolicy::Agreement {
        steps: vec!["investigate".into(), "implement".into()],
        on_conflict: ConflictPolicy::Fail,
    };
    let report = fixture
        .orchestrator
        .execute(plan, CancellationToken::new())
        .await
        .unwrap();
    assert!(report.conflicting_steps.is_empty());
    assert_eq!(report.selected_result().unwrap().response.content, "same");
    let fixture = Fixture::new(vec![(200, " ")], Duration::ZERO, 100).await;
    let report = fixture
        .orchestrator
        .execute(fixture.plan(false), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report.failure, Some(Failure::RejectedOutput));
    assert!(report.selected_result().is_none());
    assert_eq!(report.attempts[0].task_state, Some(TaskState::Completed));
    assert_eq!(
        report.attempts[0].result.as_ref().unwrap().response.content,
        " "
    );
}

#[tokio::test]
async fn pre_cancelled_work_and_exhausted_retries_never_start_extra_inference() {
    let fixture = Fixture::new(vec![], Duration::ZERO, 100).await;
    let token = CancellationToken::new();
    token.cancel();
    let report = fixture
        .orchestrator
        .execute(fixture.plan(false), token)
        .await
        .unwrap();
    assert_eq!(report.failure, Some(Failure::Cancelled));
    assert!(report.attempts.is_empty());
    assert!(fixture.inference().is_empty());
    let fixture = Fixture::new(vec![(503, "private")], Duration::ZERO, 100).await;
    let report = fixture
        .orchestrator
        .execute(fixture.plan(false), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report.failure, Some(Failure::TransientRejection));
    assert_eq!(report.attempts.len(), 1);
    assert_eq!(fixture.inference().len(), 1);
}

#[tokio::test]
async fn plan_validation_bounds_text_dependencies_attempts_and_output_policy() {
    let fixture = Fixture::new(vec![], Duration::ZERO, 100).await;
    let original = fixture.plan(true);
    for index in 0..9 {
        let mut plan = original.clone();
        match index {
            0 => plan.steps[0].prompt = "x".repeat(64 * 1024),
            1 => plan.steps[1].name = plan.steps[0].name.clone(),
            2 => plan.steps[1].dependencies = vec!["investigate".into(), "investigate".into()],
            3 => plan.steps[0].max_attempts = 4,
            4 => plan.max_total_attempts = 49,
            5 => plan.max_total_output_tokens = u64::MAX,
            6 => {
                let duplicate = plan.steps[0].candidates[0];
                plan.steps[0].candidates.push(duplicate);
            }
            7 => {
                plan.output = OutputPolicy::Step {
                    step: "missing".into(),
                }
            }
            _ => plan.task.required_capabilities = caps("missing"),
        }
        assert!(plan.validate().is_err(), "invalid case {index}");
    }
    let mut value = serde_json::to_value(original).unwrap();
    value["unexpected_authority"] = json!(true);
    assert!(serde_json::from_value::<Workflow>(value).is_err());
    assert!(fixture.inference().is_empty());
}

#[tokio::test]
async fn queued_delegation_cancellation_is_owned_and_terminal_states_are_preserved() {
    let fixture = Fixture::new(vec![], Duration::ZERO, 100).await;
    let task = Task {
        id: TaskId::new(),
        prompt: "queued".into(),
        required_capabilities: caps("research"),
        state: TaskState::Queued,
    };
    fixture
        .agents
        .delegate_task(fixture.coordinator, fixture.workers[0], task.clone())
        .await
        .unwrap();
    assert!(
        fixture
            .agents
            .cancel_queued_task(fixture.workers[1], task.id)
            .is_err()
    );
    assert_eq!(
        fixture.agents.get_task(task.id).await.unwrap().state,
        TaskState::Queued
    );
    assert!(
        fixture
            .agents
            .cancel_queued_task(fixture.workers[0], task.id)
            .unwrap()
    );
    assert!(
        !fixture
            .agents
            .cancel_queued_task(fixture.workers[0], task.id)
            .unwrap()
    );
    assert_eq!(
        fixture.agents.get_task(task.id).await.unwrap().state,
        TaskState::Cancelled
    );
}

#[tokio::test]
async fn candidate_health_is_refreshed_after_prior_step_evidence_expires() {
    let fixture = Fixture::with_health_age(
        vec![(200, "first"), (200, "second")],
        Duration::from_millis(750),
        100,
        Duration::from_millis(500),
    )
    .await;
    let report = fixture
        .orchestrator
        .execute(fixture.plan(true), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(report.task.state, TaskState::Completed);
    assert_eq!(report.attempts.len(), 2);
    assert_eq!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(path, _)| path == "/api/version")
            .count(),
        3
    );
}
