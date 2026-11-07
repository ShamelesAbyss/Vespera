//! Bounded priority admission with an explicit maximum number of priority bypasses.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::Notify;
use uuid::Uuid;
use vespera_providers::CancellationToken;

/// Bounds waiting requests and starvation from higher-priority arrivals.
#[derive(Debug, Clone, Copy)]
pub struct QueuePolicy {
    /// Maximum waiting requests, excluding granted permits.
    pub capacity: usize,
    /// Once bypassed this many times, the oldest waiting request wins.
    pub max_priority_bypasses: u32,
}
impl Default for QueuePolicy {
    fn default() -> Self {
        Self {
            capacity: 64,
            max_priority_bypasses: 8,
        }
    }
}

/// Admission queue errors; no provider work is started on rejection.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum QueueError {
    /// The waiting queue is full.
    #[error("scheduler admission queue is full")]
    Full,
    /// Shutdown has stopped new admissions.
    #[error("scheduler admission queue is closed")]
    Closed,
    /// The request was cancelled while waiting.
    #[error("scheduler queue request cancelled")]
    Cancelled,
    /// A zero concurrency limit cannot run requests.
    #[error("scheduler concurrency limit must be positive")]
    InvalidConcurrency,
    /// Arrival ordering cannot be represented safely.
    #[error("scheduler queue arrival counter exhausted")]
    SequenceExhausted,
}

/// Local admission counts; active includes preparation as well as inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueSnapshot {
    /// Waiting requests.
    pub waiting: usize,
    /// Granted, unreleased permits.
    pub active: usize,
    /// Whether new requests are rejected.
    pub closed: bool,
}

struct Entry {
    priority: u8,
    arrival: u64,
    bypasses: u32,
}
#[derive(Default)]
struct State {
    waiting: BTreeMap<Uuid, Entry>,
    active: BTreeSet<Uuid>,
    arrival: u64,
    closed: bool,
}
struct Inner {
    state: Mutex<State>,
    changed: Notify,
    policy: QueuePolicy,
    concurrency: usize,
}
impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Internal coordination queue. It never spawns a detached worker.
#[derive(Clone)]
pub(crate) struct AdmissionQueue(Arc<Inner>);

/// A unique granted queue slot. Releasing or dropping it is idempotent.
pub(crate) struct QueuePermit {
    inner: Arc<Inner>,
    id: Uuid,
}
impl QueuePermit {
    pub(crate) fn release(&self) {
        let removed = self.inner.state().active.remove(&self.id);
        if removed {
            self.inner.changed.notify_waiters();
        }
    }
}
impl Drop for QueuePermit {
    fn drop(&mut self) {
        self.release();
    }
}
struct Ticket {
    inner: Arc<Inner>,
    id: Uuid,
    armed: bool,
}
impl Drop for Ticket {
    fn drop(&mut self) {
        if self.armed {
            self.inner.state().waiting.remove(&self.id);
            self.inner.changed.notify_waiters();
        }
    }
}

