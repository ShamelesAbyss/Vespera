//! Projection of observed broker events into a bounded renderer-facing graph.

use std::collections::BTreeMap;

use vespera_core::{
    AgentId, BrokerEventEntry, EventKind, ExternalAttempt, ExternalTaskChange, TaskId, TaskState,
};

const MAX_TASKS: usize = 256;
const MAX_WORKERS: usize = 64;

#[derive(Clone, Debug)]
pub struct TaskNode {
    pub slot: usize,
    pub state: TaskState,
    pub attempt: Option<ExternalAttempt>,
    pub change: ExternalTaskChange,
    pub observed_at_unix_ms: u64,
}

#[derive(Clone, Default)]
pub struct Graph {
    pub tasks: BTreeMap<TaskId, TaskNode>,
    pub workers: BTreeMap<AgentId, usize>,
}

impl Graph {
    pub fn observe(&mut self, entry: BrokerEventEntry) {
        let EventKind::ExternalTaskChanged {
            task_id,
            state,
            attempt,
            change,
        } = entry.event.kind
        else {
            return;
        };
        if !self.tasks.contains_key(&task_id) && self.tasks.len() == MAX_TASKS {
            return;
        }
        if let Some(attempt) = attempt
            && self.workers.len() < MAX_WORKERS
        {
            let slot = self.workers.len();
            self.workers.entry(attempt.worker_id).or_insert(slot);
        }
        let slot = self
            .tasks
            .get(&task_id)
            .map_or(self.tasks.len(), |task| task.slot);
        self.tasks.insert(
            task_id,
            TaskNode {
                slot,
                state,
                attempt,
                change,
                observed_at_unix_ms: entry.event.occurred_at_unix_ms,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vespera_core::{BrokerEventCursor, EventEnvelope};

    #[test]
    fn observations_preserve_task_identity_and_do_not_invent_workers() {
        let mut graph = Graph::default();
        let task_id = TaskId::new();
        let worker_id = AgentId::new();
        let stream_id = task_id.to_string().parse().unwrap();
        let entry = |sequence, kind| BrokerEventEntry {
            cursor: BrokerEventCursor {
                stream_id,
                sequence,
            },
            event: EventEnvelope::new("fixture", 42, kind).unwrap(),
        };
        graph.observe(entry(
            1,
            EventKind::AgentCreated {
                agent_id: worker_id,
            },
        ));
        assert!(graph.workers.is_empty());
        graph.observe(entry(
            2,
            EventKind::ExternalTaskChanged {
                task_id,
                state: TaskState::Queued,
                attempt: None,
                change: ExternalTaskChange::Submitted,
            },
        ));
        assert!(graph.workers.is_empty());
        graph.observe(entry(
            3,
            EventKind::ExternalTaskChanged {
                task_id,
                state: TaskState::Running,
                change: ExternalTaskChange::Claimed,
                attempt: Some(ExternalAttempt {
                    worker_id,
                    attempt_id: TaskId::new().to_string().parse().unwrap(),
                    generation: 1,
                }),
            },
        ));
        assert_eq!(graph.tasks.len(), 1);
        assert_eq!(graph.tasks[&task_id].slot, 0);
        assert_eq!(graph.tasks[&task_id].state, TaskState::Running);
        assert_eq!(graph.workers.len(), 1);
    }
}
