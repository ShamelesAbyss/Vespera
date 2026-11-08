//! Integration tests for `vespera-agents`.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use vespera_agents::{AgentRuntime, AgentRuntimeError};
use vespera_core::{
    AgentDefinition, AgentId, AgentIdentity, AgentStatus, Capability, MemoryId, MemoryLayer,
    ModelId, ModelSelectionPolicy, Permission, ResourceBudget, Task, TaskId, TaskState,
};
use vespera_model_registry::ModelRegistry;
use vespera_scheduler::{ResourceScheduler, SchedulingPolicy};

fn build_runtime() -> AgentRuntime {
    build_runtime_with_limits(vespera_agents::RuntimeLimits::default())
}

#[tokio::test]
async fn durable_namespaces_follow_agent_identity_across_model_rebind_and_runtime_restart() {
    use vespera_core::MemoryRecord;
    use vespera_memory::{MemoryOrigin, MemoryQuery, MemoryStore, StorageError, StorageLimits};
    let directory = std::env::temp_dir().join(format!("vespera-agent-memory-{}", MemoryId::new()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("memory.sqlite3");
    let a = AgentId::new();
    let b = AgentId::new();
    let model = ModelId::new();
    let grants = [
        Permission::MemoryRead,
        Permission::MemoryWrite,
        Permission::Delegate,
    ]
    .into();
    let original = sample_agent(a, "same display name", model, grants, AgentStatus::Idle);
    let mut other = original.clone();
    other.identity.id = b;
    let memory = MemoryRecord {
        id: MemoryId::new(),
        agent_id: a,
        layer: MemoryLayer::Observation,
        content: "persistent agent observation".into(),
        created_at_unix_ms: 123,
        source_task_id: None,
    };
    {
        let runtime = build_runtime();
        runtime.register_agent(original.clone()).await.unwrap();
        runtime.register_agent(other.clone()).await.unwrap();
        let store = MemoryStore::open(&path, StorageLimits::default(), runtime.memory_authorizer())
            .unwrap();
        store
            .insert(a, memory.clone(), MemoryOrigin::Observation)
            .unwrap();
        let task = Task {
            id: TaskId::new(),
            prompt: "accepted source".into(),
            required_capabilities: BTreeSet::new(),
            state: TaskState::Queued,
        };
        runtime.delegate_task(a, b, task.clone()).await.unwrap();
        let mut generated = memory.clone();
        generated.id = MemoryId::new();
        generated.source_task_id = Some(task.id);
        assert_eq!(
            store.insert(a, generated.clone(), MemoryOrigin::Generated),
            Err(StorageError::SourceUnavailable)
        );
        generated.agent_id = b;
        store.insert(b, generated, MemoryOrigin::Generated).unwrap();
        runtime
            .rebind_model_policy(
                a,
                ModelSelectionPolicy::Fixed {
                    model_id: ModelId::new(),
                },
            )
            .await
            .unwrap();
        assert_eq!(store.get(a, a, memory.id).unwrap().unwrap().record, memory);
        assert_eq!(
            runtime.get_agent(a).await.unwrap().identity,
            original.identity
        );
    }
    {
        let runtime = build_runtime();
        runtime.register_agent(original).await.unwrap();
        runtime.register_agent(other).await.unwrap();
        let store = MemoryStore::open(&path, StorageLimits::default(), runtime.memory_authorizer())
            .unwrap();
        assert_eq!(store.get(a, a, memory.id).unwrap().unwrap().record, memory);
        assert_eq!(store.get(b, a, memory.id), Err(StorageError::Denied));
        assert_eq!(
            store
                .search(a, a, "persistent", &MemoryQuery::default())
                .unwrap()
                .records
                .len(),
            1
        );
        assert_eq!(store.namespace_usage(b, b).unwrap().records, 1);
    }
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

fn build_runtime_with_limits(limits: vespera_agents::RuntimeLimits) -> AgentRuntime {
    let registry = Arc::new(ModelRegistry::new());
    let policy = SchedulingPolicy {
        queue: vespera_scheduler::QueuePolicy::default(),
        budget: ResourceBudget {
            ram_bytes: 8 * 1024 * 1024 * 1024,
            vram_bytes: 8 * 1024 * 1024 * 1024,
            max_concurrent_inferences: 4,
            max_context_tokens: 8192,
        },
        default_keep_alive: Duration::from_secs(300),
        evict_idle_on_pressure: true,
    };
    let scheduler = Arc::new(ResourceScheduler::new(registry.clone(), policy));
    AgentRuntime::with_limits(registry, scheduler, limits).unwrap()
}

fn sample_agent(
    agent_id: AgentId,
    name: &str,
    model_id: ModelId,
    permissions: BTreeSet<Permission>,
    status: AgentStatus,
) -> AgentDefinition {
    AgentDefinition {
        identity: AgentIdentity {
            id: agent_id,
            name: name.to_string(),
            description: "Autonomous systems architect".to_string(),
            role: "architect".to_string(),
            personality: "analytical".to_string(),
            goals: vec!["maintain system invariants".to_string()],
        },
        model_policy: ModelSelectionPolicy::Fixed { model_id },
        permissions,
        status,
    }
}

#[tokio::test]
async fn rebinding_model_policy_preserves_identity_permissions_and_isolated_memory() {
    let runtime = build_runtime();

    let agent_a: AgentId = "0195f000-0000-7000-8000-000000000101".parse().unwrap();
    let agent_b: AgentId = "0195f000-0000-7000-8000-000000000102".parse().unwrap();
    let model_1: ModelId = "0195f000-0000-7000-8000-000000000201".parse().unwrap();
    let mem_1: MemoryId = "0195f000-0000-7000-8000-000000000301".parse().unwrap();
    let mem_2: MemoryId = "0195f000-0000-7000-8000-000000000302".parse().unwrap();

    let mut perms = BTreeSet::new();
    perms.insert(Permission::MemoryRead);
    perms.insert(Permission::MemoryWrite);

    runtime
        .register_agent(sample_agent(
            agent_a,
            "Vespera-Alpha",
            model_1,
            perms.clone(),
            AgentStatus::Idle,
        ))
        .await
        .unwrap();

    runtime
        .register_agent(sample_agent(
            agent_b,
            "Vespera-Beta",
            model_1,
            perms,
            AgentStatus::Idle,
        ))
        .await
        .unwrap();

    runtime
        .write_memory(
            agent_a,
            mem_1,
            MemoryLayer::Working,
            "alpha working context".to_string(),
            None,
        )
        .await
        .unwrap();

    runtime
        .write_memory(
            agent_b,
            mem_2,
            MemoryLayer::Episodic,
            "beta episodic trace".to_string(),
            None,
        )
        .await
        .unwrap();

    // Rebind Agent A to a new capability-based policy
    let mut required_caps = BTreeSet::new();
    required_caps.insert("reasoning".parse::<Capability>().unwrap());
    runtime
        .rebind_model_policy(
            agent_a,
            ModelSelectionPolicy::Capability {
                required: required_caps.clone(),
            },
        )
        .await
        .unwrap();

    let updated_a = runtime.get_agent(agent_a).await.unwrap();
    assert_eq!(updated_a.identity.name, "Vespera-Alpha");
    assert_eq!(
        updated_a.model_policy,
        ModelSelectionPolicy::Capability {
            required: required_caps
        }
    );

    // Verify Agent A's memory is intact and isolated from Agent B
    let alpha_memories = runtime.read_memory(agent_a, None).await.unwrap();
    assert_eq!(alpha_memories.len(), 1);
    assert_eq!(alpha_memories[0].content, "alpha working context");

    let beta_memories = runtime
        .read_memory(agent_b, Some(MemoryLayer::Episodic))
        .await
        .unwrap();
    assert_eq!(beta_memories.len(), 1);
    assert_eq!(beta_memories[0].content, "beta episodic trace");
}

#[tokio::test]
async fn enforces_permissions_for_memory_messaging_delegation_and_simulation() {
    let runtime = build_runtime();

    let restricted_id: AgentId = "0195f000-0000-7000-8000-000000000401".parse().unwrap();
    let peer_id: AgentId = "0195f000-0000-7000-8000-000000000402".parse().unwrap();
    let model_id: ModelId = "0195f000-0000-7000-8000-000000000501".parse().unwrap();
    let mem_id: MemoryId = "0195f000-0000-7000-8000-000000000601".parse().unwrap();
    let task_id: TaskId = "0195f000-0000-7000-8000-000000000701".parse().unwrap();

    // Register restricted agent with zero permissions
    runtime
        .register_agent(sample_agent(
            restricted_id,
            "Restricted-Agent",
            model_id,
            BTreeSet::new(),
            AgentStatus::Suspended,
        ))
        .await
        .unwrap();

    let mut full_perms = BTreeSet::new();
    full_perms.insert(Permission::Communicate);
    full_perms.insert(Permission::Delegate);
    full_perms.insert(Permission::SimulationAct);

    runtime
        .register_agent(sample_agent(
            peer_id,
            "Coordinator-Agent",
            model_id,
            full_perms,
            AgentStatus::Idle,
        ))
        .await
        .unwrap();

    assert!(matches!(
        runtime
            .write_memory(
                restricted_id,
                mem_id,
                MemoryLayer::LongTerm,
                "secret".to_string(),
                None
            )
            .await,
        Err(AgentRuntimeError::PermissionDenied {
            permission: Permission::MemoryWrite,
            ..
        })
    ));

    assert!(matches!(
        runtime.read_memory(restricted_id, None).await,
        Err(AgentRuntimeError::PermissionDenied {
            permission: Permission::MemoryRead,
            ..
        })
    ));

    assert!(matches!(
        runtime
            .send_message(restricted_id, peer_id, "ping".to_string(), None)
            .await,
        Err(AgentRuntimeError::PermissionDenied {
            permission: Permission::Communicate,
            ..
        })
    ));

    assert!(matches!(
        runtime.authorize_simulation_action(restricted_id).await,
        Err(AgentRuntimeError::PermissionDenied {
            permission: Permission::SimulationAct,
            ..
        })
    ));

    // Coordinator agent has Communicate, Delegate, and SimulationAct permissions
    runtime
        .authorize_simulation_action(peer_id)
        .await
        .expect("simulation action authorized");

    runtime
        .send_message(peer_id, restricted_id, "directive".to_string(), None)
        .await
        .expect("message sent");

    assert!(matches!(
        runtime.inbox_messages(restricted_id).await,
        Err(AgentRuntimeError::PermissionDenied {
            permission: Permission::Communicate,
            ..
        })
    ));
    runtime
        .send_message(peer_id, peer_id, "directive".to_string(), None)
        .await
        .unwrap();
    let inbox = runtime.inbox_messages(peer_id).await.unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].content, "directive");

    let mut caps = BTreeSet::new();
    caps.insert("analysis".parse::<Capability>().unwrap());
    let task = Task {
        id: task_id,
        prompt: "Analyze subsystem telemetry".to_string(),
        required_capabilities: caps,
        state: TaskState::Queued,
    };

    let delegated_id = runtime
        .delegate_task(peer_id, restricted_id, task)
        .await
        .expect("delegation succeeds");
    assert_eq!(delegated_id, task_id);
}

#[tokio::test]
async fn durable_inbox_keeps_its_capacity_and_identity_across_model_rebind_and_restart() {
    use vespera_agents::{AgentTaskStore, AgentTaskStoreError, RuntimeLimits};

    let agent = AgentId::new();
    let path = std::env::temp_dir().join(format!("vespera-agent-inbox-{agent}.sqlite3"));
    let definition = sample_agent(
        agent,
        "fixture",
        ModelId::new(),
        BTreeSet::from([Permission::Communicate]),
        AgentStatus::Idle,
    );
    let limits = RuntimeLimits {
        max_messages_per_agent: 1,
        ..Default::default()
    };
    let runtime = build_runtime_with_limits(limits);
    runtime.register_agent(definition.clone()).await.unwrap();
    runtime
        .attach_task_store(AgentTaskStore::open(&path).unwrap())
        .unwrap();
    let first = runtime
        .send_message(agent, agent, "first".into(), None)
        .await
        .unwrap();
    drop(runtime);

    let runtime = build_runtime_with_limits(limits);
    runtime
        .attach_task_store(AgentTaskStore::open_existing(&path).unwrap())
        .unwrap();
    runtime.register_agent(definition.clone()).await.unwrap();
    runtime
        .rebind_model_policy(
            agent,
            ModelSelectionPolicy::Fixed {
                model_id: ModelId::new(),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        runtime
            .send_message(agent, agent, "over capacity".into(), None)
            .await,
        Err(AgentRuntimeError::TaskStore(AgentTaskStoreError::Capacity))
    ));
    assert_eq!(
        runtime.inbox_messages(agent).await.unwrap(),
        vec![first.clone()]
    );
    drop(runtime);
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "DROP TABLE agent_inbox_source;
             ALTER TABLE agent_messages DROP COLUMN event;
             PRAGMA user_version=4;",
        )
        .unwrap();
    }
    assert!(
        AgentTaskStore::inspect_inbox_audit(&path, None, 16)
            .unwrap()
            .events
            .is_empty()
    );
    let runtime = build_runtime();
    runtime
        .attach_task_store(AgentTaskStore::open_existing(&path).unwrap())
        .unwrap();
    runtime.register_agent(definition).await.unwrap();
    let (second, event) = runtime
        .send_message_observed(agent, agent, "second".into(), None)
        .await
        .unwrap();
    assert_eq!(second.logical_tick, first.logical_tick + 1);
    assert_eq!(
        runtime.inbox_messages(agent).await.unwrap(),
        vec![first, second]
    );
    drop(runtime);
    assert_eq!(
        AgentTaskStore::inspect_inbox_audit(&path, None, 16)
            .unwrap()
            .events,
        vec![event.unwrap()]
    );
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn record_count_and_size_caps_reject_without_partial_mutation() {
    use vespera_agents::RuntimeLimits;
    let runtime = build_runtime_with_limits(RuntimeLimits {
        max_agents: 2,
        max_tasks: 1,
        max_memories_per_agent: 1,
        max_messages_per_agent: 1,
        max_record_bytes: 512,
        ..Default::default()
    });
    let a = AgentId::new();
    let b = AgentId::new();
    let model = ModelId::new();
    let permissions = BTreeSet::from([
        Permission::MemoryRead,
        Permission::MemoryWrite,
        Permission::Communicate,
        Permission::Delegate,
    ]);
    for id in [a, b] {
        runtime
            .register_agent(sample_agent(
                id,
                "fixture",
                model,
                permissions.clone(),
                AgentStatus::Idle,
            ))
            .await
            .unwrap();
    }
    let before = runtime.usage_snapshot().await;
    assert!(matches!(
        runtime
            .register_agent(sample_agent(
                AgentId::new(),
                "third",
                model,
                permissions,
                AgentStatus::Idle
            ))
            .await,
        Err(AgentRuntimeError::Capacity { resource: "agents" })
    ));
    assert_eq!(runtime.usage_snapshot().await, before);
    let memory = MemoryId::new();
    runtime
        .write_memory(a, memory, MemoryLayer::Working, "first".into(), None)
        .await
        .unwrap();
    let before = runtime.usage_snapshot().await;
    assert!(matches!(
        runtime
            .write_memory(b, memory, MemoryLayer::Working, "duplicate".into(), None)
            .await,
        Err(AgentRuntimeError::DuplicateMemory { .. })
    ));
    assert!(matches!(
        runtime
            .write_memory(
                a,
                MemoryId::new(),
                MemoryLayer::Working,
                "second".into(),
                None
            )
            .await,
        Err(AgentRuntimeError::Capacity {
            resource: "agent_memories"
        })
    ));
    assert!(matches!(
        runtime
            .write_memory(
                b,
                MemoryId::new(),
                MemoryLayer::Working,
                "x".repeat(513),
                None
            )
            .await,
        Err(AgentRuntimeError::Capacity {
            resource: "record_bytes"
        })
    ));
    assert_eq!(runtime.usage_snapshot().await, before);
    runtime
        .send_message(a, b, "first".into(), None)
        .await
        .unwrap();
    let before = runtime.usage_snapshot().await;
    assert!(matches!(
        runtime.send_message(a, b, "second".into(), None).await,
        Err(AgentRuntimeError::Capacity {
            resource: "agent_inbox"
        })
    ));
    assert_eq!(runtime.inbox_messages(b).await.unwrap().len(), 1);
    assert_eq!(runtime.usage_snapshot().await, before);
    let task = Task {
        id: TaskId::new(),
        prompt: "p".into(),
        required_capabilities: BTreeSet::new(),
        state: TaskState::Queued,
    };
    runtime.delegate_task(a, b, task.clone()).await.unwrap();
    let before = runtime.usage_snapshot().await;
    assert!(matches!(
        runtime
            .delegate_task(
                a,
                b,
                Task {
                    id: TaskId::new(),
                    ..task
                }
            )
            .await,
        Err(AgentRuntimeError::Capacity { resource: "tasks" })
    ));
    assert_eq!(runtime.usage_snapshot().await, before);
}

