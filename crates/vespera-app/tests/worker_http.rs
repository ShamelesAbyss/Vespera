use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use reqwest::{Client, Response, StatusCode};
use ring::hmac;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use uuid::Uuid;
use vespera_core::{AgentId, TaskId};
use vespera_sdk::{
    BrokerStreamEnd, BrokerStreamFrame, ClientError, ErrorCode, EventKind, EventSubscription,
    ExternalTaskChange, OperatorClient, SigningKey, Submission, TaskState, WorkerClient,
};

const OPERATOR: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const FIRST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SECOND: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

struct Process {
    child: Child,
    policy: PathBuf,
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.policy);
    }
}

struct Wire {
    client: Client,
    base: String,
    first: AgentId,
    second: AgentId,
}

impl Wire {
    async fn sign(&self, role: &str, principal: &str, token: &str, body: &str) -> String {
        self.sign_target("POST", &format!("/v2/{role}"), principal, token, body)
            .await
    }

    async fn sign_target(
        &self,
        method: &str,
        target: &str,
        principal: &str,
        token: &str,
        body: &str,
    ) -> String {
        let session: Value = self
            .client
            .get(format!("{}/v2/session", self.base))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let session_id = session["session_id"].as_str().unwrap();
        let elapsed = session["elapsed_ms"].as_u64().unwrap();
        let nonce = Uuid::new_v4();
        let key: Vec<_> = (0..token.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&token[i..i + 2], 16).unwrap())
            .collect();
        let bytes = format!(
            "Vespera-HMAC-SHA256\n{method}\n{target}\n{principal}\n{session_id}\n{elapsed}\n{nonce}\n{body}"
        );
        let signature = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, &key), bytes.as_bytes());
        let mac = signature
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        format!("Vespera-HMAC-SHA256 {principal}:{session_id}:{elapsed}:{nonce}:{mac}")
    }

    async fn send(&self, role: &str, auth: &str, body: String) -> Response {
        self.client
            .post(format!("{}/v2/{role}", self.base))
            .header("authorization", auth)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .unwrap()
    }

    async fn post(&self, role: &str, token: &str, body: Value) -> Response {
        let principal = match token {
            OPERATOR => "operator".into(),
            FIRST => self.first.to_string(),
            SECOND => self.second.to_string(),
            _ => panic!("unknown fixture credential"),
        };
        let body = body.to_string();
        let auth = self.sign(role, &principal, token, &body).await;
        self.send(role, &auth, body).await
    }
}

fn launch(first: AgentId, second: AgentId) -> (Process, String) {
    launch_with_database(first, second, None)
}

fn server_command(policy: &Path, database: Option<&Path>) -> Command {
    server_command_at(policy, database, "127.0.0.1:0")
}

fn server_command_at(policy: &Path, database: Option<&Path>, listen: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vespera"));
    command
        .args(["serve", "--listen", listen, "--access-file"])
        .arg(policy)
        .env("TEST_OPERATOR_TOKEN", OPERATOR)
        .env("TEST_FIRST_TOKEN", FIRST)
        .env("TEST_SECOND_TOKEN", SECOND);
    if let Some(database) = database {
        command.arg("--broker-db").arg(database);
    }
    command
}

fn launch_with_database(
    first: AgentId,
    second: AgentId,
    database: Option<&Path>,
) -> (Process, String) {
    let policy = create_policy(first, second);
    let command = server_command(&policy, database);
    let (process, base) = launch_command(command, policy);
    assert!(base.starts_with("http://127.0.0.1:"));
    (process, base)
}

fn create_policy(first: AgentId, second: AgentId) -> PathBuf {
    let policy = std::env::temp_dir().join(format!("vespera-worker-{}.json", Uuid::new_v4()));
    std::fs::write(
        &policy,
        serde_json::to_vec(&json!({
            "operator_token_env": "TEST_OPERATOR_TOKEN",
            "workers": [
                {"id": first, "capabilities": ["text-generation"], "token_env": "TEST_FIRST_TOKEN"},
                {"id": second, "capabilities": ["text-generation"], "token_env": "TEST_SECOND_TOKEN"}
            ]
        })).unwrap(),
    ).unwrap();
    policy
}

fn launch_command(mut command: Command, policy: PathBuf) -> (Process, String) {
    let child = command.stdout(Stdio::piped()).spawn().unwrap();
    let mut process = Process { child, policy };
    let stdout = process.child.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(BufReader::new(stdout).lines().next());
    });
    let base = receive
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap()
        .unwrap();
    (process, base)
}

struct TlsFixture {
    certificate: PathBuf,
    private_key: PathBuf,
    roots: String,
}

impl TlsFixture {
    fn new(name: &str) -> Self {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec![name.into()]).unwrap();
        let id = Uuid::new_v4();
        let fixture = Self {
            certificate: std::env::temp_dir().join(format!("vespera-{id}-cert.pem")),
            private_key: std::env::temp_dir().join(format!("vespera-{id}-key.pem")),
            roots: cert.pem(),
        };
        std::fs::write(&fixture.certificate, &fixture.roots).unwrap();
        std::fs::write(&fixture.private_key, signing_key.serialize_pem()).unwrap();
        fixture
    }

    fn configure(&self, command: &mut Command) {
        command
            .arg("--tls-cert")
            .arg(&self.certificate)
            .arg("--tls-key")
            .arg(&self.private_key);
    }
}

impl Drop for TlsFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.certificate);
        let _ = std::fs::remove_file(&self.private_key);
    }
}

fn fresh_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn routed_policy(first: AgentId, second: AgentId) -> PathBuf {
    let policy = create_policy(first, second);
    let mut access: Value = serde_json::from_slice(&std::fs::read(&policy).unwrap()).unwrap();
    access["workers"][0]["message_targets"] = json!([second]);
    std::fs::write(&policy, serde_json::to_vec(&access).unwrap()).unwrap();
    policy
}

