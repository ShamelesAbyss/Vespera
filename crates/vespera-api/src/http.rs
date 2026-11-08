//! Bounded HTTP/1 service for approved workers, with opt-in TLS.

mod auth;
pub(super) mod mailbox;
mod subscription;
mod tls;

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::fs::File;
use std::io::{self, Read};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{CACHE_CONTROL, CONNECTION, CONTENT_TYPE, HeaderMap, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, Semaphore, mpsc};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use vespera_core::{AgentId, BrokerEventCursor, Capability, Task, TaskId, TaskState};

use crate::storage::Store;
pub use crate::storage::StoreError;
use crate::{BrokerError, Fence, LeaseBroker, MAX_WORKERS};
use auth::{Authentication, Principal, Proof, SCHEME};

const MAX_ACCESS_FILE: u64 = 128 * 1024;
const MAX_BODY: usize = 512 * 1024;
const MAX_CONNECTIONS: usize = 32;
type Reply = Response<Full<Bytes>>;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error(
        "worker HTTP service requires a numeric loopback address or explicit TLS remote access"
    )]
    NonLoopback,
    #[error("could not read bounded TLS identity files")]
    TlsIo(#[source] io::Error),
    #[error("invalid TLS certificate chain or private key")]
    TlsIdentity,
    #[error("cannot read worker access file: {0}")]
    AccessIo(io::Error),
    #[error("worker access file is invalid or exceeds 128 KiB")]
    AccessFormat,
    #[error("credential reference is invalid, missing or contains an invalid signing key")]
    Credential,
    #[error("each worker and the operator must have a distinct signing key")]
    DuplicateCredential,
    #[error(transparent)]
    Broker(#[from] BrokerError),
    #[error(transparent)]
    Storage(#[from] StoreError),
    #[error("worker HTTP listener failed: {0}")]
    Listener(#[from] io::Error),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessFile {
    operator_token_env: String,
    workers: Vec<WorkerAccess>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerAccess {
    id: AgentId,
    capabilities: BTreeSet<Capability>,
    token_env: String,
    #[serde(default)]
    message_targets: BTreeSet<AgentId>,
}

struct Service {
    auth: Authentication,
    broker: LeaseBroker,
    store: Option<Store>,
    storage_failed: bool,
    mailboxes: mailbox::Mailboxes,
}

impl Service {
    fn load(path: &Path) -> Result<Self, ServerError> {
        let mut bytes = Vec::new();
        File::open(path)
            .and_then(|file| file.take(MAX_ACCESS_FILE + 1).read_to_end(&mut bytes))
            .map_err(ServerError::AccessIo)?;
        if bytes.len() as u64 > MAX_ACCESS_FILE {
            return Err(ServerError::AccessFormat);
        }
        let policy: AccessFile =
            serde_json::from_slice(&bytes).map_err(|_| ServerError::AccessFormat)?;
        if policy.workers.len() > MAX_WORKERS {
            return Err(ServerError::AccessFormat);
        }
        let mut service = Self {
            auth: Authentication::new(),
            broker: LeaseBroker::default(),
            store: None,
            storage_failed: false,
            mailboxes: mailbox::Mailboxes::default(),
        };
        service
            .auth
            .approve(Principal::Operator, &policy.operator_token_env)?;
        for worker in policy.workers {
            service
                .broker
                .approve_worker(worker.id, worker.capabilities)?;
            service
                .auth
                .approve(Principal::Worker(worker.id), &worker.token_env)?;
            service.mailboxes.approve(worker.id, worker.message_targets);
        }
        service.mailboxes.validate_routes()?;
        Ok(service)
    }

    fn apply(&mut self, principal: Principal, bytes: &[u8]) -> Result<Reply, HttpError> {
        if self.storage_failed {
            return Err(STORAGE_UNAVAILABLE);
        }
        let mut revoked = None;
        let result = match principal {
            Principal::Operator => {
                let operation = decode(bytes)?;
                if let OperatorRequest::Revoke { worker_id } = &operation {
                    revoked = Some(*worker_id);
                }
                operate(self, operation)
            }
            Principal::Worker(worker) => work(
                &mut self.broker,
                &mut self.mailboxes,
                worker,
                decode(bytes)?,
            ),
        };
        self.persist(revoked.filter(|_| result.is_ok()))?;
        result
    }

    fn persist(&mut self, revoked: Option<AgentId>) -> Result<(), HttpError> {
        if let Some(store) = &mut self.store
            && store
                .flush(&mut self.broker, &mut self.mailboxes, revoked, false)
                .is_err()
        {
            self.storage_failed = true;
            return Err(STORAGE_UNAVAILABLE);
        }
        Ok(())
    }
}

/// Explicit TLS identity and remote binding policy.
pub struct TlsOptions<'a> {
    pub certificate: &'a Path,
    pub private_key: &'a Path,
    pub allow_remote: bool,
}

/// A bound worker listener with an optional durable lease broker.
pub struct Server {
    listener: TcpListener,
    address: SocketAddr,
    service: Arc<Mutex<Service>>,
    tls: Option<TlsAcceptor>,
}

impl Server {
    pub async fn bind(
        address: SocketAddr,
        access_file: impl AsRef<Path>,
    ) -> Result<Self, ServerError> {
        Self::bind_with_database(address, access_file, None).await
    }

    /// Bind the worker service and optionally recover an exclusive broker database.
    pub async fn bind_with_database(
        address: SocketAddr,
        access_file: impl AsRef<Path>,
        database: Option<&Path>,
    ) -> Result<Self, ServerError> {
        Self::bind_with_transport(address, access_file, database, None).await
    }

    /// Bind with TLS when configured. A non-loopback bind also requires opt-in.
    pub async fn bind_with_transport(
        address: SocketAddr,
        access_file: impl AsRef<Path>,
        database: Option<&Path>,
        tls: Option<TlsOptions<'_>>,
    ) -> Result<Self, ServerError> {
        if !address.ip().is_loopback() && !tls.as_ref().is_some_and(|options| options.allow_remote)
        {
            return Err(ServerError::NonLoopback);
        }
        let tls = tls
            .map(|options| tls::acceptor(options.certificate, options.private_key))
            .transpose()?;
        let mut service = Service::load(access_file.as_ref())?;
        let listener = TcpListener::bind(address).await?;
        if let Some(path) = database {
            let (store, revoked) = Store::open(path, &mut service.broker, &mut service.mailboxes)?;
            for worker in revoked {
                service.auth.revoke(worker);
            }
            service.store = Some(store);
        }
        let address = listener.local_addr()?;
        Ok(Self {
            listener,
            address,
            service: Arc::new(Mutex::new(service)),
            tls,
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn scheme(&self) -> &'static str {
        if self.tls.is_some() { "https" } else { "http" }
    }

    /// Serve until the listener fails or the caller drops this future.
    pub async fn serve(self) -> Result<(), ServerError> {
        let mut connections = JoinSet::new();
        let live_slots = Arc::new(Semaphore::new(8));
        let changed = Arc::new(Notify::new());
        loop {
            tokio::select! {
                Some(_) = connections.join_next(), if !connections.is_empty() => {}
                accepted = self.listener.accept(), if connections.len() < MAX_CONNECTIONS => {
                    let (stream, _) = accepted?;
                    let state = Arc::clone(&self.service);
                    let slots = Arc::clone(&live_slots);
                    let changed = Arc::clone(&changed);
                    let tls = self.tls.clone();
                    connections.spawn(async move {
                        if let Some(acceptor) = tls {
                            if let Ok(Ok(stream)) = tokio::time::timeout(Duration::from_secs(5), acceptor.accept(stream)).await {
                                serve_peer(stream, &state, &slots, &changed).await;
                            }
                        } else {
                            serve_peer(stream, &state, &slots, &changed).await;
                        }
                    });
                }
            }
        }
    }
}

async fn serve_peer(
    stream: impl AsyncRead + AsyncWrite + Unpin + Send + 'static,
    state: &Mutex<Service>,
    slots: &Arc<Semaphore>,
    changed: &Notify,
) {
    let (upgrade_tx, mut upgrade_rx) = mpsc::channel(1);
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(Duration::from_secs(5))
        .max_buf_size(16 * 1024);
    let connection = builder
        .serve_connection(
            TokioIo::new(stream),
            service_fn(|request| dispatch(state, request, slots, changed, &upgrade_tx)),
        )
        .with_upgrades();
    if matches!(
        tokio::time::timeout(Duration::from_secs(15), connection).await,
        Ok(Ok(()))
    ) && let Ok(subscription) = upgrade_rx.try_recv()
    {
        subscription.run(state, changed).await;
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum OperatorRequest {
    Submit {
        task_id: TaskId,
        prompt: String,
        required_capabilities: BTreeSet<Capability>,
        eligible_workers: BTreeSet<AgentId>,
        lifetime_seconds: u64,
        max_attempts: u32,
    },
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
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum WorkerRequest {
    Claim {},
    Renew {
        fence: Fence,
    },
    Complete {
        fence: Fence,
        content: String,
    },
    Replay {
        after: Option<BrokerEventCursor>,
        limit: u32,
    },
    SendMessage {
        recipient: AgentId,
        message_id: uuid::Uuid,
        content: String,
    },
    Inbox {},
    AcknowledgeMessage {
        sender: AgentId,
        message_id: uuid::Uuid,
    },
}

async fn dispatch(
    state: &Mutex<Service>,
    request: Request<Incoming>,
    slots: &Arc<Semaphore>,
    changed: &Notify,
    upgrade: &mpsc::Sender<subscription::Subscription>,
) -> Result<Reply, Infallible> {
    let result = match request.uri().path() {
        "/v2/operator/events" | "/v2/worker/events" => {
            subscription::open(state, request, slots, upgrade).await
        }
        _ => handle(state, request, changed).await,
    };
    let mut reply = result.unwrap_or_else(HttpError::reply);
    if reply.status() != StatusCode::SWITCHING_PROTOCOLS {
        reply
            .headers_mut()
            .insert(CONNECTION, HeaderValue::from_static("close"));
    }
    Ok(reply)
}

async fn handle(
    state: &Mutex<Service>,
    request: Request<Incoming>,
    changed: &Notify,
) -> Result<Reply, HttpError> {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path();
    let expected_method = match path {
        "/v2/session" => Method::GET,
        "/v2/operator" | "/v2/worker" => Method::POST,
        _ => return Err(HttpError(StatusCode::NOT_FOUND, "unknown_route")),
    };
    if parts.method != expected_method {
        return Err(HttpError(
            StatusCode::METHOD_NOT_ALLOWED,
            if expected_method == Method::GET {
                "expected_get"
            } else {
                "expected_post"
            },
        ));
    }
    if parts.uri.query().is_some() {
        return Err(HttpError(StatusCode::BAD_REQUEST, "unexpected_query"));
    }
    if path == "/v2/session" {
        return Ok(reply(StatusCode::OK, &state.lock().await.auth.session()));
    }
    let proof = Proof::parse(&parts.headers).ok_or(UNAUTHORIZED)?;
    let principal = proof.principal;
    if (path == "/v2/operator") != (principal == Principal::Operator) {
        return Err(UNAUTHORIZED);
    }
    let bytes = read_body(&parts.headers, body).await?;
    let mut service = state.lock().await;
    service.auth.verify(proof, path, &bytes)?;
    let result = service.apply(principal, &bytes);
    changed.notify_waiters();
    result
}

async fn read_body(headers: &HeaderMap, body: Incoming) -> Result<Bytes, HttpError> {
    if headers
        .get(CONTENT_TYPE)
        .is_none_or(|value| value != "application/json")
    {
        return Err(HttpError(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "expected_json",
        ));
    }
    let bytes = Limited::new(body, MAX_BODY)
        .collect()
        .await
        .map_err(|error| {
            if error.is::<LengthLimitError>() {
                HttpError(StatusCode::PAYLOAD_TOO_LARGE, "body_limit")
            } else {
                HttpError(StatusCode::BAD_REQUEST, "invalid_body")
            }
        })?
        .to_bytes();
    Ok(bytes)
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, HttpError> {
    serde_json::from_slice(bytes).map_err(|_| HttpError(StatusCode::BAD_REQUEST, "invalid_json"))
}

fn operate(service: &mut Service, operation: OperatorRequest) -> Result<Reply, HttpError> {
    let broker = &mut service.broker;
    let reply = match operation {
        OperatorRequest::Submit {
            task_id,
            prompt,
            required_capabilities,
            eligible_workers,
            lifetime_seconds,
            max_attempts,
        } => {
            broker.submit(
                Task {
                    id: task_id,
                    prompt,
                    required_capabilities,
                    state: TaskState::Queued,
                },
                eligible_workers,
                Duration::from_secs(lifetime_seconds),
                max_attempts,
            )?;
            reply(
                StatusCode::CREATED,
                &json!({ "task_id": task_id, "state": TaskState::Queued }),
            )
        }
        OperatorRequest::Inspect { task_id } => {
            let state = broker.state(task_id)?;
            reply(
                StatusCode::OK,
                &json!({ "state": state, "result": broker.result(task_id)? }),
            )
        }
        OperatorRequest::Cancel { task_id } => {
            reply(StatusCode::OK, &json!({ "state": broker.cancel(task_id)? }))
        }
        OperatorRequest::Revoke { worker_id } => {
            broker
                .revoke_worker(worker_id)
                .map_err(|_| HttpError(StatusCode::NOT_FOUND, "unknown_worker"))?;
            service.auth.revoke(worker_id);
            service.mailboxes.revoke(worker_id);
            reply(StatusCode::NO_CONTENT, &())
        }
        OperatorRequest::Replay { after, limit } => {
            reply(StatusCode::OK, &broker.replay(None, after, limit)?)
        }
    };
    Ok(reply)
}

fn work(
    broker: &mut LeaseBroker,
    mailboxes: &mut mailbox::Mailboxes,
    worker: AgentId,
    operation: WorkerRequest,
) -> Result<Reply, HttpError> {
    let reply = match operation {
        WorkerRequest::Claim {} => match broker.claim(worker)? {
            Some(lease) => reply(StatusCode::OK, &lease),
            None => reply(StatusCode::NO_CONTENT, &()),
        },
        WorkerRequest::Renew { fence } => reply(
            StatusCode::OK,
            &json!({ "expires_in_ms": broker.renew(worker, fence)? }),
        ),
        WorkerRequest::Complete { fence, content } => {
            reply(StatusCode::OK, &broker.complete(worker, fence, content)?)
        }
        WorkerRequest::Replay { after, limit } => {
            reply(StatusCode::OK, &broker.replay(Some(worker), after, limit)?)
        }
        WorkerRequest::SendMessage {
            recipient,
            message_id,
            content,
        } => {
            if mailboxes.send(worker, recipient, message_id, content)? {
                broker.events.publish_message(worker, recipient);
            }
            reply(StatusCode::NO_CONTENT, &())
        }
        WorkerRequest::Inbox {} => reply(StatusCode::OK, mailboxes.inbox(worker)?),
        WorkerRequest::AcknowledgeMessage { sender, message_id } => {
            mailboxes.acknowledge(worker, sender, message_id)?;
            reply(StatusCode::NO_CONTENT, &())
        }
    };
    Ok(reply)
}

struct HttpError(StatusCode, &'static str);

const UNAUTHORIZED: HttpError = HttpError(StatusCode::UNAUTHORIZED, "unauthorized");
const STORAGE_UNAVAILABLE: HttpError =
    HttpError(StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable");

impl HttpError {
    fn reply(self) -> Reply {
        let mut response = reply(self.0, &json!({ "error": self.1 }));
        if self.0 == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                hyper::header::WWW_AUTHENTICATE,
                hyper::header::HeaderValue::from_static(SCHEME),
            );
        }
        if self.0 == StatusCode::METHOD_NOT_ALLOWED {
            response.headers_mut().insert(
                hyper::header::ALLOW,
                hyper::header::HeaderValue::from_static(if self.1 == "expected_get" {
                    "GET"
                } else {
                    "POST"
                }),
            );
        }
        response
    }
}

impl From<BrokerError> for HttpError {
    fn from(error: BrokerError) -> Self {
        let (status, code) = match error {
            BrokerError::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_input"),
            BrokerError::Capacity => (StatusCode::SERVICE_UNAVAILABLE, "capacity"),
            BrokerError::Unauthorized => return UNAUTHORIZED,
            BrokerError::Busy => (StatusCode::CONFLICT, "worker_busy"),
            BrokerError::Duplicate => (StatusCode::CONFLICT, "duplicate_task"),
            BrokerError::Missing => (StatusCode::NOT_FOUND, "unknown_task"),
            BrokerError::StaleLease => (StatusCode::CONFLICT, "stale_lease"),
            BrokerError::ResultConflict => (StatusCode::CONFLICT, "result_conflict"),
            BrokerError::ForeignCursor => (StatusCode::CONFLICT, "foreign_cursor"),
            BrokerError::ExpiredCursor => (StatusCode::GONE, "cursor_expired"),
        };
        Self(status, code)
    }
}

fn reply(status: StatusCode, value: &impl Serialize) -> Reply {
    let mut builder = Response::builder()
        .status(status)
        .header(CACHE_CONTROL, "no-store");
    let body = if status == StatusCode::NO_CONTENT {
        Bytes::new()
    } else {
        builder = builder.header(CONTENT_TYPE, "application/json");
        Bytes::from(serde_json::to_vec(value).expect("broker response is JSON serializable"))
    };
    builder
        .body(Full::new(body))
        .expect("valid broker response headers")
}