#[tokio::test]
async fn aggregate_quota_failure_preserves_memory_identity_and_logical_order() {
    use vespera_agents::RuntimeLimits;
    let runtime = build_runtime_with_limits(RuntimeLimits {
        max_retained_content_bytes: 256,
        max_record_bytes: 512,
        ..Default::default()
    });
    let a = AgentId::new();
    runtime
        .register_agent(sample_agent(
            a,
            "fixture",
            ModelId::new(),
            BTreeSet::from([Permission::MemoryRead, Permission::MemoryWrite]),
            AgentStatus::Idle,
        ))
        .await
        .unwrap();
    let before = runtime.usage_snapshot().await;
    let memory = MemoryId::new();
    assert!(matches!(
        runtime
            .write_memory(a, memory, MemoryLayer::Observation, "x".repeat(256), None)
            .await,
        Err(AgentRuntimeError::Capacity {
            resource: "retained_content_bytes"
        })
    ));
    assert_eq!(runtime.usage_snapshot().await, before);
    let entry = runtime
        .write_memory(a, memory, MemoryLayer::Observation, "small".into(), None)
        .await
        .unwrap();
    assert_eq!(entry.logical_tick, 1);
    assert_eq!(
        runtime.usage_snapshot().await.retained_content_bytes,
        before.retained_content_bytes + 5
    );
}