#[tokio::test]
async fn broker_limits_idle_connections_from_one_peer_and_reclaims_slots() {
    let (_process, base) = launch(AgentId::new(), AgentId::new());
    let address = base.strip_prefix("http://").unwrap();
    let mut occupied = Vec::new();
    for _ in 0..16 {
        occupied.push(TcpStream::connect(address).await.unwrap());
    }
    let mut excess = TcpStream::connect(address).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), excess.read_u8())
        .await
        .expect("the over-quota connection should close before the header deadline");
    assert!(result.is_err());
    drop(occupied);

    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(reply) = client.get(format!("{base}/v2/session")).send().await
                && reply.status() == StatusCode::OK
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("closing a peer's connections should free its slots");
}

#[tokio::test]
async fn broker_rejects_unknown_workers_and_times_out_stalled_request_bodies() {
    let first = AgentId::new();
    let second = AgentId::new();
    let (_process, base) = launch(first, second);
    let wire = Wire {
        client: Client::new(),
        base,
        first,
        second,
    };
    let address = wire.base.strip_prefix("http://").unwrap();
    for (principal, seconds, expected) in [(AgentId::new(), 2, 401), (first, 8, 408)] {
        let auth = wire
            .sign("worker", &principal.to_string(), FIRST, "{}")
            .await;
        let mut socket = TcpStream::connect(address).await.unwrap();
        let headers = format!(
            "POST /v2/worker HTTP/1.1\r\nHost: {address}\r\nAuthorization: {auth}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n"
        );
        socket.write_all(headers.as_bytes()).await.unwrap();
        let mut reply = Vec::new();
        tokio::time::timeout(Duration::from_secs(seconds), socket.read_to_end(&mut reply))
            .await
            .expect("the broker should respond before the connection deadline")
            .unwrap();
        assert!(reply.starts_with(format!("HTTP/1.1 {expected}").as_bytes()));
    }
}

#[tokio::test]
async fn worker_mailboxes_enforce_direction_ownership_quota_and_revocation() {
    let first = AgentId::new();
    let second = AgentId::new();
    let policy = routed_policy(first, second);
    let db = DatabaseFixture(
        std::env::temp_dir().join(format!("vespera-broker-{}.sqlite3", Uuid::new_v4())),
    );
    let (process, base) = launch_command(server_command(&policy, Some(&db.0)), policy);
    let endpoint = base.strip_prefix("http://").unwrap().parse().unwrap();
    let operator = OperatorClient::new(endpoint, SigningKey::from_hex(OPERATOR).unwrap()).unwrap();
    let sender = WorkerClient::new(endpoint, first, SigningKey::from_hex(FIRST).unwrap()).unwrap();
    let recipient =
        WorkerClient::new(endpoint, second, SigningKey::from_hex(SECOND).unwrap()).unwrap();
    let mut live = operator.subscribe(None).await.unwrap();
    assert!(matches!(
        live_frame(&mut live).await,
        BrokerStreamFrame::Ready { .. }
    ));
    let id = Uuid::new_v4();
    assert!(matches!(
        recipient.send_message(first, id, "denied").await,
        Err(ClientError::Rejected { status: 401, .. })
    ));
    sender.send_message(second, id, "hello").await.unwrap();
    let BrokerStreamFrame::Event { entry } = live_frame(&mut live).await else {
        panic!("expected operator message evidence");
    };
    assert_eq!(
        entry.event.kind,
        EventKind::AgentMessageSent {
            from_agent_id: first,
            to_agent_id: second
        }
    );
    assert_eq!(entry.event.correlation_task_id, None);
    let encoded = serde_json::to_string(&entry).unwrap();
    assert!(!encoded.contains("hello") && !encoded.contains(&id.to_string()));
    sender.send_message(second, id, "hello").await.unwrap();
    assert!(matches!(
        sender.send_message(second, id, "changed").await,
        Err(ClientError::Rejected {
            code: Some(ErrorCode::MessageConflict),
            ..
        })
    ));
    sender.acknowledge_message(first, id).await.unwrap();
    assert!(sender.inbox().await.unwrap().is_empty());
    let inbox = recipient.inbox().await.unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(
        (
            inbox[0].sender,
            inbox[0].message_id,
            inbox[0].content.as_str()
        ),
        (first, id, "hello")
    );
    recipient.acknowledge_message(first, id).await.unwrap();
    sender.send_message(second, id, "hello").await.unwrap();
    assert!(recipient.inbox().await.unwrap().is_empty());
    let page = operator.replay(None, 64).await.unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0], entry);
    assert!(sender.replay(None, 64).await.unwrap().entries.is_empty());
    assert!(recipient.replay(None, 64).await.unwrap().entries.is_empty());
    for _ in 0..32 {
        sender
            .send_message(second, Uuid::new_v4(), "queued")
            .await
            .unwrap();
    }
    assert!(matches!(
        sender
            .send_message(second, Uuid::new_v4(), "overflow")
            .await,
        Err(ClientError::Rejected {
            code: Some(ErrorCode::InboxCapacity),
            ..
        })
    ));
    assert_eq!(recipient.inbox().await.unwrap().len(), 32);
    operator.revoke(first).await.unwrap();
    assert!(recipient.inbox().await.unwrap().is_empty());
    assert!(matches!(
        sender.inbox().await,
        Err(ClientError::Rejected { status: 401, .. })
    ));
    let page = operator.replay(None, 64).await.unwrap();
    assert_eq!(page.entries.len(), 33);
    drop(live);
    drop(process);
    let policy = routed_policy(first, second);
    let (_restarted, base) = launch_command(server_command(&policy, Some(&db.0)), policy);
    let endpoint = base.strip_prefix("http://").unwrap().parse().unwrap();
    let operator = OperatorClient::new(endpoint, SigningKey::from_hex(OPERATOR).unwrap()).unwrap();
    let recovered = operator.replay(None, 64).await.unwrap();
    assert_eq!(recovered.entries, page.entries);
    let recipient =
        WorkerClient::new(endpoint, second, SigningKey::from_hex(SECOND).unwrap()).unwrap();
    let sender = WorkerClient::new(endpoint, first, SigningKey::from_hex(FIRST).unwrap()).unwrap();
    assert!(recipient.inbox().await.unwrap().is_empty());
    assert!(matches!(
        sender.inbox().await,
        Err(ClientError::Rejected { status: 401, .. })
    ));
}

