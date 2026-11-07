//! Bounded HTTP/1 service for an explicitly approved set of local workers.

mod auth;

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
use hyper::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderMap};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use vespera_core::{AgentId, BrokerEventCursor, Capability, Task, TaskId, TaskState};

use crate::{BrokerError, Fence, LeaseBroker, MAX_WORKERS};
use auth::{Authentication, Principal, Proof, SCHEME};

const MAX_ACCESS_FILE: u64 = 128 * 1024;
const MAX_BODY: usize = 512 * 1024;
const MAX_CONNECTIONS: usize = 32;
type Reply = Response<Full<Bytes>>;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("worker HTTP service requires a numeric loopback address")]
    NonLoopback,
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
}

struct Service {
    auth: Authentication,
    broker: LeaseBroker,
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
        }
        Ok(service)
    }
}

/// A bound loopback listener with an in-memory lease broker.
pub struct Server {
    listener: TcpListener,
    address: SocketAddr,
    service: Arc<Mutex<Service>>,
}

impl Server {
    pub async fn bind(
        address: SocketAddr,
        access_file: impl AsRef<Path>,
    ) -> Result<Self, ServerError> {
        if !address.ip().is_loopback() {
            return Err(ServerError::NonLoopback);
        }
        let service = Arc::new(Mutex::new(Service::load(access_file.as_ref())?));
        let listener = TcpListener::bind(address).await?;
        let address = listener.local_addr()?;
        Ok(Self {
            listener,
            address,
            service,
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Serve until the listener fails or the caller drops this future.
    pub async fn serve(self) -> Result<(), ServerError> {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                Some(_) = connections.join_next(), if !connections.is_empty() => {}
                accepted = self.listener.accept(), if connections.len() < MAX_CONNECTIONS => {
                    let (stream, _) = accepted?;
                    let state = Arc::clone(&self.service);
                    connections.spawn(async move {
                        let mut builder = http1::Builder::new();
                        builder
                            .timer(TokioTimer::new())
                            .header_read_timeout(Duration::from_secs(5))
                            .max_buf_size(16 * 1024)
                            .keep_alive(false);
                        let connection = builder.serve_connection(TokioIo::new(stream), service_fn(
                            move |request| dispatch(Arc::clone(&state), request),
                        ));
                        let _ = tokio::time::timeout(Duration::from_secs(15), connection).await;
                    });
                }
            }
        }
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
}

async fn dispatch(
    state: Arc<Mutex<Service>>,
    request: Request<Incoming>,
) -> Result<Reply, Infallible> {
    Ok(handle(&state, request)
        .await
        .unwrap_or_else(HttpError::reply))
}

async fn handle(state: &Mutex<Service>, request: Request<Incoming>) -> Result<Reply, HttpError> {
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
    match principal {
        Principal::Operator => operate(&mut service, decode(&bytes)?),
        Principal::Worker(worker) => work(&mut service.broker, worker, decode(&bytes)?),
    }
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
    };
    Ok(reply)
}

struct HttpError(StatusCode, &'static str);

const UNAUTHORIZED: HttpError = HttpError(StatusCode::UNAUTHORIZED, "unauthorized");

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