#[tokio::test]
async fn policy_rebinding_updates_content_quota_transactionally() {
    use vespera_agents::RuntimeLimits;
    let runtime = build_runtime_with_limits(RuntimeLimits {
        max_retained_content_bytes: 5,
        max_record_bytes: 128,
        ..Default::default()
    });
    let a = AgentId::new();
    let model = ModelId::new();
    let mut agent = sample_agent(a, "a", model, BTreeSet::new(), AgentStatus::Idle);
    agent.identity.role = "r".into();
    agent.identity.description.clear();
    agent.identity.personality.clear();
    agent.identity.goals.clear();
    runtime.register_agent(agent).await.unwrap();
    assert_eq!(runtime.usage_snapshot().await.retained_content_bytes, 2);
    assert!(matches!(
        runtime
            .rebind_model_policy(
                a,
                ModelSelectionPolicy::Capability {
                    required: BTreeSet::from(["xxxx".parse().unwrap()])
                }
            )
            .await,
        Err(AgentRuntimeError::Capacity { .. })
    ));
    assert_eq!(
        runtime.get_agent(a).await.unwrap().model_policy,
        ModelSelectionPolicy::Fixed { model_id: model }
    );
    assert_eq!(runtime.usage_snapshot().await.retained_content_bytes, 2);
    runtime
        .rebind_model_policy(
            a,
            ModelSelectionPolicy::Capability {
                required: BTreeSet::from(["x".parse().unwrap()]),
            },
        )
        .await
        .unwrap();
    assert_eq!(runtime.usage_snapshot().await.retained_content_bytes, 3);
    runtime
        .rebind_model_policy(a, ModelSelectionPolicy::Fixed { model_id: model })
        .await
        .unwrap();
    assert_eq!(runtime.usage_snapshot().await.retained_content_bytes, 2);
}