#[tokio::test]
async fn durable_mailbox_recovers_order_and_deduplication_under_current_policy() {
    let first = AgentId::new();
    let second = AgentId::new();
    let db = DatabaseFixture(
        std::env::temp_dir().join(format!("vespera-broker-{}.sqlite3", Uuid::new_v4())),
    );
    let (initial, _) = launch_with_database(first, second, Some(&db.0));
    drop(initial);
    rusqlite::Connection::open(&db.0)
        .unwrap()
        .execute_batch("DROP TABLE broker_mailbox; PRAGMA user_version = 2;")
        .unwrap();
    let start = |grant| {
        let policy = if grant {
            routed_policy(first, second)
        } else {
            create_policy(first, second)
        };
        let mut access: Value = serde_json::from_slice(&std::fs::read(&policy).unwrap()).unwrap();
        access["operator_mailbox_receipts"] = json!(true);
        std::fs::write(&policy, serde_json::to_vec(&access).unwrap()).unwrap();
        let (process, base) = launch_command(server_command(&policy, Some(&db.0)), policy);
        let endpoint = base.strip_prefix("http://").unwrap().parse().unwrap();
        let operator =
            OperatorClient::new(endpoint, SigningKey::from_hex(OPERATOR).unwrap()).unwrap();
        let sender =
            WorkerClient::new(endpoint, first, SigningKey::from_hex(FIRST).unwrap()).unwrap();
        let recipient =
            WorkerClient::new(endpoint, second, SigningKey::from_hex(SECOND).unwrap()).unwrap();
        (process, operator, sender, recipient)
    };
    let acknowledged = Uuid::new_v4();
    let pending = [Uuid::new_v4(), Uuid::new_v4()];
    let (process, operator, sender, recipient) = start(true);
    let mut live = operator.subscribe(None).await.unwrap();
    assert!(matches!(
        live_frame(&mut live).await,
        BrokerStreamFrame::Ready { .. }
    ));
    sender
        .send_message(second, acknowledged, "private acknowledged content")
        .await
        .unwrap();
    sender
        .acknowledge_message(first, acknowledged)
        .await
        .unwrap();
    let (one, two) = tokio::join!(
        recipient.acknowledge_message(first, acknowledged),
        recipient.acknowledge_message(first, acknowledged)
    );
    one.unwrap();
    two.unwrap();
    assert!(
        matches!(live_frame(&mut live).await, BrokerStreamFrame::Event { entry } if matches!(entry.event.kind, EventKind::AgentMessageSent { .. }))
    );
    let BrokerStreamFrame::Event { entry: receipt } = live_frame(&mut live).await else {
        panic!("expected acknowledgement on the operator stream");
    };
    assert_eq!(receipt.event.schema_version, 8);
    assert_eq!(
        receipt.event.kind,
        EventKind::AgentMessageAcknowledged {
            from_agent_id: first,
            to_agent_id: second
        }
    );
    let encoded = serde_json::to_string(&receipt).unwrap();
    assert!(
        !encoded.contains(&acknowledged.to_string())
            && !encoded.contains("private acknowledged content")
    );
    assert!(sender.replay(None, 64).await.unwrap().entries.is_empty());
    assert!(recipient.replay(None, 64).await.unwrap().entries.is_empty());
    drop(live);
    for id in pending {
        sender.send_message(second, id, "pending").await.unwrap();
    }
    let before = operator.replay(None, 64).await.unwrap();
    assert_eq!(before.entries.len(), 4);
    assert_eq!(before.entries[1], receipt);
    drop(process);

    let (process, operator, sender, recipient) = start(true);
    let recovered = recipient.inbox().await.unwrap();
    assert_eq!(
        recovered
            .iter()
            .map(|entry| entry.message_id)
            .collect::<Vec<_>>(),
        pending
    );
    assert!(
        recovered
            .iter()
            .all(|entry| entry.sender == first && entry.content == "pending")
    );
    sender
        .send_message(second, acknowledged, "private acknowledged content")
        .await
        .unwrap();
    sender
        .send_message(second, pending[0], "pending")
        .await
        .unwrap();
    assert!(matches!(
        sender.send_message(second, acknowledged, "changed").await,
        Err(ClientError::Rejected {
            code: Some(ErrorCode::MessageConflict),
            ..
        })
    ));
    assert_eq!(recipient.inbox().await.unwrap().len(), 2);
    recipient
        .acknowledge_message(first, acknowledged)
        .await
        .unwrap();
    assert_eq!(
        operator.replay(None, 64).await.unwrap().entries,
        before.entries
    );
    drop(process);

    let (process, _, _, recipient) = start(false);
    assert!(recipient.inbox().await.unwrap().is_empty());
    drop(process);
    let (process, operator, sender, recipient) = start(true);
    assert!(recipient.inbox().await.unwrap().is_empty());
    sender
        .send_message(second, acknowledged, "new after regrant")
        .await
        .unwrap();
    assert_eq!(recipient.inbox().await.unwrap().len(), 1);
    let committed = operator.replay(None, 64).await.unwrap().entries;
    drop(process);

    rusqlite::Connection::open(&db.0)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_mailbox_update BEFORE UPDATE ON broker_mailbox
             BEGIN SELECT RAISE(ABORT, 'injected mailbox write failure'); END;",
        )
        .unwrap();
    let (process, _, sender, recipient) = start(true);
    assert!(matches!(
        sender
            .send_message(second, Uuid::new_v4(), "must roll back")
            .await,
        Err(ClientError::Rejected {
            code: Some(ErrorCode::StorageUnavailable),
            ..
        })
    ));
    assert!(matches!(
        recipient.inbox().await,
        Err(ClientError::Rejected {
            code: Some(ErrorCode::StorageUnavailable),
            ..
        })
    ));
    drop(process);

    let (process, _, _, recipient) = start(true);
    assert!(matches!(
        recipient.acknowledge_message(first, acknowledged).await,
        Err(ClientError::Rejected {
            code: Some(ErrorCode::StorageUnavailable),
            ..
        })
    ));
    drop(process);

    let (_process, operator, _, recipient) = start(true);
    let inbox = recipient.inbox().await.unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].message_id, acknowledged);
    assert_eq!(operator.replay(None, 64).await.unwrap().entries, committed);
}

