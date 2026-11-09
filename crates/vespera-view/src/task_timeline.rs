use std::collections::VecDeque;

use vespera_core::{BrokerEventEntry, EventKind, ExternalTaskChange, TaskId};
use vespera_view::Graph;

const MAX_TRANSITIONS: usize = 256;

#[derive(Clone, Default)]
pub(super) struct Projection {
    pub graph: Graph,
    pub recent: VecDeque<(u64, TaskId, ExternalTaskChange)>,
}

impl Projection {
    fn observe(&mut self, entry: BrokerEventEntry) {
        let EventKind::ExternalTaskChanged {
            task_id, change, ..
        } = entry.event.kind
        else {
            return;
        };
        if self.recent.len() == 5 {
            self.recent.pop_front();
        }
        self.recent
            .push_back((entry.cursor.sequence, task_id, change));
        self.graph.observe(entry);
    }
}

#[derive(Clone, Default)]
struct Window {
    baseline: Projection,
    entries: VecDeque<BrokerEventEntry>,
    floor: u64,
    head: u64,
    generation: u64,
    ready: bool,
}

struct Frozen {
    window: Window,
    sequence: u64,
}

#[derive(Default)]
pub(super) struct Timeline {
    window: Window,
    live: Projection,
    frozen: Option<Frozen>,
}

impl Timeline {
    pub fn ready(&mut self, discarded_through: u64) {
        self.window.ready = true;
        if self.window.head == 0 {
            self.window.floor = discarded_through;
            self.window.head = discarded_through;
        }
    }

    pub fn observe(&mut self, entry: BrokerEventEntry) -> bool {
        self.window.head = entry.cursor.sequence;
        if !matches!(entry.event.kind, EventKind::ExternalTaskChanged { .. }) {
            return false;
        }
        self.live.observe(entry.clone());
        self.window.entries.push_back(entry);
        if self.window.entries.len() > MAX_TRANSITIONS
            && let Some(oldest) = self.window.entries.pop_front()
        {
            self.window.floor = oldest.cursor.sequence;
            self.window.baseline.observe(oldest);
        }
        !self.is_frozen()
    }

    pub fn reset(&mut self) {
        self.window = Window {
            generation: self.window.generation.wrapping_add(1),
            ..Window::default()
        };
        self.live = Projection::default();
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen.is_some()
    }

    pub fn has_replay_source(&self) -> bool {
        self.frozen
            .as_ref()
            .is_some_and(|frozen| frozen.window.ready)
    }

    pub fn source_changed(&self) -> bool {
        self.frozen
            .as_ref()
            .is_some_and(|frozen| frozen.window.generation != self.window.generation)
    }

    pub fn toggle(&mut self) {
        if self.frozen.is_some() {
            self.frozen = None;
        } else {
            self.freeze();
        }
    }

    fn freeze(&mut self) -> &mut Frozen {
        self.frozen.get_or_insert_with(|| Frozen {
            window: self.window.clone(),
            sequence: self.window.head,
        })
    }

    pub fn step(&mut self, earlier: bool) {
        let frozen = self.freeze();
        let positions = frozen
            .window
            .entries
            .iter()
            .map(|entry| entry.cursor.sequence)
            .chain([frozen.window.floor, frozen.window.head]);
        let next = if earlier {
            positions.filter(|seq| *seq < frozen.sequence).max()
        } else {
            positions.filter(|seq| *seq > frozen.sequence).min()
        };
        if let Some(sequence) = next {
            frozen.sequence = sequence;
        }
    }

    pub fn projection(&self) -> Projection {
        let Some(frozen) = &self.frozen else {
            return self.live.clone();
        };
        let mut result = frozen.window.baseline.clone();
        for entry in frozen
            .window
            .entries
            .iter()
            .take_while(|entry| entry.cursor.sequence <= frozen.sequence)
        {
            result.observe(entry.clone());
        }
        result
    }

    pub fn label(&self, connected: bool) -> String {
        let live = self.window.head;
        let Some(frozen) = &self.frozen else {
            return if connected {
                format!("Live #{live}")
            } else if self.window.ready {
                format!("Last observed #{live}")
            } else {
                "No broker observations".into()
            };
        };
        if !frozen.window.ready {
            return "Replay unavailable at captured position".into();
        }
        let source = if self.source_changed() {
            "new feed"
        } else if !connected {
            "last observed"
        } else {
            "live"
        };
        format!(
            "Replay #{} ({}-{}) | {source} #{live}",
            frozen.sequence, frozen.window.floor, frozen.window.head
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vespera_core::{BrokerEventCursor, EventEnvelope, TaskState};

    #[test]
    fn frozen_window_remembers_whether_a_broker_was_observed() {
        let mut timeline = Timeline::default();
        timeline.toggle();
        assert!(!timeline.has_replay_source());
        assert_eq!(
            timeline.label(false),
            "Replay unavailable at captured position"
        );
        timeline.ready(0);
        assert!(!timeline.has_replay_source());
        timeline.toggle();
        timeline.toggle();
        assert!(timeline.has_replay_source());
        assert_eq!(timeline.label(false), "Replay #0 (0-0) | last observed #0");
        timeline.reset();
        assert!(timeline.has_replay_source());
    }

    #[test]
    fn capture_replays_exact_prefix_after_rollover_and_live_reset() {
        let task_id = TaskId::new();
        let stream_id = task_id.to_string().parse().unwrap();
        let entry = |sequence, kind| BrokerEventEntry {
            cursor: BrokerEventCursor {
                stream_id,
                sequence,
            },
            event: EventEnvelope::new("fixture", 42, kind).unwrap(),
        };
        let submitted = |id| EventKind::ExternalTaskChanged {
            task_id: id,
            state: TaskState::Queued,
            attempt: None,
            change: ExternalTaskChange::Submitted,
        };
        let mut timeline = Timeline::default();
        timeline.ready(10);
        timeline.observe(entry(11, submitted(task_id)));
        for sequence in 12..=267 {
            timeline.observe(entry(sequence, submitted(TaskId::new())));
        }
        assert_eq!(timeline.window.floor, 11);
        assert_eq!(timeline.window.entries.len(), MAX_TRANSITIONS);
        assert_eq!(
            timeline.window.baseline.graph.tasks[&task_id].state,
            TaskState::Queued
        );
        timeline.observe(entry(
            268,
            EventKind::ExternalTaskChanged {
                task_id,
                state: TaskState::Cancelled,
                attempt: None,
                change: ExternalTaskChange::Cancelled,
            },
        ));
        timeline.step(true);
        assert_eq!(
            timeline.projection().graph.tasks[&task_id].state,
            TaskState::Queued
        );
        assert_eq!(timeline.projection().recent.back().unwrap().0, 267);
        timeline.step(false);
        assert_eq!(
            timeline.projection().graph.tasks[&task_id].state,
            TaskState::Cancelled
        );
        timeline.reset();
        assert!(timeline.source_changed());
        timeline.observe(entry(1, submitted(TaskId::new())));
        assert_eq!(
            timeline.projection().graph.tasks[&task_id].state,
            TaskState::Cancelled
        );
        timeline.toggle();
        assert!(!timeline.projection().graph.tasks.contains_key(&task_id));
    }
}