#[tokio::test]
async fn memory_and_message_task_references_require_known_appropriate_owners() {
    let runtime = build_runtime();
    let a = AgentId::new();
    let b = AgentId::new();
    let c = AgentId::new();
    let model = ModelId::new();
    let permissions = BTreeSet::from([
        Permission::MemoryRead,
        Permission::MemoryWrite,
        Permission::Communicate,
        Permission::Delegate,
    ]);
    for id in [a, b, c] {
        runtime
            .register_agent(sample_agent(
                id,
                "fixture",
                model,
                permissions.clone(),
                AgentStatus::Idle,
            ))
            .await
            .unwrap();
    }
    let source = Task {
        id: TaskId::new(),
        prompt: "source task".into(),
        required_capabilities: BTreeSet::new(),
        state: TaskState::Queued,
    };
    runtime.delegate_task(a, b, source.clone()).await.unwrap();
    let before = runtime.usage_snapshot().await;
    assert!(matches!(
        runtime
            .write_memory(
                a,
                MemoryId::new(),
                MemoryLayer::Working,
                "foreign source".into(),
                Some(source.id)
            )
            .await,
        Err(AgentRuntimeError::TaskReferenceDenied { .. })
    ));
    assert!(matches!(
        runtime
            .write_memory(
                b,
                MemoryId::new(),
                MemoryLayer::Working,
                "missing source".into(),
                Some(TaskId::new())
            )
            .await,
        Err(AgentRuntimeError::TaskReferenceDenied { .. })
    ));
    assert!(matches!(
        runtime
            .send_message(a, c, "foreign correlation".into(), Some(source.id))
            .await,
        Err(AgentRuntimeError::TaskReferenceDenied { .. })
    ));
    assert_eq!(runtime.usage_snapshot().await, before);
    let entry = runtime
        .write_memory(
            b,
            MemoryId::new(),
            MemoryLayer::Goal,
            "authorized reference".into(),
            Some(source.id),
        )
        .await
        .unwrap();
    assert_eq!(entry.agent_id, b);
    assert_eq!(entry.source_task_id, Some(source.id));
    runtime
        .send_message(a, b, "recipient task".into(), Some(source.id))
        .await
        .unwrap();
    runtime
        .send_message(b, c, "sender task".into(), Some(source.id))
        .await
        .unwrap();
}

