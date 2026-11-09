//! Async client for the Vespera v2 worker protocol over loopback or TLS.
//!
//! Use an operator key for task submission and a worker key for execution.
//! Each action obtains the server's session and signs a fresh proof. Calls
//! use HTTP/1 with a timeout for each exchange. Only the opt-in read-only
//! event subscription reconnects automatically.
//!
//! ```no_run
//! use vespera_sdk::{SigningKey, WorkerClient};
//!
//! async fn run_one() -> Result<(), Box<dyn std::error::Error>> {
//!     let worker = std::env::var("VESPERA_WORKER_ID")?.parse()?;
//!     let key = SigningKey::from_hex(&std::env::var("VESPERA_WORKER_TOKEN")?)?;
//!     let client = WorkerClient::new("127.0.0.1:8765".parse()?, worker, key)?;
//!     if let Some(lease) = client.claim().await? {
//!         // Replace this example result with the worker's authorized computation.
//!         client.complete(lease.fence, "Acknowledged.").await?;
//!     }
//!     Ok(())
//! }
//! ```

mod subscription;
mod transport;

use std::collections::BTreeSet;
use std::net::SocketAddr;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use subscription::{EventSubscription, ResumableEventSubscription};
/// Identity of one running broker session, obtained when opening an event stream.
pub type BrokerSessionId = Uuid;
pub use vespera_core::{
    AgentId, BrokerEventCursor, BrokerEventEntry, BrokerEventPage, BrokerStreamEnd,
    BrokerStreamFrame, Capability, EventEnvelope, EventKind, ExternalAttempt, ExternalTaskChange,
    Task, TaskId, TaskState,
};

use transport::Transport;

/// A principal's signing key. Keep it private and supply it out of band.
pub struct SigningKey([u8; 32]);

impl SigningKey {
    pub fn from_hex(value: &str) -> Result<Self, ClientError> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ClientError::InvalidKey);
        }
        let mut key = [0; 32];
        let (pairs, _) = value.as_bytes().as_chunks::<2>();
        for (byte, digits) in key.iter_mut().zip(pairs) {
            let digits = std::str::from_utf8(digits).map_err(|_| ClientError::InvalidKey)?;
            *byte = u8::from_str_radix(digits, 16).map_err(|_| ClientError::InvalidKey)?;
        }
        Ok(Self(key))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(
        "endpoint must be a concrete numeric address with a nonzero port and TLS for remote access"
    )]
    InvalidEndpoint,
    #[error("TLS trust roots must contain one to sixteen valid PEM certificates")]
    InvalidTrustRoots,
    #[error("signing key must contain 64 lowercase hexadecimal characters")]
    InvalidKey,
    #[error("worker identity is invalid")]
    InvalidWorker,
    #[error("could not initialize worker HTTP client")]
    ClientInitialization,
    #[error("could not encode worker request")]
    Encoding,
    #[error("worker request exceeds the HTTP body limit")]
    RequestTooLarge,
    #[error("worker HTTP exchange timed out")]
    Timeout,
    #[error("worker HTTP exchange failed")]
    Transport,
    #[error("broker session changed before the request was sent")]
    SessionChanged,
    #[error("worker service sent an unexpected WebSocket frame")]
    InvalidEventFrame,
    #[error("worker HTTP response exceeds the body limit")]
    ResponseTooLarge,
    #[error("worker service sent an unexpected response (HTTP {status})")]
    UnexpectedResponse { status: u16 },
    #[error("worker service rejected the request (HTTP {status}, {code:?})")]
    Rejected {
        status: u16,
        code: Option<ErrorCode>,
    },
}

/// Known v2 error codes. A newer or non-JSON error may have only a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidJson,
    InvalidInput,
    InvalidBody,
    UnexpectedQuery,
    Unauthorized,
    UnknownRoute,
    UnknownTask,
    UnknownWorker,
    ExpectedGet,
    ExpectedPost,
    WorkerBusy,
    DuplicateTask,
    StaleLease,
    ResultConflict,
    ForeignCursor,
    CursorExpired,
    BodyLimit,
    BodyTimeout,
    ExpectedJson,
    NonceCapacity,
    Capacity,
    StorageUnavailable,
    InvalidUpgrade,
    InvalidCursor,
    SubscriptionCapacity,
    MessageConflict,
    InboxCapacity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fence {
    pub task_id: TaskId,
    pub attempt_id: Uuid,
    pub generation: u32,
}

