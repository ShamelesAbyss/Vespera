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
    AgentRuntime::new(registry, scheduler)
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
        .send_message(
            peer_id,
            restricted_id,
            "directive".to_string(),
            Some(task_id),
        )
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
        .send_message(peer_id, peer_id, "directive".to_string(), Some(task_id))
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