#[tokio::test]
async fn delegated_dispatch_reuses_history_capacity_and_failure_keeps_one_charge() {
    use vespera_agents::RuntimeLimits;
    let runtime = build_runtime_with_limits(RuntimeLimits {
        max_tasks: 1,
        ..Default::default()
    });
    let a = AgentId::new();
    runtime
        .register_agent(sample_agent(
            a,
            "fixture",
            ModelId::new(),
            BTreeSet::from([Permission::Delegate]),
            AgentStatus::Idle,
        ))
        .await
        .unwrap();
    let task = Task {
        id: TaskId::new(),
        prompt: "retained task".into(),
        required_capabilities: BTreeSet::new(),
        state: TaskState::Queued,
    };
    runtime.delegate_task(a, a, task.clone()).await.unwrap();
    let before = runtime.usage_snapshot().await;
    // No provider/model is registered: accepted dispatch fails resolution, retaining its history.
    assert!(
        runtime
            .start_task_execution(
                a,
                task.clone(),
                None,
                vespera_providers::CancellationToken::new()
            )
            .await
            .is_err()
    );
    assert_eq!(
        runtime.get_task(task.id).await.unwrap().state,
        TaskState::Failed
    );
    assert_eq!(runtime.usage_snapshot().await, before);
    runtime
        .set_agent_status(a, AgentStatus::Idle)
        .await
        .unwrap();
    assert!(matches!(
        runtime
            .start_task_execution(
                a,
                Task {
                    id: TaskId::new(),
                    ..task
                },
                None,
                vespera_providers::CancellationToken::new()
            )
            .await,
        Err(AgentRuntimeError::Capacity { resource: "tasks" })
    ));
    assert_eq!(
        runtime.get_agent(a).await.unwrap().status,
        AgentStatus::Idle
    );
    assert_eq!(runtime.usage_snapshot().await, before);
}