#[tokio::test]
async fn tls_broker_requires_opt_in_and_verifies_trust_for_http_and_websocket() {
    let first = AgentId::new();
    let second = AgentId::new();
    let identity = TlsFixture::new("127.0.0.1");
    let wrong_name = TlsFixture::new("localhost");
    let policy = create_policy(first, second);
    let mut refused = server_command_at(&policy, None, "0.0.0.0:0");
    identity.configure(&mut refused);
    let rejected = refused.output().unwrap();
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("explicit TLS remote access"));

    let operator_key = fresh_token();
    let worker_key = fresh_token();
    let mut command = server_command(&policy, None);
    identity.configure(&mut command);
    command
        .env("TEST_OPERATOR_TOKEN", &operator_key)
        .env("TEST_FIRST_TOKEN", &worker_key)
        .env("TEST_SECOND_TOKEN", fresh_token());
    let (_process, base) = launch_command(command, policy);
    let endpoint = base.strip_prefix("https://").unwrap().parse().unwrap();
    let operator = OperatorClient::new_tls(
        endpoint,
        SigningKey::from_hex(&operator_key).unwrap(),
        identity.roots.as_bytes(),
    )
    .unwrap();
    let worker = WorkerClient::new_tls(
        endpoint,
        first,
        SigningKey::from_hex(&worker_key).unwrap(),
        identity.roots.as_bytes(),
    )
    .unwrap();
    let untrusted = WorkerClient::new_tls(
        endpoint,
        first,
        SigningKey::from_hex(&worker_key).unwrap(),
        wrong_name.roots.as_bytes(),
    )
    .unwrap();
    let plaintext =
        WorkerClient::new(endpoint, first, SigningKey::from_hex(&worker_key).unwrap()).unwrap();
    assert!(matches!(
        untrusted.claim().await,
        Err(ClientError::Transport)
    ));
    assert!(matches!(
        plaintext.claim().await,
        Err(ClientError::Transport)
    ));
    assert!(matches!(
        WorkerClient::new(
            "192.0.2.1:8765".parse().unwrap(),
            first,
            SigningKey::from_hex(&worker_key).unwrap()
        ),
        Err(ClientError::InvalidEndpoint)
    ));
    let mut socket = worker.subscribe(None).await.unwrap();
    assert!(matches!(
        live_frame(&mut socket).await,
        BrokerStreamFrame::Ready { .. }
    ));
    let task = TaskId::new();
    operator
        .submit(&Submission {
            task_id: task,
            prompt: "TLS fixture".into(),
            required_capabilities: ["text-generation".parse().unwrap()].into(),
            eligible_workers: [first].into(),
            lifetime_seconds: 60,
            max_attempts: 2,
        })
        .await
        .unwrap();
    assert_eq!(worker.claim().await.unwrap().unwrap().task.id, task);
    assert!(
        matches!(live_frame(&mut socket).await, BrokerStreamFrame::Event { entry }
        if entry.event.correlation_task_id == Some(task))
    );

    let wrong_policy = create_policy(first, second);
    let mut wrong_command = server_command(&wrong_policy, None);
    wrong_name.configure(&mut wrong_command);
    let (_wrong_process, base) = launch_command(wrong_command, wrong_policy);
    let wrong_endpoint = base.strip_prefix("https://").unwrap().parse().unwrap();
    let mismatched = WorkerClient::new_tls(
        wrong_endpoint,
        first,
        SigningKey::from_hex(FIRST).unwrap(),
        wrong_name.roots.as_bytes(),
    )
    .unwrap();
    assert!(matches!(
        mismatched.claim().await,
        Err(ClientError::Transport)
    ));
}

struct DatabaseFixture(PathBuf);

