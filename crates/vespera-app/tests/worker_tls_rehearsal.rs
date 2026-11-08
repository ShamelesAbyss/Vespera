//! Opt-in Windows/Linux or independent-host release rehearsal against a private TLS broker.

use std::fs::{self, OpenOptions};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use vespera_sdk::{
    AgentId, BrokerEventCursor, BrokerStreamFrame, ClientError, EventKind, EventSubscription,
    ExternalTaskChange, OperatorClient, SigningKey, Submission, TaskId, TaskState, WorkerClient,
};

#[derive(Deserialize, Serialize)]
struct Checkpoint {
    task: TaskId,
    message: Uuid,
    claim_cursor: BrokerEventCursor,
    completion_cursor: BrokerEventCursor,
}

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} for the isolated rehearsal"))
}

fn key(name: &str) -> SigningKey {
    SigningKey::from_hex(&required(name)).expect("expected a 32-byte fixture signing key")
}

async fn frame(subscription: &mut EventSubscription) -> BrokerStreamFrame {
    tokio::time::timeout(Duration::from_secs(10), subscription.next())
        .await
        .expect("timed out waiting for broker event")
        .unwrap()
        .expect("broker closed the subscription")
}

async fn resumed_completion(
    worker: &WorkerClient,
    after: BrokerEventCursor,
    task: TaskId,
) -> BrokerEventCursor {
    let mut subscription = worker.subscribe(Some(after)).await.unwrap();
    assert!(matches!(
        frame(&mut subscription).await,
        BrokerStreamFrame::Ready { after: ready, .. } if ready == after
    ));
    let BrokerStreamFrame::Event { entry } = frame(&mut subscription).await else {
        panic!("expected retained completion event");
    };
    assert!(matches!(
        entry.event.kind,
        EventKind::ExternalTaskChanged {
            task_id, change: ExternalTaskChange::Completed, state: TaskState::Completed, ..
        } if task_id == task
    ));
    entry.cursor
}

#[tokio::test]
#[ignore = "requires a dedicated non-loopback TLS broker and fixture credentials"]
async fn authenticated_tls_worker_reconnect_and_restart_rehearsal() {
    let endpoint: SocketAddr = required("VESPERA_REHEARSAL_ENDPOINT").parse().unwrap();
    assert!(!endpoint.ip().is_loopback() && !endpoint.ip().is_unspecified());
    let roots = fs::read(required("VESPERA_REHEARSAL_CA")).unwrap();
    let checkpoint = PathBuf::from(required("VESPERA_REHEARSAL_CHECKPOINT"));
    let first: AgentId = required("VESPERA_REHEARSAL_FIRST").parse().unwrap();
    let second: AgentId = required("VESPERA_REHEARSAL_SECOND").parse().unwrap();
    let operator =
        OperatorClient::new_tls(endpoint, key("VESPERA_REHEARSAL_OPERATOR_KEY"), &roots).unwrap();
    let worker =
        WorkerClient::new_tls(endpoint, first, key("VESPERA_REHEARSAL_FIRST_KEY"), &roots).unwrap();
    let other = WorkerClient::new_tls(
        endpoint,
        second,
        key("VESPERA_REHEARSAL_SECOND_KEY"),
        &roots,
    )
    .unwrap();

    if checkpoint.exists() {
        let saved: Checkpoint =
            serde_json::from_reader(fs::File::open(checkpoint).unwrap()).unwrap();
        let inspection = operator.inspect(saved.task).await.unwrap();
        assert_eq!(inspection.state, TaskState::Completed);
        assert!(inspection.result.is_some());
        assert_eq!(
            resumed_completion(&worker, saved.claim_cursor, saved.task).await,
            saved.completion_cursor
        );
        let inbox = other.inbox().await.unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(
            (inbox[0].sender, inbox[0].message_id),
            (first, saved.message)
        );
        other
            .acknowledge_message(first, saved.message)
            .await
            .unwrap();
        worker
            .send_message(second, saved.message, "fixture mailbox message")
            .await
            .unwrap();
        assert!(other.inbox().await.unwrap().is_empty());
        println!(
            "Recovered the completed task, scoped cursor, and pending inbox after broker restart."
        );
        return;
    }

    let wrong_key = SigningKey::from_hex(&format!(
        "{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    ))
    .unwrap();
    let unauthorized = WorkerClient::new_tls(endpoint, first, wrong_key, &roots).unwrap();
    assert!(matches!(
        unauthorized.claim().await,
        Err(ClientError::Rejected { status: 401, .. })
    ));
    let mut subscription = worker.subscribe(None).await.unwrap();
    assert!(matches!(
        frame(&mut subscription).await,
        BrokerStreamFrame::Ready { .. }
    ));
    let task = TaskId::new();
    operator
        .submit(&Submission {
            task_id: task,
            prompt: "Isolated release fixture".into(),
            required_capabilities: ["text-generation".parse().unwrap()].into(),
            eligible_workers: [first].into(),
            lifetime_seconds: 60,
            max_attempts: 2,
        })
        .await
        .unwrap();
    assert!(other.claim().await.unwrap().is_none());
    let lease = worker
        .claim()
        .await
        .unwrap()
        .expect("worker receives its eligible lease");
    assert_eq!(lease.task.id, task);
    assert!(worker.renew(lease.fence).await.unwrap() > 0);
    let BrokerStreamFrame::Event { entry } = frame(&mut subscription).await else {
        panic!("expected the worker's claim event");
    };
    assert!(matches!(entry.event.kind, EventKind::ExternalTaskChanged {
        task_id, change: ExternalTaskChange::Claimed, ..
    } if task_id == task));
    drop(subscription);
    worker
        .complete(lease.fence, "fixture result")
        .await
        .unwrap();
    let completion_cursor = resumed_completion(&worker, entry.cursor, task).await;
    assert!(matches!(
        other.subscribe(Some(entry.cursor)).await,
        Err(ClientError::Rejected { status: 409, .. })
    ));

    let message = Uuid::new_v4();
    assert!(matches!(
        other.send_message(first, message, "denied route").await,
        Err(ClientError::Rejected { status: 401, .. })
    ));
    worker
        .send_message(second, message, "fixture mailbox message")
        .await
        .unwrap();
    let inbox = other.inbox().await.unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!((inbox[0].sender, inbox[0].message_id), (first, message));
    serde_json::to_writer(
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(checkpoint)
            .unwrap(),
        &Checkpoint {
            task,
            message,
            claim_cursor: entry.cursor,
            completion_cursor,
        },
    )
    .unwrap();
    println!(
        "Verified signed TLS leasing, messaging, and WebSocket resume. Restart the broker to verify recovery."
    );
}
