//! Stable, metadata-only projections of committed workflow evidence.
use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, OptionalExtension, params};
use vespera_core::{EventEnvelope, EventKind, TaskId, TaskState, WorkflowDependency};

use super::{WorkflowStoreError, db};
use crate::{Workflow, WorkflowReport};

const MAX_EVENTS: usize = 97;
pub(super) const MAX_BYTES: usize = MAX_EVENTS * 4096;
const SOURCE: &str = "workflow-store";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Accepted(TaskId),
    Finished(TaskId),
    Root,
}

fn key(kind: &EventKind, root: TaskId) -> Option<Key> {
    match kind {
        EventKind::WorkflowAttemptAccepted { task_id, .. } if *task_id != root => {
            Some(Key::Accepted(*task_id))
        }
        EventKind::WorkflowAttemptFinished { task_id, .. } if *task_id != root => {
            Some(Key::Finished(*task_id))
        }
        EventKind::TaskCompleted { task_id }
        | EventKind::TaskFailed { task_id }
        | EventKind::TaskCancelled { task_id }
            if *task_id == root =>
        {
            Some(Key::Root)
        }
        _ => None,
    }
}

pub(super) fn read(
    connection: &Connection,
    task: TaskId,
) -> Result<Option<Vec<EventEnvelope>>, WorkflowStoreError> {
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db)?;
    if version < 3 {
        return Ok(None);
    }
    let payload = connection.query_row(
        "SELECT CASE WHEN length(audit)>?2 THEN x'' ELSE audit END FROM workflows WHERE task_id=?1",
        params![task.to_string(), MAX_BYTES as i64],
        |row| row.get::<_, Option<Vec<u8>>>(0),
    ).optional().map_err(db)?;
    let Some(Some(payload)) = payload else {
        return Ok(None);
    };
    let events: Vec<EventEnvelope> =
        serde_json::from_slice(&payload).map_err(|_| WorkflowStoreError::InvalidCheckpoint)?;
    let mut keys = BTreeSet::new();
    let mut ids = BTreeSet::new();
    if events.len() > MAX_EVENTS
        || events.iter().any(|event| {
            event.source != SOURCE
                || event.correlation_task_id != Some(task)
                || !ids.insert(event.event_id)
                || !key(&event.kind, task).is_some_and(|key| keys.insert(key))
        })
    {
        return Err(WorkflowStoreError::InvalidCheckpoint);
    }
    Ok(Some(events))
}

pub(super) fn extend(
    events: &mut Vec<EventEnvelope>,
    workflow: &Workflow,
    report: &WorkflowReport,
    timestamp: u64,
) -> Result<(), WorkflowStoreError> {
    let root = workflow.task.id;
    let mut seen: BTreeSet<_> = events
        .iter()
        .filter_map(|event| key(&event.kind, root))
        .collect();
    let mut append = |kind| {
        let key = key(&kind, root).ok_or(WorkflowStoreError::InvalidCheckpoint)?;
        if seen.insert(key) {
            let mut event = EventEnvelope::new(SOURCE, timestamp, kind)
                .map_err(|_| WorkflowStoreError::InvalidCheckpoint)?;
            event.correlation_task_id = Some(root);
            events.push(event);
        }
        Ok::<_, WorkflowStoreError>(())
    };
    let mut completed = BTreeMap::<&str, WorkflowDependency>::new();
    for attempt in report.attempts.iter().filter(|attempt| attempt.delegated) {
        let step = workflow
            .steps
            .iter()
            .find(|step| step.name == attempt.step)
            .ok_or(WorkflowStoreError::InvalidCheckpoint)?;
        let dependencies = step
            .dependencies
            .iter()
            .map(|name| {
                completed
                    .get(name.as_str())
                    .cloned()
                    .ok_or(WorkflowStoreError::InvalidCheckpoint)
            })
            .collect::<Result<_, _>>()?;
        append(EventKind::WorkflowAttemptAccepted {
            task_id: attempt.task_id,
            from_agent_id: workflow.coordinator,
            to_agent_id: attempt.agent_id,
            dependencies,
        })?;
        if let Some(state @ (TaskState::Completed | TaskState::Failed | TaskState::Cancelled)) =
            attempt.task_state
        {
            append(EventKind::WorkflowAttemptFinished {
                task_id: attempt.task_id,
                state,
            })?;
        }
        if attempt.failure.is_none()
            && let Some(result) = &attempt.result
        {
            completed.insert(
                &step.name,
                WorkflowDependency {
                    task_id: result.task.id,
                    agent_id: result.agent_id,
                },
            );
        }
    }
    let terminal = match report.task.state {
        TaskState::Completed => Some(EventKind::TaskCompleted { task_id: root }),
        TaskState::Failed => Some(EventKind::TaskFailed { task_id: root }),
        TaskState::Cancelled => Some(EventKind::TaskCancelled { task_id: root }),
        _ => None,
    };
    if let Some(terminal) = terminal {
        append(terminal)?;
    }
    if events.len() > MAX_EVENTS {
        return Err(WorkflowStoreError::InvalidCheckpoint);
    }
    Ok(())
}