impl AdmissionQueue {
    pub(crate) fn new(concurrency: u32, policy: QueuePolicy) -> Self {
        Self(Arc::new(Inner {
            state: Mutex::new(State::default()),
            changed: Notify::new(),
            policy,
            concurrency: concurrency as usize,
        }))
    }
    pub(crate) fn snapshot(&self) -> QueueSnapshot {
        let state = self.0.state();
        QueueSnapshot {
            waiting: state.waiting.len(),
            active: state.active.len(),
            closed: state.closed,
        }
    }
    pub(crate) fn close(&self) {
        {
            let mut state = self.0.state();
            state.closed = true;
            state.waiting.clear();
        }
        self.0.changed.notify_waiters();
    }
    pub(crate) async fn drain(&self, timeout: Duration) -> bool {
        let Some(deadline) = tokio::time::Instant::now().checked_add(timeout) else {
            return false;
        };
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.snapshot().active == 0 {
                return true;
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                return false;
            }
        }
    }
    pub(crate) async fn acquire(
        &self,
        priority: u8,
        cancel: &CancellationToken,
    ) -> Result<QueuePermit, QueueError> {
        if cancel.is_cancelled() {
            return Err(QueueError::Cancelled);
        }
        let id = Uuid::new_v4();
        {
            let mut state = self.0.state();
            if state.closed {
                return Err(QueueError::Closed);
            }
            if self.0.concurrency == 0 {
                return Err(QueueError::InvalidConcurrency);
            }
            if state.waiting.is_empty() && state.active.len() < self.0.concurrency {
                state.active.insert(id);
                return Ok(QueuePermit {
                    inner: self.0.clone(),
                    id,
                });
            }
            if state.waiting.len() >= self.0.policy.capacity {
                return Err(QueueError::Full);
            }
            let arrival = state
                .arrival
                .checked_add(1)
                .ok_or(QueueError::SequenceExhausted)?;
            state.arrival = arrival;
            state.waiting.insert(
                id,
                Entry {
                    priority,
                    arrival,
                    bypasses: 0,
                },
            );
        }
        let mut ticket = Ticket {
            inner: self.0.clone(),
            id,
            armed: true,
        };
        loop {
            let changed = self.0.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if cancel.is_cancelled() {
                return Err(QueueError::Cancelled);
            }
            let granted = {
                let mut state = self.0.state();
                if state.closed {
                    return Err(QueueError::Closed);
                }
                let oldest_forced = state
                    .waiting
                    .iter()
                    .filter(|(_, entry)| entry.bypasses >= self.0.policy.max_priority_bypasses)
                    .min_by_key(|(_, entry)| entry.arrival)
                    .map(|(id, _)| *id);
                let selected = oldest_forced.or_else(|| {
                    state
                        .waiting
                        .iter()
                        .max_by_key(|(_, entry)| (entry.priority, std::cmp::Reverse(entry.arrival)))
                        .map(|(id, _)| *id)
                });
                if state.active.len() < self.0.concurrency && selected == Some(id) {
                    state.waiting.remove(&id);
                    for entry in state.waiting.values_mut() {
                        entry.bypasses = entry.bypasses.saturating_add(1);
                    }
                    state.active.insert(id);
                    true
                } else {
                    false
                }
            };
            if granted {
                ticket.armed = false;
                self.0.changed.notify_waiters();
                return Ok(QueuePermit {
                    inner: self.0.clone(),
                    id,
                });
            }
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(QueueError::Cancelled),
                () = changed => (),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn waiting(queue: &AdmissionQueue, count: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while queue.snapshot().waiting != count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("queue reached expected waiting count");
    }
    fn enqueue(
        queue: &AdmissionQueue,
        priority: u8,
    ) -> tokio::task::JoinHandle<Result<QueuePermit, QueueError>> {
        let queue = queue.clone();
        tokio::spawn(async move { queue.acquire(priority, &CancellationToken::new()).await })
    }

    #[tokio::test]
    async fn queue_capacity_cancellation_and_abort_are_bounded() {
        let queue = AdmissionQueue::new(
            1,
            QueuePolicy {
                capacity: 1,
                max_priority_bypasses: 2,
            },
        );
        let active = queue.acquire(0, &CancellationToken::new()).await.unwrap();
        let request = enqueue(&queue, 0);
        waiting(&queue, 1).await;
        assert!(matches!(
            queue.acquire(0, &CancellationToken::new()).await,
            Err(QueueError::Full)
        ));
        request.abort();
        assert!(matches!(request.await, Err(error) if error.is_cancelled()));
        assert_eq!(queue.snapshot().waiting, 0);
        let cancel = CancellationToken::new();
        let q = queue.clone();
        let token = cancel.clone();
        let request = tokio::spawn(async move { q.acquire(0, &token).await });
        waiting(&queue, 1).await;
        cancel.cancel();
        assert!(matches!(request.await.unwrap(), Err(QueueError::Cancelled)));
        drop(active);
        assert_eq!(queue.snapshot().active, 0);
    }

    #[tokio::test]
    async fn priority_overtaking_is_bounded_and_equal_priority_is_fifo() {
        let queue = AdmissionQueue::new(
            1,
            QueuePolicy {
                capacity: 8,
                max_priority_bypasses: 2,
            },
        );
        let active = queue.acquire(0, &CancellationToken::new()).await.unwrap();
        let low = enqueue(&queue, 0);
        waiting(&queue, 1).await;
        let high1 = enqueue(&queue, 9);
        waiting(&queue, 2).await;
        let high2 = enqueue(&queue, 9);
        waiting(&queue, 3).await;
        let high3 = enqueue(&queue, 9);
        waiting(&queue, 4).await;
        drop(active);
        let grant = high1.await.unwrap().unwrap();
        assert!(!low.is_finished());
        assert!(!high2.is_finished());
        drop(grant);
        let grant = high2.await.unwrap().unwrap();
        assert!(!low.is_finished());
        assert!(!high3.is_finished());
        drop(grant);
        let grant = low.await.unwrap().unwrap();
        assert!(!high3.is_finished());
        drop(grant);
        drop(high3.await.unwrap().unwrap());
        assert_eq!(
            queue.snapshot(),
            QueueSnapshot {
                active: 0,
                waiting: 0,
                closed: false
            }
        );
    }

    #[tokio::test]
    async fn shutdown_rejects_waiters_and_reports_drain_timeout_honestly() {
        let queue = AdmissionQueue::new(1, QueuePolicy::default());
        let active = queue.acquire(0, &CancellationToken::new()).await.unwrap();
        let request = enqueue(&queue, 0);
        waiting(&queue, 1).await;
        queue.close();
        assert!(matches!(request.await.unwrap(), Err(QueueError::Closed)));
        assert!(matches!(
            queue.acquire(0, &CancellationToken::new()).await,
            Err(QueueError::Closed)
        ));
        assert!(!queue.drain(Duration::ZERO).await);
        active.release();
        assert!(queue.drain(Duration::from_secs(1)).await);
        assert_eq!(
            queue.snapshot(),
            QueueSnapshot {
                active: 0,
                waiting: 0,
                closed: true
            }
        );
    }
}