impl Drop for DatabaseFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[tokio::test]
async fn durable_service_recovers_work_and_fences_interrupted_attempts() {
    let first = AgentId::new();
    let second = AgentId::new();
    let db = DatabaseFixture(
        std::env::temp_dir().join(format!("vespera-broker-{}.sqlite3", Uuid::new_v4())),
    );
    let (process, base) = launch_with_database(first, second, Some(&db.0));
    let second_host = server_command(&process.policy, Some(&db.0))
        .output()
        .unwrap();
    assert!(!second_host.status.success());
    assert!(second_host.stdout.is_empty());
    assert!(String::from_utf8_lossy(&second_host.stderr).contains("broker database is busy"));

    let endpoint = base.strip_prefix("http://").unwrap().parse().unwrap();
    let operator = OperatorClient::new(endpoint, SigningKey::from_hex(OPERATOR).unwrap()).unwrap();
    let worker = WorkerClient::new(endpoint, first, SigningKey::from_hex(FIRST).unwrap()).unwrap();
    let completed = TaskId::new();
    let interrupted = TaskId::new();
    let queued = TaskId::new();
    let exhausted = TaskId::new();
    let submission = |task_id, max_attempts| Submission {
        task_id,
        prompt: "Durable worker task".into(),
        required_capabilities: ["text-generation".parse().unwrap()].into(),
        eligible_workers: [first, second].into(),
        lifetime_seconds: 60,
        max_attempts,
    };
    operator.submit(&submission(completed, 2)).await.unwrap();
    let complete_fence = worker.claim().await.unwrap().unwrap().fence;
    let accepted = worker
        .complete(complete_fence, "saved outcome")
        .await
        .unwrap();
    operator.submit(&submission(interrupted, 2)).await.unwrap();
    let old_fence = worker.claim().await.unwrap().unwrap().fence;
    operator.submit(&submission(exhausted, 1)).await.unwrap();
    let second_worker =
        WorkerClient::new(endpoint, second, SigningKey::from_hex(SECOND).unwrap()).unwrap();
    let exhausted_fence = second_worker.claim().await.unwrap().unwrap().fence;
    assert_eq!(exhausted_fence.task_id, exhausted);
    operator.revoke(second).await.unwrap();
    operator
        .submit(&Submission {
            eligible_workers: [first].into(),
            ..submission(queued, 2)
        })
        .await
        .unwrap();
    let previous_cursor = operator.replay(None, 64).await.unwrap().next_cursor;
    let previous_worker = worker.replay(None, 64).await.unwrap();
    assert_eq!(previous_worker.entries.len(), 3);
    drop(process); // Kill the service without graceful shutdown.

    let (_restarted, base) = launch_with_database(first, second, Some(&db.0));
    let endpoint = base.strip_prefix("http://").unwrap().parse().unwrap();
    let operator = OperatorClient::new(endpoint, SigningKey::from_hex(OPERATOR).unwrap()).unwrap();
    let worker = WorkerClient::new(endpoint, first, SigningKey::from_hex(FIRST).unwrap()).unwrap();
    let revoked =
        WorkerClient::new(endpoint, second, SigningKey::from_hex(SECOND).unwrap()).unwrap();
    assert_eq!(
        operator.inspect(completed).await.unwrap().result.unwrap(),
        accepted
    );
    assert_eq!(
        worker
            .complete(complete_fence, "saved outcome")
            .await
            .unwrap(),
        accepted
    );
    assert_eq!(
        operator.inspect(exhausted).await.unwrap().state,
        TaskState::Failed
    );
    assert_eq!(
        operator.inspect(interrupted).await.unwrap().state,
        TaskState::Queued
    );
    assert!(matches!(
        revoked.claim().await,
        Err(ClientError::Rejected { status: 401, .. })
    ));
    assert!(matches!(
        worker.renew(old_fence).await,
        Err(ClientError::Rejected {
            code: Some(ErrorCode::StaleLease),
            ..
        })
    ));
    assert!(matches!(
        revoked.replay(Some(previous_worker.next_cursor), 1).await,
        Err(ClientError::Rejected { status: 401, .. })
    ));
    assert!(matches!(
        worker.replay(Some(previous_cursor), 1).await,
        Err(ClientError::Rejected {
            code: Some(ErrorCode::ForeignCursor),
            ..
        })
    ));
    let operator_recovery = operator.replay(Some(previous_cursor), 1).await.unwrap();
    let recovery = worker
        .replay(Some(previous_worker.next_cursor), 1)
        .await
        .unwrap();
    assert_eq!(
        recovery.entries[0].event,
        operator_recovery.entries[0].event
    );
    assert_eq!(
        recovery.next_cursor.sequence,
        previous_worker.next_cursor.sequence + 1
    );
    assert_eq!(
        operator_recovery.next_cursor.sequence,
        previous_cursor.sequence + 1
    );
    let retained = worker.replay(None, 64).await.unwrap();
    assert_eq!(&retained.entries[..3], previous_worker.entries.as_slice());
    assert_eq!(retained.entries.len(), 4);
    assert!(
        matches!(recovery.entries[0].event.kind, EventKind::ExternalTaskChanged {
        change: ExternalTaskChange::LeaseExpired, attempt: Some(attempt), ..
    } if attempt.attempt_id == old_fence.attempt_id)
    );
    let replacement = worker.claim().await.unwrap().unwrap();
    assert_eq!(replacement.fence.task_id, interrupted);
    assert_eq!(replacement.fence.generation, old_fence.generation + 1);
    assert_ne!(replacement.fence.attempt_id, old_fence.attempt_id);
    worker
        .complete(replacement.fence, "recovered outcome")
        .await
        .unwrap();
    assert_eq!(worker.claim().await.unwrap().unwrap().fence.task_id, queued);
}

