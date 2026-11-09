//! Write synthetic, metadata-only events for native Swarm visual testing.

use std::path::PathBuf;
use vespera_core::{
    AgentId, AgentStatus, EventEnvelope, EventKind, TaskId, TaskState, WorkflowDependency,
};
use vespera_events::Journal;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().ok_or("provide a new fixture journal path")?);
    let live = match args.next().as_deref() {
        None => false,
        Some(flag) if flag == "--live" && args.next().is_none() => true,
        _ => return Err("expected a new fixture journal path and optional --live".into()),
    };
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let mut journal = Journal::initialize(&path, 128)?;
    let agents: [AgentId; 3] = [
        "10000000-0000-4000-8000-000000000001".parse()?,
        "10000000-0000-4000-8000-000000000002".parse()?,
        "10000000-0000-4000-8000-000000000003".parse()?,
    ];
    let root = TaskId::new();
    let [seed, left, right, join, failed, waiting] = std::array::from_fn(|_| TaskId::new());
    let [owner, first, second] = agents;
    let mut events = Vec::new();
    let mut push = |source, correlation, kind| -> Result<(), vespera_core::DomainError> {
        let mut event = EventEnvelope::new(source, 0, kind)?;
        event.correlation_task_id = correlation;
        events.push(event);
        Ok(())
    };
    for agent_id in agents {
        push(
            "agent-runtime",
            None,
            EventKind::AgentRegistered {
                agent_id,
                status: AgentStatus::Idle,
            },
        )?;
    }
    let dependency = |task_id, agent_id| WorkflowDependency { task_id, agent_id };
    for (task_id, to_agent_id, dependencies, state) in [
        (seed, owner, vec![], Some(TaskState::Completed)),
        (
            left,
            first,
            vec![dependency(seed, owner)],
            Some(TaskState::Completed),
        ),
        (
            right,
            second,
            vec![dependency(seed, owner)],
            Some(TaskState::Completed),
        ),
        (
            join,
            owner,
            vec![dependency(left, first), dependency(right, second)],
            Some(TaskState::Completed),
        ),
        (failed, first, vec![], Some(TaskState::Failed)),
        (waiting, second, vec![], None),
    ] {
        let workflow = if [seed, left, right, join].contains(&task_id) {
            root
        } else {
            TaskId::new()
        };
        push(
            "workflow-store",
            Some(workflow),
            EventKind::WorkflowAttemptAccepted {
                task_id,
                from_agent_id: owner,
                to_agent_id,
                dependencies,
            },
        )?;
        if let Some(state) = state {
            push(
                "workflow-store",
                Some(workflow),
                EventKind::WorkflowAttemptFinished { task_id, state },
            )?;
        }
        if task_id == join {
            push(
                "workflow-store",
                Some(root),
                EventKind::TaskCompleted { task_id: root },
            )?;
        }
    }
    push(
        "agent-runtime",
        None,
        EventKind::AgentStatusChanged {
            agent_id: first,
            previous: AgentStatus::Idle,
            status: AgentStatus::Failed,
        },
    )?;
    push(
        "agent-runtime",
        None,
        EventKind::AgentStatusChanged {
            agent_id: second,
            previous: AgentStatus::Idle,
            status: AgentStatus::Running,
        },
    )?;
    push(
        "swarm-visual-fixture",
        None,
        EventKind::AgentMessageSent {
            from_agent_id: first,
            to_agent_id: second,
        },
    )?;
    push(
        "swarm-visual-fixture",
        None,
        EventKind::AgentMessageAcknowledged {
            from_agent_id: first,
            to_agent_id: second,
        },
    )?;
    journal.append_batch(&events)?;
    println!(
        "Wrote {} synthetic visual-test events to {}",
        events.len(),
        path.display()
    );
    if live {
        println!(
            "Open the scratch journal in Swarm, then press Enter here to commit a synthetic message."
        );
        let mut line = String::new();
        while std::io::stdin().read_line(&mut line)? != 0 {
            let cursor = journal.append(&EventEnvelope::new(
                "swarm-visual-fixture",
                0,
                EventKind::AgentMessageSent {
                    from_agent_id: first,
                    to_agent_id: second,
                },
            )?)?;
            println!(
                "Committed synthetic message at sequence {}",
                cursor.sequence
            );
            line.clear();
        }
    }
    Ok(())
}
