use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use reqwest::{Client, Response, StatusCode};
use serde_json::{Value, json};
use vespera_core::{AgentId, TaskId};

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

async fn post(client: &Client, base: &str, role: &str, token: &str, body: Value) -> Response {
    client
        .post(format!("{base}/v1/{role}"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn serves_fenced_work_to_authenticated_workers_only() {
    let first = AgentId::new();
    let second = AgentId::new();
    let task = TaskId::new();
    let policy = std::env::temp_dir().join(format!("vespera-worker-{task}.json"));
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
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let submit = json!({
        "action": "submit", "task_id": task, "prompt": "Run one bounded task",
        "required_capabilities": ["text-generation"], "eligible_workers": [first, second],
        "lifetime_seconds": 60, "max_attempts": 2
    });
    let claim = json!({"action": "claim"});
    assert_eq!(
        post(&client, &base, "operator", FIRST, submit.clone())
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(&client, &base, "worker", OPERATOR, claim.clone())
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post(&client, &base, "worker", FIRST, claim.clone())
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let spoof = post(
        &client,
        &base,
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
    let oversized = client
        .post(format!("{base}/v1/worker"))
        .bearer_auth(FIRST)
        .header("content-type", "application/json")
        .body(" ".repeat(512 * 1024 + 1))
        .send()
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        post(&client, &base, "operator", OPERATOR, submit.clone())
            .await
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        post(&client, &base, "operator", OPERATOR, submit)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let lease = post(&client, &base, "worker", FIRST, claim.clone())
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
        post(&client, &base, "worker", SECOND, renew.clone())
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        post(&client, &base, "worker", FIRST, renew).await.status(),
        StatusCode::OK
    );
    let revoke = json!({"action": "revoke", "worker_id": first});
    assert_eq!(
        post(&client, &base, "operator", OPERATOR, revoke)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(&client, &base, "worker", FIRST, claim.clone())
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let reassigned = post(&client, &base, "worker", SECOND, claim)
        .await
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(reassigned["fence"]["generation"], 2);
    let stale = json!({"action": "complete", "fence": lease["fence"], "content": "stale"});
    assert_eq!(
        post(&client, &base, "worker", SECOND, stale).await.status(),
        StatusCode::CONFLICT
    );
    let complete =
        json!({"action": "complete", "fence": reassigned["fence"], "content": "accepted"});
    let accepted = post(&client, &base, "worker", SECOND, complete.clone())
        .await
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let retried = post(&client, &base, "worker", SECOND, complete)
        .await
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(accepted, retried);
    assert_eq!(accepted["worker"], json!(second));
    let cancel = post(
        &client,
        &base,
        "operator",
        OPERATOR,
        json!({"action": "cancel", "task_id": task}),
    )
    .await;
    assert_eq!(cancel.json::<Value>().await.unwrap()["state"], "completed");
    let inspected = post(
        &client,
        &base,
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