#[derive(Debug, Deserialize)]
pub struct Lease {
    pub task: Task,
    pub fence: Fence,
    pub expires_in_ms: u64,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
pub struct AcceptedResult {
    pub worker: AgentId,
    pub fence: Fence,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct TaskInspection {
    pub state: TaskState,
    pub result: Option<AcceptedResult>,
}

#[derive(Debug, Serialize)]
pub struct Submission {
    pub task_id: TaskId,
    pub prompt: String,
    pub required_capabilities: BTreeSet<Capability>,
    pub eligible_workers: BTreeSet<AgentId>,
    pub lifetime_seconds: u64,
    pub max_attempts: u32,
}

#[derive(Debug, Deserialize)]
pub struct SubmittedTask {
    pub task_id: TaskId,
    pub state: TaskState,
}

#[derive(Debug, Deserialize)]
pub struct WorkerMessage {
    pub message_id: Uuid,
    pub sender: AgentId,
    pub content: String,
}

#[derive(Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum OperatorRequest<'a> {
    Submit(&'a Submission),
    Inspect {
        task_id: TaskId,
    },
    Cancel {
        task_id: TaskId,
    },
    Revoke {
        worker_id: AgentId,
    },
    Replay {
        after: Option<BrokerEventCursor>,
        limit: u32,
    },
    ReplayTaskAudit {
        after: Option<BrokerEventCursor>,
        limit: u32,
    },
}

#[derive(Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum WorkerRequest<'a> {
    Claim {},
    Renew {
        fence: Fence,
    },
    Complete {
        fence: Fence,
        content: &'a str,
    },
    Replay {
        after: Option<BrokerEventCursor>,
        limit: u32,
    },
    SendMessage {
        recipient: AgentId,
        message_id: Uuid,
        content: &'a str,
    },
    Inbox {},
    AcknowledgeMessage {
        sender: AgentId,
        message_id: Uuid,
    },
}

#[derive(Deserialize)]
struct StateReply {
    state: TaskState,
}

#[derive(Deserialize)]
struct Renewal {
    expires_in_ms: u64,
}

/// Operator access to submit, inspect and cancel tasks and revoke workers.
pub struct OperatorClient(Transport);

impl OperatorClient {
    /// Connect over TLS using only the supplied PEM trust roots and the endpoint IP as identity.
    pub fn new_tls(
        address: SocketAddr,
        key: SigningKey,
        roots_pem: &[u8],
    ) -> Result<Self, ClientError> {
        Transport::new_tls(address, "/v2/operator", "operator".into(), key, roots_pem).map(Self)
    }
    /// Subscribe to live broker events with the operator's cursor.
    pub async fn subscribe(
        &self,
        after: Option<BrokerEventCursor>,
    ) -> Result<EventSubscription, ClientError> {
        EventSubscription::connect(&self.0, after).await
    }
    /// Read operator events with bounded reconnects and explicit cursor acknowledgement.
    pub fn subscribe_resumable(
        &self,
        after: Option<BrokerEventCursor>,
    ) -> ResumableEventSubscription<'_> {
        ResumableEventSubscription::new(&self.0, after)
    }
    /// Read a page of observed broker events, including all workers.
    pub async fn replay(
        &self,
        after: Option<BrokerEventCursor>,
        limit: u32,
    ) -> Result<BrokerEventPage, ClientError> {
        self.0
            .post(&OperatorRequest::Replay { after, limit })
            .await?
            .json(StatusCode::OK)
            .await
    }
    /// Read task transitions from a retention window separate from mailbox activity.
    ///
    /// Use a cursor issued by this method to resume a task audit.
    pub async fn replay_task_audit(
        &self,
        after: Option<BrokerEventCursor>,
        limit: u32,
    ) -> Result<BrokerEventPage, ClientError> {
        self.0
            .post(&OperatorRequest::ReplayTaskAudit { after, limit })
            .await?
            .json(StatusCode::OK)
            .await
    }
    pub fn new(address: SocketAddr, key: SigningKey) -> Result<Self, ClientError> {
        Transport::new(address, "/v2/operator", "operator".into(), key).map(Self)
    }

    pub async fn submit(&self, submission: &Submission) -> Result<SubmittedTask, ClientError> {
        self.0
            .post(&OperatorRequest::Submit(submission))
            .await?
            .json(StatusCode::CREATED)
            .await
    }

    pub async fn inspect(&self, task_id: TaskId) -> Result<TaskInspection, ClientError> {
        self.0
            .post(&OperatorRequest::Inspect { task_id })
            .await?
            .json(StatusCode::OK)
            .await
    }

    pub async fn cancel(&self, task_id: TaskId) -> Result<TaskState, ClientError> {
        self.cancel_with_session(task_id, None).await
    }

