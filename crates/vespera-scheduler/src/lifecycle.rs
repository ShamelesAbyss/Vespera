//! Opt-in bounded transport for observed runtime lifecycle transitions.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use vespera_core::{AgentId, AgentStatus, EventEnvelope, EventKind, ModelId, TaskId};

use crate::{ResourceScheduler, lock};

const CAPACITY: usize = 256;

pub(super) struct LifecycleSender {
    sender: mpsc::Sender<EventEnvelope>,
    omitted: Arc<AtomicU64>,
}

pub struct LifecycleEvents {
    pub receiver: mpsc::Receiver<EventEnvelope>,
    /// Envelopes omitted by the producer because the sink was full or unavailable.
    pub omitted: Arc<AtomicU64>,
}

impl ResourceScheduler {
    /// Observe metadata-only lifecycle evidence without blocking execution.
    pub fn observe_lifecycle(&self) -> LifecycleEvents {
        let (sender, receiver) = mpsc::channel(CAPACITY);
        let omitted = Arc::new(AtomicU64::new(0));
        *lock(&self.lifecycle) = Some(LifecycleSender {
            sender,
            omitted: omitted.clone(),
        });
        LifecycleEvents { receiver, omitted }
    }

    /// Close the lifecycle producer and let the consumer drain accepted envelopes.
    pub fn stop_lifecycle_observation(&self) {
        lock(&self.lifecycle).take();
    }

    /// Observe the current accepted registry inventory before a run starts.
    pub async fn observe_model_registrations(&self) {
        for model in self.registry.list_registrations().await {
            self.emit_lifecycle(
                "model-registry",
                None,
                EventKind::ModelRegistered {
                    model_id: model.model_id,
                    provider_id: model.provider_id,
                },
            );
        }
    }

    /// Record a runtime registration after the agent definition has been accepted.
    pub fn observe_agent_registration(&self, agent_id: AgentId, status: AgentStatus) {
        self.emit_lifecycle(
            "agent-runtime",
            None,
            EventKind::AgentRegistered { agent_id, status },
        );
    }

    /// Observe an actual agent status change, with a task correlation when applicable.
    pub fn observe_agent_status(
        &self,
        agent_id: AgentId,
        previous: AgentStatus,
        status: AgentStatus,
        task: Option<TaskId>,
    ) {
        if previous != status {
            self.emit_lifecycle(
                "agent-runtime",
                task,
                EventKind::AgentStatusChanged {
                    agent_id,
                    previous,
                    status,
                },
            );
        }
    }

    /// Observe entry into a managed provider call after the request was validated.
    pub fn observe_inference_started(&self, task_id: TaskId, model_id: ModelId, agent_id: AgentId) {
        self.emit_lifecycle(
            "agent-runtime",
            Some(task_id),
            EventKind::InferenceStarted {
                task_id,
                model_id,
                agent_id: Some(agent_id),
            },
        );
    }

    pub(super) fn emit_lifecycle(
        &self,
        source: &'static str,
        task: Option<TaskId>,
        kind: EventKind,
    ) {
        let observer = lock(&self.lifecycle);
        let Some(observer) = observer.as_ref() else {
            return;
        };
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok());
        let event = timestamp.and_then(|time| EventEnvelope::new(source, time, kind).ok());
        if event.is_none_or(|mut event| {
            event.correlation_task_id = task;
            observer.sender.try_send(event).is_err()
        }) {
            observer.omitted.fetch_add(1, Ordering::Relaxed);
        }
    }
}
