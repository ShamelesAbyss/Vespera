use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as SlotMutex, Weak};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use http_body_util::Full;
use hyper::body::{Body, Bytes, Incoming};
use hyper::header::{CACHE_CONTROL, HeaderValue};
use hyper::upgrade::{OnUpgrade, Upgraded};
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::sync::{Mutex, Notify, OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::time::{Instant, sleep_until, timeout};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::server::create_response_with_body;
use tokio_tungstenite::tungstenite::protocol::{Message, Role, WebSocketConfig};
use vespera_core::{
    AgentId, BrokerEventCursor, BrokerEventPage, BrokerStreamEnd, BrokerStreamFrame,
};

use super::auth::{Principal, Proof};
use super::{HttpError, Reply, STORAGE_UNAVAILABLE, Service, UNAUTHORIZED};

const LIFETIME: Duration = Duration::from_secs(60);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const CHECK_INTERVAL: Duration = Duration::from_secs(1);
const INVALID_CURSOR: HttpError = HttpError(StatusCode::BAD_REQUEST, "invalid_cursor");
const CAPACITY: HttpError = HttpError(StatusCode::SERVICE_UNAVAILABLE, "subscription_capacity");

type Socket = WebSocketStream<TokioIo<Upgraded>>;

pub(super) struct Slots {
    operator: Arc<Semaphore>,
    workers: Arc<Semaphore>,
    by_worker: SlotMutex<BTreeMap<AgentId, Weak<Semaphore>>>,
}

impl Slots {
    pub(super) fn new() -> Self {
        Self {
            operator: Arc::new(Semaphore::new(2)),
            workers: Arc::new(Semaphore::new(6)),
            by_worker: SlotMutex::default(),
        }
    }

    fn acquire(&self, worker: Option<AgentId>) -> Result<StreamPermit, HttpError> {
        let Some(worker) = worker else {
            return Ok(StreamPermit {
                _pool: Arc::clone(&self.operator)
                    .try_acquire_owned()
                    .map_err(|_| CAPACITY)?,
                _worker: None,
            });
        };
        let pool = Arc::clone(&self.workers)
            .try_acquire_owned()
            .map_err(|_| CAPACITY)?;
        let mut workers = self
            .by_worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        workers.retain(|_, slots| slots.strong_count() > 0);
        let entry = workers.entry(worker).or_default();
        let limit = entry.upgrade().unwrap_or_else(|| {
            let limit = Arc::new(Semaphore::new(2));
            *entry = Arc::downgrade(&limit);
            limit
        });
        Ok(StreamPermit {
            _pool: pool,
            _worker: Some(limit.try_acquire_owned().map_err(|_| CAPACITY)?),
        })
    }
}

struct StreamPermit {
    _pool: OwnedSemaphorePermit,
    _worker: Option<OwnedSemaphorePermit>,
}

pub(super) struct Subscription {
    upgrade: OnUpgrade,
    _slot: StreamPermit,
    worker: Option<AgentId>,
    cursor: BrokerEventCursor,
    discarded_through: u64,
}

pub(super) async fn open(
    state: &Mutex<Service>,
    mut request: Request<Incoming>,
    slots: &Slots,
    sender: &mpsc::Sender<Subscription>,
) -> Result<Reply, HttpError> {
    if request.method() != Method::GET {
        return Err(HttpError(StatusCode::METHOD_NOT_ALLOWED, "expected_get"));
    }
    if request.body().size_hint().exact() != Some(0) {
        return Err(HttpError(StatusCode::BAD_REQUEST, "invalid_body"));
    }
    let mut response = create_response_with_body(&request, || Full::new(Bytes::new()))
        .map_err(|_| HttpError(StatusCode::BAD_REQUEST, "invalid_upgrade"))?;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));

    let uri = request.uri();
    let target = uri.path_and_query().ok_or(INVALID_CURSOR)?.as_str();
    let after = uri.query().map(parse_cursor).transpose()?;
    let proof = Proof::parse(request.headers()).ok_or(UNAUTHORIZED)?;
    let worker = match (uri.path(), proof.principal) {
        ("/v2/operator/events", Principal::Operator) => None,
        ("/v2/worker/events", Principal::Worker(worker)) => Some(worker),
        _ => return Err(UNAUTHORIZED),
    };
    let mut service = state.lock().await;
    service.auth.verify_upgrade(proof, target)?;
    let slot = slots.acquire(worker)?;
    let initial = replay(&mut service, worker, after)?;
    let cursor = after.unwrap_or(BrokerEventCursor {
        stream_id: initial.next_cursor.stream_id,
        sequence: initial.discarded_through,
    });
    sender
        .try_send(Subscription {
            upgrade: hyper::upgrade::on(&mut request),
            _slot: slot,
            worker,
            cursor,
            discarded_through: initial.discarded_through,
        })
        .map_err(|_| CAPACITY)?;
    Ok(response)
}