    /// Cancel an observed task only in the broker session that supplied its view.
    pub async fn cancel_in_session(
        &self,
        task_id: TaskId,
        session: BrokerSessionId,
    ) -> Result<TaskState, ClientError> {
        self.cancel_with_session(task_id, Some(session)).await
    }

    async fn cancel_with_session(
        &self,
        task_id: TaskId,
        session: Option<BrokerSessionId>,
    ) -> Result<TaskState, ClientError> {
        let reply: StateReply = self
            .0
            .post_in_session(&OperatorRequest::Cancel { task_id }, session)
            .await?
            .json(StatusCode::OK)
            .await?;
        Ok(reply.state)
    }

    pub async fn revoke(&self, worker_id: AgentId) -> Result<(), ClientError> {
        self.0
            .post(&OperatorRequest::Revoke { worker_id })
            .await?
            .no_content()
            .await
    }
}

/// Worker access to claim, renew and complete attempts for one approved ID.
pub struct WorkerClient(Transport);

impl WorkerClient {
    /// Deliver text on an operator-approved route using a caller-generated message ID.
    pub async fn send_message(
        &self,
        recipient: AgentId,
        message_id: Uuid,
        content: &str,
    ) -> Result<(), ClientError> {
        self.0
            .post(&WorkerRequest::SendMessage {
                recipient,
                message_id,
                content,
            })
            .await?
            .no_content()
            .await
    }

    /// Read this worker's unacknowledged messages in arrival order.
    pub async fn inbox(&self) -> Result<Vec<WorkerMessage>, ClientError> {
        self.0
            .post(&WorkerRequest::Inbox {})
            .await?
            .json(StatusCode::OK)
            .await
    }

    /// Acknowledge an inbox entry using the sender and ID from that entry.
    pub async fn acknowledge_message(
        &self,
        sender: AgentId,
        message_id: Uuid,
    ) -> Result<(), ClientError> {
        self.0
            .post(&WorkerRequest::AcknowledgeMessage { sender, message_id })
            .await?
            .no_content()
            .await
    }
    /// Connect over TLS using only the supplied PEM trust roots and the endpoint IP as identity.
    pub fn new_tls(
        address: SocketAddr,
        worker_id: AgentId,
        key: SigningKey,
        roots_pem: &[u8],
    ) -> Result<Self, ClientError> {
        worker_id
            .validate()
            .map_err(|_| ClientError::InvalidWorker)?;
        Transport::new_tls(address, "/v2/worker", worker_id.to_string(), key, roots_pem).map(Self)
    }
    /// Subscribe to live events for this worker's assigned attempts.
    pub async fn subscribe(
        &self,
        after: Option<BrokerEventCursor>,
    ) -> Result<EventSubscription, ClientError> {
        EventSubscription::connect(&self.0, after).await
    }
    /// Read this worker's events with bounded reconnects and explicit cursor acknowledgement.
    pub fn subscribe_resumable(
        &self,
        after: Option<BrokerEventCursor>,
    ) -> ResumableEventSubscription<'_> {
        ResumableEventSubscription::new(&self.0, after)
    }
    /// Read a page of events about attempts assigned to this worker.
    pub async fn replay(
        &self,
        after: Option<BrokerEventCursor>,
        limit: u32,
    ) -> Result<BrokerEventPage, ClientError> {
        self.0
            .post(&WorkerRequest::Replay { after, limit })
            .await?
            .json(StatusCode::OK)
            .await
    }
    pub fn new(
        address: SocketAddr,
        worker_id: AgentId,
        key: SigningKey,
    ) -> Result<Self, ClientError> {
        worker_id
            .validate()
            .map_err(|_| ClientError::InvalidWorker)?;
        Transport::new(address, "/v2/worker", worker_id.to_string(), key).map(Self)
    }

    /// Returns `None` when no task is currently eligible.
    pub async fn claim(&self) -> Result<Option<Lease>, ClientError> {
        let reply = self.0.post(&WorkerRequest::Claim {}).await?;
        if reply.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        reply.json(StatusCode::OK).await.map(Some)
    }

    /// Return the new lease duration remaining, in milliseconds.
    pub async fn renew(&self, fence: Fence) -> Result<u64, ClientError> {
        let reply: Renewal = self
            .0
            .post(&WorkerRequest::Renew { fence })
            .await?
            .json(StatusCode::OK)
            .await?;
        Ok(reply.expires_in_ms)
    }

    /// An exact completion retry uses a fresh proof with the same fence and content.
    pub async fn complete(
        &self,
        fence: Fence,
        content: &str,
    ) -> Result<AcceptedResult, ClientError> {
        self.0
            .post(&WorkerRequest::Complete { fence, content })
            .await?
            .json(StatusCode::OK)
            .await
    }
}
