use std::collections::{BTreeMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use vespera_core::{
    AgentId, BrokerEventCursor, BrokerEventEntry, BrokerEventPage, EventEnvelope, EventKind,
    ExternalAttempt, ExternalTaskChange, TaskId, TaskState,
};

use crate::BrokerError;

const RETENTION: usize = 256;
const MAX_PAGE: u32 = 64;

pub(super) struct Transition {
    pub(super) task_id: TaskId,
    pub(super) change: ExternalTaskChange,
    pub(super) state: TaskState,
    pub(super) attempt: Option<ExternalAttempt>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Feed {
    pub(super) stream_id: Uuid,
    latest: u64,
    events: VecDeque<BrokerEventEntry>,
    #[serde(skip)]
    pub(super) dirty: bool,
}

impl Default for Feed {
    fn default() -> Self {
        Self {
            stream_id: Uuid::new_v4(),
            latest: 0,
            events: VecDeque::new(),
            dirty: true,
        }
    }
}

impl Feed {
    fn cursor(&self, sequence: u64) -> BrokerEventCursor {
        BrokerEventCursor {
            stream_id: self.stream_id,
            sequence,
        }
    }

    fn push(&mut self, event: EventEnvelope) {
        self.dirty = true;
        self.latest += 1;
        if self.events.len() == RETENTION {
            self.events.pop_front();
        }
        self.events.push_back(BrokerEventEntry {
            cursor: self.cursor(self.latest),
            event,
        });
    }

    pub(super) fn valid_for(&self, worker: Option<AgentId>) -> bool {
        if self.stream_id.is_nil()
            || self.latest > i64::MAX as u64
            || self.events.len() as u64 != self.latest.min(RETENTION as u64)
        {
            return false;
        }
        let discarded = self.latest - self.events.len() as u64;
        self.events.iter().enumerate().all(|(index, entry)| {
            let EventKind::ExternalTaskChanged {
                task_id, attempt, ..
            } = entry.event.kind
            else {
                return false;
            };
            entry.cursor == self.cursor(discarded + index as u64 + 1)
                && entry.event.source == "worker_broker"
                && entry.event.correlation_task_id == Some(task_id)
                && worker
                    .is_none_or(|worker| attempt.is_some_and(|attempt| attempt.worker_id == worker))
        })
    }

    fn replay(
        &self,
        after: Option<BrokerEventCursor>,
        limit: u32,
    ) -> Result<BrokerEventPage, BrokerError> {
        if !(1..=MAX_PAGE).contains(&limit) {
            return Err(BrokerError::Invalid("event page limit"));
        }
        let discarded_through = self.latest - self.events.len() as u64;
        let sequence = match after {
            Some(cursor) => {
                if cursor.stream_id != self.stream_id {
                    return Err(BrokerError::ForeignCursor);
                }
                if cursor.sequence > self.latest {
                    return Err(BrokerError::Invalid("event cursor sequence"));
                }
                if cursor.sequence < discarded_through {
                    return Err(BrokerError::ExpiredCursor);
                }
                cursor.sequence
            }
            None => discarded_through,
        };
        let entries: Vec<_> = self
            .events
            .iter()
            .skip((sequence - discarded_through) as usize)
            .take(limit as usize)
            .cloned()
            .collect();
        let next_cursor = entries
            .last()
            .map_or_else(|| self.cursor(sequence), |entry| entry.cursor);
        Ok(BrokerEventPage {
            entries,
            next_cursor,
            discarded_through,
            has_more: next_cursor.sequence < self.latest,
        })
    }
}

#[derive(Default)]
pub(super) struct EventFeeds {
    pub(super) operator: Feed,
    pub(super) workers: BTreeMap<AgentId, Feed>,
}

impl EventFeeds {
    pub(super) fn approve(&mut self, worker: AgentId) {
        self.workers.insert(worker, Feed::default());
    }

    pub(super) fn revoke(&mut self, worker: AgentId) {
        self.workers.remove(&worker);
    }

    pub(super) fn publish(&mut self, transition: Transition) {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                elapsed.as_millis().try_into().unwrap_or(u64::MAX)
            });
        let Transition {
            task_id,
            change,
            state,
            attempt,
        } = transition;
        let mut event = EventEnvelope::new(
            "worker_broker",
            timestamp,
            EventKind::ExternalTaskChanged {
                task_id,
                change,
                state,
                attempt,
            },
        )
        .expect("broker event uses validated identities and static source");
        event.correlation_task_id = Some(task_id);
        if let Some(worker_feed) =
            attempt.and_then(|attempt| self.workers.get_mut(&attempt.worker_id))
        {
            worker_feed.push(event.clone());
        }
        self.operator.push(event);
    }

    pub(super) fn replay(
        &self,
        worker: Option<AgentId>,
        after: Option<BrokerEventCursor>,
        limit: u32,
    ) -> Result<BrokerEventPage, BrokerError> {
        let feed = match worker {
            Some(worker) => self.workers.get(&worker).ok_or(BrokerError::Unauthorized)?,
            None => &self.operator,
        };
        feed.replay(after, limit)
    }
}