#[tokio::test]
async fn serves_fenced_work_to_authenticated_workers_only() {
    let first = AgentId::new();
    let second = AgentId::new();
    let task = TaskId::new();
    let (_process, base) = launch(first, second);
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let wire = Wire {
        client,
        base,
        first,
        second,
    };
    let submit = json!({
        "action": "submit", "task_id": task, "prompt": "Run one bounded task",
        "required_capabilities": ["text-generation"], "eligible_workers": [first, second],
        "lifetime_seconds": 60, "max_attempts": 2
    });
    let claim = json!({"action": "claim"});
    let claim_body = claim.to_string();
    let proof = wire
        .sign("worker", &first.to_string(), FIRST, &claim_body)
        .await;
    let tampered = wire.send("worker", &proof, format!("{claim_body} ")).await;
    assert_eq!(tampered.status(), StatusCode::UNAUTHORIZED);
    let (a, b) = tokio::join!(
        wire.send("worker", &proof, claim_body.clone()),
        wire.send("worker", &proof, claim_body),
    );
    let mut statuses = [a.status().as_u16(), b.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [204, 401]);
    let impersonation = wire
        .sign("worker", &second.to_string(), FIRST, &claim.to_string())
        .await;
    assert_eq!(
        wire.send("worker", &impersonation, claim.to_string())
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let old_client = wire
        .client
        .post(format!("{}/v1/worker", wire.base))
        .bearer_auth(FIRST)
        .json(&claim)
        .send()
        .await
        .unwrap();
    assert_eq!(old_client.status(), StatusCode::NOT_FOUND);
    let bearer = wire
        .send("worker", &format!("Bearer {FIRST}"), claim.to_string())
        .await;
    assert_eq!(bearer.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        wire.post("operator", FIRST, submit.clone()).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        wire.post("worker", OPERATOR, claim.clone()).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        wire.post("worker", FIRST, claim.clone()).await.status(),
        StatusCode::NO_CONTENT
    );
    let spoof = wire
        .post(
            "worker",
            FIRST,
            json!({"action": "claim", "worker_id": second}),
        )
        .await;
    assert_eq!(spoof.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        spoof.json::<Value>().await.unwrap(),
        json!({"error": "invalid_json"})
    );
    let oversized_body = " ".repeat(512 * 1024 + 1);
    let auth = wire
        .sign("worker", &first.to_string(), FIRST, &oversized_body)
        .await;
    let oversized = wire.send("worker", &auth, oversized_body).await;
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        wire.post("operator", OPERATOR, submit.clone())
            .await
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        wire.post("operator", OPERATOR, submit).await.status(),
        StatusCode::CONFLICT
    );
    let lease = wire
        .post("worker", FIRST, claim.clone())
        .await
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(lease["task"]["id"], json!(task));
    assert_eq!(lease["fence"]["generation"], 1);
    let renew = json!({"action": "renew", "fence": lease["fence"]});
    assert_eq!(
        wire.post("worker", SECOND, renew.clone()).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        wire.post("worker", FIRST, renew).await.status(),
        StatusCode::OK
    );
    let revoke = json!({"action": "revoke", "worker_id": first});
    assert_eq!(
        wire.post("operator", OPERATOR, revoke).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        wire.post("worker", FIRST, claim.clone()).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let reassigned = wire
        .post("worker", SECOND, claim)
        .await
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(reassigned["fence"]["generation"], 2);
    let stale = json!({"action": "complete", "fence": lease["fence"], "content": "stale"});
    assert_eq!(
        wire.post("worker", SECOND, stale).await.status(),
        StatusCode::CONFLICT
    );
    let complete =
        json!({"action": "complete", "fence": reassigned["fence"], "content": "accepted"});
    let accepted = wire
        .post("worker", SECOND, complete.clone())
        .await
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let retried = wire
        .post("worker", SECOND, complete)
        .await
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(accepted, retried);
    assert_eq!(accepted["worker"], json!(second));
    let cancel = wire
        .post(
            "operator",
            OPERATOR,
            json!({"action": "cancel", "task_id": task}),
        )
        .await;
    assert_eq!(cancel.json::<Value>().await.unwrap()["state"], "completed");
    let inspected = wire
        .post(
            "operator",
            OPERATOR,
            json!({"action": "inspect", "task_id": task}),
        )
        .await;
    assert_eq!(
        inspected.json::<Value>().await.unwrap(),
        json!({"state": "completed", "result": accepted})
    );
}

#[tokio::test]
async fn sdk_executes_and_reconciles_a_task_against_the_cli_service() {
    let first = AgentId::new();
    let second = AgentId::new();
    let (_process, base) = launch(first, second);
    let endpoint = base.strip_prefix("http://").unwrap().parse().unwrap();
    let operator = OperatorClient::new(endpoint, SigningKey::from_hex(OPERATOR).unwrap()).unwrap();
    let worker = WorkerClient::new(endpoint, first, SigningKey::from_hex(FIRST).unwrap()).unwrap();
    let other = WorkerClient::new(endpoint, second, SigningKey::from_hex(SECOND).unwrap()).unwrap();
    assert!(worker.claim().await.unwrap().is_none());
    let task = TaskId::new();
    let submission = Submission {
        task_id: task,
        prompt: "Summarize: λ".into(),
        required_capabilities: ["text-generation".parse().unwrap()].into(),
        eligible_workers: [first].into(),
        lifetime_seconds: 30,
        max_attempts: 2,
    };
    let queued = operator.submit(&submission).await.unwrap();
    assert_eq!((queued.task_id, queued.state), (task, TaskState::Queued));
    assert!(other.claim().await.unwrap().is_none());
    let lease = worker.claim().await.unwrap().unwrap();
    assert_eq!(lease.task.prompt, submission.prompt);
    assert_eq!(lease.task.id, task);
    let page = worker.replay(None, 1).await.unwrap();
    let claim = &page.entries[0].event;
    assert_eq!(claim.schema_version, 3);
    assert!(matches!(
        claim.kind,
        EventKind::ExternalTaskChanged {
            change: ExternalTaskChange::Claimed, attempt: Some(attempt), ..
        } if attempt.worker_id == first && attempt.attempt_id == lease.fence.attempt_id
    ));
    assert!(other.replay(None, 1).await.unwrap().entries.is_empty());
    assert!(matches!(
        other.replay(Some(page.next_cursor), 1).await,
        Err(ClientError::Rejected {
            status: 409,
            code: Some(ErrorCode::ForeignCursor)
        })
    ));
    assert!(worker.renew(lease.fence).await.unwrap() > 0);
    assert!(matches!(
        other.renew(lease.fence).await,
        Err(ClientError::Rejected {
            status: 409,
            code: Some(ErrorCode::StaleLease)
        })
    ));
    let result = worker.complete(lease.fence, "Result: λ").await.unwrap();
    assert_eq!(
        worker.complete(lease.fence, "Result: λ").await.unwrap(),
        result
    );
    let reconnected =
        WorkerClient::new(endpoint, first, SigningKey::from_hex(FIRST).unwrap()).unwrap();
    let next = reconnected
        .replay(Some(page.next_cursor), 64)
        .await
        .unwrap();
    assert_eq!(next.entries.len(), 1);
    assert!(!next.has_more);
    assert!(matches!(
        next.entries[0].event.kind,
        EventKind::ExternalTaskChanged {
            change: ExternalTaskChange::Completed,
            state: TaskState::Completed,
            ..
        }
    ));
    let serialized = serde_json::to_string(&next).unwrap();
    assert!(!serialized.contains("Result:"));
    assert!(!serialized.contains("Summarize:"));
    let inspected = operator.inspect(task).await.unwrap();
    assert_eq!(inspected.state, TaskState::Completed);
    assert_eq!(inspected.result.unwrap(), result);
    let pending = TaskId::new();
    operator
        .submit(&Submission {
            task_id: pending,
            ..submission
        })
        .await
        .unwrap();
    assert_eq!(
        operator.cancel(pending).await.unwrap(),
        TaskState::Cancelled
    );
    let page = operator.replay(None, 64).await.unwrap();
    assert_eq!(page.entries.len(), 5);
    assert!(
        matches!(page.entries[4].event.kind, EventKind::ExternalTaskChanged {
        change: ExternalTaskChange::Cancelled, task_id, attempt: None, ..
    } if task_id == pending)
    );
    assert!(
        reconnected
            .replay(Some(next.next_cursor), 64)
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    operator.revoke(first).await.unwrap();
    assert!(matches!(
        reconnected.replay(Some(next.next_cursor), 64).await,
        Err(ClientError::Rejected {
            status: 401,
            code: Some(ErrorCode::Unauthorized)
        })
    ));
}

#[test]
fn serve_rejects_non_loopback_before_loading_policy() {
    let output = Command::new(env!("CARGO_BIN_EXE_vespera"))
        .args([
            "serve",
            "--listen",
            "0.0.0.0:0",
            "--access-file",
            "missing.json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("requires a numeric loopback address")
    );
}

async fn live_frame(stream: &mut EventSubscription) -> BrokerStreamFrame {
    tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn cancellation_cannot_cross_broker_sessions_with_reused_task_ids() {
    let first = AgentId::new();
    let second = AgentId::new();
    let (process, base) = launch(first, second);
    let address = base.strip_prefix("http://").unwrap();
    let operator = OperatorClient::new(
        address.parse().unwrap(),
        SigningKey::from_hex(OPERATOR).unwrap(),
    )
    .unwrap();
    let mut stream = operator.subscribe(None).await.unwrap();
    assert!(matches!(
        live_frame(&mut stream).await,
        BrokerStreamFrame::Ready { .. }
    ));
    let old_session = stream.session_id();
    let submission = Submission {
        task_id: TaskId::new(),
        prompt: "Session-bound cancellation fixture".into(),
        required_capabilities: ["text-generation".parse().unwrap()].into(),
        eligible_workers: [first].into(),
        lifetime_seconds: 60,
        max_attempts: 1,
    };
    operator.submit(&submission).await.unwrap();
    assert!(matches!(
        live_frame(&mut stream).await,
        BrokerStreamFrame::Event { .. }
    ));
    drop(stream);
    drop(process);
    let policy = create_policy(first, second);
    let (_replacement, replacement_base) =
        launch_command(server_command_at(&policy, None, address), policy);
    assert_eq!(base, replacement_base);
    operator.submit(&submission).await.unwrap();
    assert!(matches!(
        operator
            .cancel_in_session(submission.task_id, old_session)
            .await,
        Err(ClientError::SessionChanged)
    ));
    assert_eq!(
        operator.inspect(submission.task_id).await.unwrap().state,
        TaskState::Queued
    );
    let mut current = operator.subscribe(None).await.unwrap();
    assert!(matches!(
        live_frame(&mut current).await,
        BrokerStreamFrame::Ready { .. }
    ));
    assert_ne!(current.session_id(), old_session);
    assert_eq!(
        operator
            .cancel_in_session(submission.task_id, current.session_id())
            .await
            .unwrap(),
        TaskState::Cancelled
    );
}

#[tokio::test]
async fn resumable_worker_subscription_uses_only_acknowledged_cursors_after_tls_restart() {
    let first = AgentId::new();
    let second = AgentId::new();
    let identity = TlsFixture::new("127.0.0.1");
    let db = DatabaseFixture(
        std::env::temp_dir().join(format!("vespera-broker-{}.sqlite3", Uuid::new_v4())),
    );
    let operator_key = fresh_token();
    let first_key = fresh_token();
    let second_key = fresh_token();
    let start = |listen: &str| {
        let policy = create_policy(first, second);
        let mut command = server_command_at(&policy, Some(&db.0), listen);
        identity.configure(&mut command);
        command
            .env("TEST_OPERATOR_TOKEN", &operator_key)
            .env("TEST_FIRST_TOKEN", &first_key)
            .env("TEST_SECOND_TOKEN", &second_key);
        launch_command(command, policy)
    };
    let (process, base) = start("127.0.0.1:0");
    let address = base.strip_prefix("https://").unwrap();
    let endpoint = address.parse().unwrap();
    let operator = OperatorClient::new_tls(
        endpoint,
        SigningKey::from_hex(&operator_key).unwrap(),
        identity.roots.as_bytes(),
    )
    .unwrap();
    let worker = WorkerClient::new_tls(
        endpoint,
        first,
        SigningKey::from_hex(&first_key).unwrap(),
        identity.roots.as_bytes(),
    )
    .unwrap();
    let other = WorkerClient::new_tls(
        endpoint,
        second,
        SigningKey::from_hex(&second_key).unwrap(),
        identity.roots.as_bytes(),
    )
    .unwrap();
    let submission = || Submission {
        task_id: TaskId::new(),
        prompt: "Resumable subscription fixture".into(),
        required_capabilities: ["text-generation".parse().unwrap()].into(),
        eligible_workers: [first].into(),
        lifetime_seconds: 60,
        max_attempts: 1,
    };
    let mut stream = worker.subscribe_resumable(None);
    tokio::time::timeout(Duration::from_secs(10), async {
        let Some(BrokerStreamFrame::Ready {
            after: baseline, ..
        }) = stream.next().await.unwrap()
        else {
            panic!("missing initial baseline");
        };
        operator.submit(&submission()).await.unwrap();
        let lease = worker.claim().await.unwrap().unwrap();
        let Some(BrokerStreamFrame::Event { entry: claim }) = stream.next().await.unwrap() else {
            panic!("missing claim event");
        };
        assert_eq!(stream.cursor(), Some(baseline));
        assert!(!stream.acknowledge(baseline));
        assert!(matches!(
            stream.next().await.unwrap(),
            Some(BrokerStreamFrame::Event { entry }) if entry == claim
        ));
        assert!(stream.acknowledge(claim.cursor));
        worker.complete(lease.fence, "done").await.unwrap();
        let Some(BrokerStreamFrame::Event { entry: completed }) = stream.next().await.unwrap()
        else {
            panic!("missing completion event");
        };
        assert!(stream.acknowledge(completed.cursor));

        drop(process);
        let (_restarted, restarted_base) = start(address);
        assert_eq!(base, restarted_base);
        operator.submit(&submission()).await.unwrap();
        let next_lease = worker.claim().await.unwrap().unwrap();
        assert!(matches!(
            stream.next().await.unwrap(),
            Some(BrokerStreamFrame::Ready { after, .. }) if after == completed.cursor
        ));
        let Some(BrokerStreamFrame::Event { entry: resumed }) = stream.next().await.unwrap() else {
            panic!("missing claim after restart");
        };
        assert_eq!(resumed.cursor.sequence, completed.cursor.sequence + 1);
        assert!(matches!(
            resumed.event.kind,
            EventKind::ExternalTaskChanged { task_id, change: ExternalTaskChange::Claimed, .. }
                if task_id == next_lease.fence.task_id
        ));
        assert!(stream.acknowledge(resumed.cursor));
        let mut foreign = other.subscribe_resumable(stream.cursor());
        assert!(matches!(
            foreign.next().await,
            Err(ClientError::Rejected { status: 409, .. })
        ));
        assert!(foreign.next().await.unwrap().is_none());
        operator.revoke(first).await.unwrap();
        assert!(matches!(
            stream.next().await.unwrap(),
            Some(BrokerStreamFrame::End {
                reason: BrokerStreamEnd::Unauthorized
            })
        ));
        assert!(stream.next().await.unwrap().is_none());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn websocket_delivers_scoped_events_resumes_and_observes_revocation() {
    let first = AgentId::new();
    let second = AgentId::new();
    let (_process, base) = launch(first, second);
    let endpoint = base.strip_prefix("http://").unwrap().parse().unwrap();
    let operator = OperatorClient::new(endpoint, SigningKey::from_hex(OPERATOR).unwrap()).unwrap();
    let worker = WorkerClient::new(endpoint, first, SigningKey::from_hex(FIRST).unwrap()).unwrap();
    let other = WorkerClient::new(endpoint, second, SigningKey::from_hex(SECOND).unwrap()).unwrap();
    let wire = Wire {
        client: Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        base,
        first,
        second,
    };
    let cursor = worker.replay(None, 1).await.unwrap().next_cursor;
    let target = format!(
        "/v2/worker/events?after={}:{}",
        cursor.stream_id, cursor.sequence
    );
    let proof = wire
        .sign_target("GET", &target, &first.to_string(), FIRST, "")
        .await;
    let upgrade = |path: &str| {
        wire.client
            .get(format!("{}{path}", wire.base))
            .header("connection", "Upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header("authorization", &proof)
    };
    assert_eq!(
        upgrade("/v2/worker/events").send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        upgrade(&target).send().await.unwrap().status(),
        StatusCode::SWITCHING_PROTOCOLS
    );
    assert_eq!(
        upgrade(&target).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let mut live = worker.subscribe(None).await.unwrap();
    let mut isolated = other.subscribe(None).await.unwrap();
    let mut overview = operator.subscribe(None).await.unwrap();
    assert!(
        matches!(live_frame(&mut live).await, BrokerStreamFrame::Ready {
        after, discarded_through: 0
    } if after.sequence == 0)
    );
    assert!(matches!(
        live_frame(&mut isolated).await,
        BrokerStreamFrame::Ready { .. }
    ));
    assert!(matches!(
        live_frame(&mut overview).await,
        BrokerStreamFrame::Ready { .. }
    ));
    let task_id = TaskId::new();
    operator
        .submit(&Submission {
            task_id,
            prompt: "Live subscription fixture".into(),
            required_capabilities: ["text-generation".parse().unwrap()].into(),
            eligible_workers: [first].into(),
            lifetime_seconds: 60,
            max_attempts: 2,
        })
        .await
        .unwrap();
    let lease = worker.claim().await.unwrap().unwrap();
    assert!(
        matches!(live_frame(&mut overview).await, BrokerStreamFrame::Event { entry }
        if matches!(entry.event.kind, EventKind::ExternalTaskChanged {
            change: ExternalTaskChange::Submitted, task_id: submitted, ..
        } if submitted == task_id))
    );
    let BrokerStreamFrame::Event { entry } = live_frame(&mut live).await else {
        panic!("missing live claim event");
    };
    assert!(matches!(entry.event.kind, EventKind::ExternalTaskChanged {
        change: ExternalTaskChange::Claimed, attempt: Some(attempt), ..
    } if attempt.attempt_id == lease.fence.attempt_id));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), isolated.next())
            .await
            .is_err()
    );
    drop(live);
    worker.complete(lease.fence, "result").await.unwrap();
    let mut resumed = worker.subscribe(Some(entry.cursor)).await.unwrap();
    assert!(
        matches!(live_frame(&mut resumed).await, BrokerStreamFrame::Ready {
        after, ..
    } if after == entry.cursor)
    );
    let BrokerStreamFrame::Event { entry: completion } = live_frame(&mut resumed).await else {
        panic!("missing resumed completion event");
    };
    assert!(matches!(
        completion.event.kind,
        EventKind::ExternalTaskChanged {
            change: ExternalTaskChange::Completed,
            ..
        }
    ));
    assert_eq!(completion.cursor.sequence, entry.cursor.sequence + 1);
    assert!(matches!(
        other.subscribe(Some(entry.cursor)).await,
        Err(ClientError::Rejected { status: 409, .. })
    ));
    operator.revoke(first).await.unwrap();
    assert!(matches!(
        live_frame(&mut resumed).await,
        BrokerStreamFrame::End {
            reason: BrokerStreamEnd::Unauthorized
        }
    ));
    assert!(matches!(
        worker.subscribe(None).await,
        Err(ClientError::Rejected { status: 401, .. })
    ));
}

#[tokio::test]
async fn subscription_quota_keeps_http_actions_available() {
    let first = AgentId::new();
    let second = AgentId::new();
    let (_process, base) = launch(first, second);
    let endpoint = base.strip_prefix("http://").unwrap().parse().unwrap();
    let worker = WorkerClient::new(endpoint, first, SigningKey::from_hex(FIRST).unwrap()).unwrap();
    let mut subscriptions = Vec::new();
    for _ in 0..8 {
        subscriptions.push(worker.subscribe(None).await.unwrap());
    }
    assert!(matches!(
        worker.subscribe(None).await,
        Err(ClientError::Rejected { status: 503, .. })
    ));
    assert!(worker.claim().await.unwrap().is_none());
    drop(subscriptions.pop());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match worker.subscribe(None).await {
                Ok(_) => break,
                Err(ClientError::Rejected { status: 503, .. }) => {
                    tokio::time::sleep(Duration::from_millis(100)).await
                }
                Err(error) => panic!("unexpected reconnect error: {error}"),
            }
        }
    })
    .await
    .unwrap();
}
