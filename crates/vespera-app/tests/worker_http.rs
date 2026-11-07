use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use reqwest::{Client, Response, StatusCode};
use ring::hmac;
use serde_json::{Value, json};
use uuid::Uuid;
use vespera_core::{AgentId, TaskId};
use vespera_sdk::{
    ClientError, ErrorCode, OperatorClient, SigningKey, Submission, TaskState, WorkerClient,
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
            "Vespera-HMAC-SHA256\nPOST\n/v2/{role}\n{principal}\n{session_id}\n{elapsed}\n{nonce}\n{body}"
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
    let child = Command::new(env!("CARGO_BIN_EXE_vespera"))
        .args(["serve", "--listen", "127.0.0.1:0", "--access-file"])
        .arg(&policy)
        .env("TEST_OPERATOR_TOKEN", OPERATOR)
        .env("TEST_FIRST_TOKEN", FIRST)
        .env("TEST_SECOND_TOKEN", SECOND)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
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
    assert!(base.starts_with("http://127.0.0.1:"));
    (process, base)
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
    operator.revoke(first).await.unwrap();
    assert!(matches!(
        worker.claim().await,
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