fn parse_cursor(query: &str) -> Result<BrokerEventCursor, HttpError> {
    let (id, sequence) = query
        .strip_prefix("after=")
        .and_then(|value| value.split_once(':'))
        .ok_or(INVALID_CURSOR)?;
    let stream_id = id.parse::<uuid::Uuid>().map_err(|_| INVALID_CURSOR)?;
    let number = sequence.parse::<u64>().map_err(|_| INVALID_CURSOR)?;
    if id != stream_id.to_string() || sequence != number.to_string() {
        return Err(INVALID_CURSOR);
    }
    Ok(BrokerEventCursor {
        stream_id,
        sequence: number,
    })
}

fn replay(
    service: &mut Service,
    worker: Option<AgentId>,
    after: Option<BrokerEventCursor>,
) -> Result<BrokerEventPage, HttpError> {
    if service.storage_failed {
        return Err(STORAGE_UNAVAILABLE);
    }
    let page = service.broker.replay(worker, after, 1);
    service.persist(None)?;
    Ok(page?)
}

impl Subscription {
    pub(super) async fn run(self, state: &Mutex<Service>, changed: &Notify) {
        let Subscription {
            upgrade,
            _slot,
            worker,
            mut cursor,
            discarded_through,
        } = self;
        let Ok(Ok(io)) = timeout(WRITE_TIMEOUT, upgrade).await else {
            return;
        };
        let config = WebSocketConfig::default()
            .read_buffer_size(4096)
            .write_buffer_size(0)
            .max_write_buffer_size(8192)
            .max_message_size(Some(4096))
            .max_frame_size(Some(4096));
        let mut socket =
            WebSocketStream::from_raw_socket(TokioIo::new(io), Role::Server, Some(config)).await;
        let deadline = Instant::now() + LIFETIME;
        if !send(
            &mut socket,
            &BrokerStreamFrame::Ready {
                after: cursor,
                discarded_through,
            },
        )
        .await
        {
            return;
        }
        let reason = loop {
            if Instant::now() >= deadline {
                break BrokerStreamEnd::Lifetime;
            }
            let notification = changed.notified();
            let page = {
                let mut service = state.lock().await;
                replay(&mut service, worker, Some(cursor))
            };
            match page {
                Ok(page) => {
                    if let Some(entry) = page.entries.into_iter().next() {
                        cursor = entry.cursor;
                        if !send(&mut socket, &BrokerStreamFrame::Event { entry }).await {
                            return;
                        }
                        continue;
                    }
                }
                Err(error) => {
                    break match error.0 {
                        StatusCode::UNAUTHORIZED => BrokerStreamEnd::Unauthorized,
                        StatusCode::GONE => BrokerStreamEnd::CursorExpired,
                        _ => BrokerStreamEnd::StorageUnavailable,
                    };
                }
            }
            tokio::select! {
                () = notification => {}
                () = sleep_until((Instant::now() + CHECK_INTERVAL).min(deadline)) => {}
                message = socket.next() => match message {
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                        if !matches!(timeout(WRITE_TIMEOUT, socket.flush()).await, Ok(Ok(()))) {
                            return;
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        let _ = timeout(WRITE_TIMEOUT, socket.flush()).await;
                        return;
                    }
                    _ => return,
                }
            }
        };
        if send(&mut socket, &BrokerStreamFrame::End { reason }).await {
            let _ = timeout(WRITE_TIMEOUT, socket.close(None)).await;
        }
    }
}

async fn send(socket: &mut Socket, frame: &BrokerStreamFrame) -> bool {
    let text = serde_json::to_string(frame).expect("broker stream frame is serializable");
    matches!(
        timeout(WRITE_TIMEOUT, socket.send(Message::Text(text.into()))).await,
        Ok(Ok(()))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_worker_pool_preserves_operator_slots() {
        let slots = Slots::new();
        let mut permits = Vec::new();
        for worker in [AgentId::new(), AgentId::new(), AgentId::new()] {
            for _ in 0..2 {
                permits.push(
                    slots
                        .acquire(Some(worker))
                        .unwrap_or_else(|_| panic!("worker slot unavailable")),
                );
            }
        }
        let additional = AgentId::new();
        assert!(slots.acquire(Some(additional)).is_err());
        let operator = slots
            .acquire(None)
            .unwrap_or_else(|_| panic!("reserved operator slot unavailable"));
        let second_operator = slots
            .acquire(None)
            .unwrap_or_else(|_| panic!("second operator slot unavailable"));
        assert!(slots.acquire(None).is_err());
        drop(permits.pop());
        assert!(slots.acquire(Some(additional)).is_ok());
        drop(operator);
        assert!(slots.acquire(None).is_ok());
        drop(second_operator);
    }
}
