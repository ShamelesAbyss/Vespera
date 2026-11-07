use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct ScratchDirectory(PathBuf);

impl ScratchDirectory {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "vespera-cli-test-{}-{nanos}-{counter}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("unique test directory");
        Self(path)
    }

    fn write_config(&self, content: &str) -> PathBuf {
        let path = self.0.join("test.toml");
        fs::write(&path, content).expect("test configuration");
        path
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn execute(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vespera"))
        .args(args)
        .current_dir(workspace())
        .output()
        .expect("CLI process")
}

#[test]
fn validates_sample_as_machine_readable_inventory() {
    let output = execute(&["validate-config", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(report["valid"], true);
    assert_eq!(report["models_registered"], 1);
    assert_eq!(report["agents_declared"], 1);
    assert_eq!(report["allow_remote"], false);
    assert_eq!(report["resource_budget"]["max_concurrent_inferences"], 1);
    assert!(
        report["notice"]
            .as_str()
            .expect("notice")
            .contains("have not been tested or started")
    );
    let text = String::from_utf8(output.stdout).expect("UTF-8");
    assert!(!text.contains("127.0.0.1"));
    assert!(!text.contains("credential_env"));
}

#[test]
fn validates_human_output() {
    let output = execute(&["validate-config"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("configuration valid"));
}

#[test]
fn unknown_fields_fail_and_inline_secrets_do_not_leak() {
    let directory = ScratchDirectory::new();
    let path = directory.write_config("schema_version = 1\napi_key = \"top-secret-test-value\"\n");
    let output = execute(&["validate-config", "--config", path.to_str().expect("path")]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("input excerpts are suppressed"));
    assert!(!error.contains("top-secret-test-value"));
}

#[test]
fn embedded_endpoint_credentials_are_redacted_from_failure() {
    let directory = ScratchDirectory::new();
    let sample = fs::read_to_string(workspace().join("config/local.toml")).expect("sample");
    let path = directory.write_config(&sample.replace(
        "http://127.0.0.1:11434",
        "http://user:top-secret-test-value@127.0.0.1:11434",
    ));
    let output = execute(&["validate-config", "--config", path.to_str().expect("path")]);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("embedded URL credentials are forbidden"));
    assert!(!error.contains("top-secret-test-value"));
}

#[test]
fn unreachable_provider_is_only_a_declaration_not_a_connection_test() {
    let directory = ScratchDirectory::new();
    let sample = fs::read_to_string(workspace().join("config/local.toml")).expect("sample");
    let path =
        directory.write_config(&sample.replace("http://127.0.0.1:11434", "http://127.0.0.1:1"));
    let output = execute(&[
        "validate-config",
        "--config",
        path.to_str().expect("path"),
        "--json",
    ]);
    assert!(output.status.success());
}

#[test]
fn missing_config_fails_cleanly() {
    let directory = ScratchDirectory::new();
    let path = directory.0.join("missing.toml");
    let output = execute(&["validate-config", "--config", path.to_str().expect("path")]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("could not read configuration"));
}

#[test]
fn remote_endpoint_requires_opt_in() {
    let directory = ScratchDirectory::new();
    let sample = fs::read_to_string(workspace().join("config/local.toml")).expect("sample");
    let path =
        directory.write_config(&sample.replace("http://127.0.0.1:11434", "https://example.com/v1"));
    let output = execute(&["validate-config", "--config", path.to_str().expect("path")]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("local-first network policy"));
}

#[test]
fn doctor_outputs_evidence_not_invented_vram() {
    let output = execute(&["doctor", "--json"]);
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("doctor JSON");
    assert_eq!(report["os"], std::env::consts::OS);
    assert_eq!(report["architecture"], std::env::consts::ARCH);
    assert!(report["gpu_vram_bytes"].is_null());
    assert!(
        report["gpu_telemetry_status"]
            .as_str()
            .expect("GPU status")
            .contains("not implemented")
    );
}

#[test]
fn runtime_help_advertises_only_supported_commands_and_requires_execution_inputs() {
    let output = execute(&["run-agent"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("required"));
    let help = execute(&["--help"]);
    assert!(help.status.success());
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("validate-config"));
    assert!(text.contains("doctor"));
    assert!(text.contains("run-agent"));
    assert!(text.contains("provider-health"));
    assert!(text.contains("models"));
    assert!(text.contains("agents"));
    assert!(!text.contains("connect-provider"));
}

#[test]
fn version_is_available() {
    let output = execute(&["--version"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("vespera 0.1.0"));
}

#[derive(Clone)]
struct CapturedRequest {
    path: String,
    headers: String,
    body: String,
}
struct HttpFixture {
    endpoint: String,
    requests: std::sync::Arc<std::sync::Mutex<Vec<CapturedRequest>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    inference_delay_ms: std::sync::Arc<AtomicU64>,
    lifecycle_delay_ms: std::sync::Arc<AtomicU64>,
    hold_health: std::sync::Arc<AtomicBool>,
    hold_inference: std::sync::Arc<AtomicBool>,
    deny_health: std::sync::Arc<AtomicBool>,
    residency: std::sync::Arc<std::sync::Mutex<String>>,
}
impl HttpFixture {
    fn new(health_denied: bool) -> Self {
        Self::with_ollama_answer(health_denied, "fixture integrated answer")
    }

    fn with_ollama_answer(health_denied: bool, answer: &str) -> Self {
        use std::io::{Read, Write};
        let chat_response = serde_json::json!({
            "message": {"role": "assistant", "content": answer},
            "done": true, "done_reason": "stop", "prompt_eval_count": 10, "eval_count": 3
        })
        .to_string();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let capture = requests.clone();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = stop.clone();
        let inference_delay_ms = std::sync::Arc::new(AtomicU64::new(0));
        let delay = inference_delay_ms.clone();
        let lifecycle_delay_ms = std::sync::Arc::new(AtomicU64::new(0));
        let lifecycle_delay = lifecycle_delay_ms.clone();
        let hold_health = std::sync::Arc::new(AtomicBool::new(false));
        let hold_inference = std::sync::Arc::new(AtomicBool::new(false));
        let health_gate = hold_health.clone();
        let inference_gate = hold_inference.clone();
        let deny_health = std::sync::Arc::new(AtomicBool::new(health_denied));
        let denied = deny_health.clone();
        let residency = std::sync::Arc::new(std::sync::Mutex::new(r#"{"models":[]}"#.to_owned()));
        let observed_residency = residency.clone();
        let worker = std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let (mut socket, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    }
                    Err(_) => break,
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let request = loop {
                    let mut buffer = [0; 4096];
                    let count = match socket.read(&mut buffer) {
                        Ok(count) if count > 0 => count,
                        _ => break None,
                    };
                    bytes.extend_from_slice(&buffer[..count]);
                    if bytes.len() > 1024 * 1024 {
                        break None;
                    }
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_string();
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            let path = headers
                                .lines()
                                .next()
                                .unwrap()
                                .split_whitespace()
                                .nth(1)
                                .unwrap()
                                .to_owned();
                            break Some(CapturedRequest {
                                path,
                                headers,
                                body: String::from_utf8_lossy(&bytes[end + 4..end + 4 + length])
                                    .to_string(),
                            });
                        }
                    }
                };
                let Some(request) = request else {
                    continue;
                };
                let residency_body = observed_residency.lock().unwrap().clone();
                let (status, body) = if denied.load(Ordering::SeqCst)
                    && matches!(request.path.as_str(), "/api/version" | "/v1/models")
                {
                    ("401 Unauthorized", "fixture-private-error-body")
                } else {
                    match request.path.as_str() {
                        "/api/version" => ("200 OK", r#"{"version":"fixture-v1"}"#),
                        "/api/generate" => ("200 OK", r#"{"done":true}"#),
                        "/api/ps" => ("200 OK", residency_body.as_str()),
                        "/api/chat" => ("200 OK", chat_response.as_str()),
                        "/v1/models" => ("200 OK", r#"{"data":[{"id":"fixture-model"}]}"#),
                        "/v1/chat/completions" => (
                            "200 OK",
                            r#"{"choices":[{"message":{"role":"assistant","content":"fixture integrated answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":3}}"#,
                        ),
                        _ => ("404 Not Found", "{}"),
                    }
                };
                let inference =
                    matches!(request.path.as_str(), "/api/chat" | "/v1/chat/completions");
                let health = matches!(request.path.as_str(), "/api/version" | "/v1/models");
                let lifecycle = request.path == "/api/generate";
                capture.lock().unwrap().push(request);
                let gate = if inference {
                    Some(&inference_gate)
                } else {
                    health.then_some(&health_gate)
                };
                if let Some(gate) = gate {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
                    while gate.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
                if inference {
                    std::thread::sleep(std::time::Duration::from_millis(
                        delay.load(Ordering::SeqCst),
                    ));
                } else if lifecycle {
                    std::thread::sleep(std::time::Duration::from_millis(
                        lifecycle_delay.load(Ordering::SeqCst),
                    ));
                }
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes());
            }
        });
        Self {
            endpoint,
            requests,
            stop,
            worker: Some(worker),
            inference_delay_ms,
            lifecycle_delay_ms,
            hold_health,
            hold_inference,
            deny_health,
            residency,
        }
    }
    fn captured(&self) -> Vec<CapturedRequest> {
        self.requests.lock().unwrap().clone()
    }
}
impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
const AGENT: &str = "33333333-3333-4333-8333-333333333333";
fn runtime_config(fixture: &HttpFixture, openai: bool, known_costs: bool) -> String {
    let sample = fs::read_to_string(workspace().join("config/local.toml")).unwrap();
    let sample = sample
        .replace("http://127.0.0.1:11434", &fixture.endpoint)
        .replace("example-local-model", "fixture-model");
    let sample = if openai {
        sample.replace("kind = \"ollama\"", "kind = \"open_ai_compatible\"")
    } else {
        sample
    };
    if known_costs {
        sample.replace("availability = \"unknown\"", "availability = \"available\"\ncontext_window_tokens = 512\n[models.resources]\nram_bytes = 100\nvram_bytes = 0")
    } else {
        sample
    }
}
#[test]
fn world_model_choices_are_authorized_bounded_and_revision_fenced() {
    use std::time::{Duration, Instant};
    use vespera_world::{Position, ResidentSpec, Role, World, WorldSpec, WorldStore};

    let provider = HttpFixture::with_ollama_answer(false, r#"{"choice":1}"#);
    let directory = ScratchDirectory::new();
    let database = directory.0.join("world.sqlite3");
    let resident = AGENT.parse().unwrap();
    let initial = World::new(WorldSpec {
        seed: 0,
        width: 5,
        depth: 5,
        depot: Position { x: 0, z: 0 },
        residents: vec![ResidentSpec {
            id: resident,
            name: "private resident label".into(),
            role: Role::Forager,
            position: Position { x: 2, z: 2 },
        }],
        sites: vec![],
    })
    .unwrap();
    WorldStore::initialize(&database, &initial).unwrap();
    let invoke = |config: &Path, revision: &str, apply: bool| {
        let mut flags = vec![
            "world",
            "--database",
            database.to_str().unwrap(),
            "decide",
            "--config",
            config.to_str().unwrap(),
            "--resident",
            AGENT,
            "--revision",
            revision,
            "--resume",
            "--unload-after",
        ];
        if apply {
            flags.push("--apply");
        }
        execute(&flags)
    };
    let config = runtime_config(&provider, false, true);
    let denied = directory.write_config(&config);
    assert!(!invoke(&denied, "0", true).status.success());
    assert!(provider.captured().is_empty());
    let allowed = config.replace("permissions = []", "permissions = [\"simulation_act\"]");
    let config_path = directory.write_config(&allowed);
    assert!(!invoke(&config_path, "1", true).status.success());
    assert!(provider.captured().is_empty());

    let preview = invoke(&config_path, "0", false);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let preview: serde_json::Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert!(preview["committed_revision"].is_null());
    assert_eq!(WorldStore::read(&database).unwrap(), initial);
    let prompt = provider
        .captured()
        .into_iter()
        .find(|request| request.path == "/api/chat")
        .unwrap();
    assert!(!prompt.body.contains("private resident label"));
    assert!(prompt.body.contains("choices_in_index_order"));
    let applied = invoke(&config_path, "0", true);
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(report["committed_revision"], 1);
    let committed = WorldStore::read(&database).unwrap();
    let state = committed.snapshot();
    assert_eq!(state.tick, 0);
    assert_eq!(state.residents[0].position, Position { x: 1, z: 2 });
    let provenance = state.residents[0].last_model_decision.unwrap();
    assert_eq!(
        report["source"]["task_id"],
        provenance.source.task_id.to_string()
    );

    let invalid =
        HttpFixture::with_ollama_answer(false, r#"{"choice":0,"extra":"private response"}"#);
    let invalid_path = directory.write_config(
        &runtime_config(&invalid, false, true)
            .replace("permissions = []", "permissions = [\"simulation_act\"]"),
    );
    let rejected = invoke(&invalid_path, "1", true);
    assert!(!rejected.status.success());
    assert!(!String::from_utf8_lossy(&rejected.stdout).contains("private response"));
    assert_eq!(WorldStore::read(&database).unwrap(), committed);

    let config_path = directory.write_config(&allowed);
    let before = provider.captured().len();
    provider.inference_delay_ms.store(1600, Ordering::SeqCst);
    std::thread::scope(|scope| {
        let pending = scope.spawn(|| invoke(&config_path, "1", true));
        let deadline = Instant::now() + Duration::from_secs(8);
        while !provider.captured()[before..]
            .iter()
            .any(|request| request.path == "/api/chat")
        {
            assert!(!pending.is_finished() && Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(25));
        }
        let advanced = WorldStore::update(&database, World::advance).unwrap();
        let rejected = pending.join().unwrap();
        assert!(!rejected.status.success());
        assert!(String::from_utf8_lossy(&rejected.stderr).contains("world changed"));
        assert_eq!(WorldStore::read(&database).unwrap(), advanced);
    });
}

#[test]
fn world_run_rotates_sparse_decisions_and_keeps_ticking_on_invalid_output() {
    use vespera_world::{Position, ResidentSpec, Role, World, WorldSpec, WorldStore};

    const PEER: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let directory = ScratchDirectory::new();
    let initial = World::new(WorldSpec {
        seed: 17,
        width: 5,
        depth: 5,
        depot: Position { x: 0, z: 0 },
        residents: vec![
            ResidentSpec {
                id: AGENT.parse().unwrap(),
                name: "First".into(),
                role: Role::Forager,
                position: Position { x: 2, z: 2 },
            },
            ResidentSpec {
                id: PEER.parse().unwrap(),
                name: "Second".into(),
                role: Role::Builder,
                position: Position { x: 3, z: 2 },
            },
        ],
        sites: vec![],
    })
    .unwrap();
    let good = HttpFixture::with_ollama_answer(false, r#"{"choice":0}"#);
    let invalid = HttpFixture::with_ollama_answer(false, r#"{"choice":999}"#);
    for (name, provider, expected_revision) in [("good", &good, 7), ("invalid", &invalid, 5)] {
        let config = runtime_config(provider, false, true)
            .replace("permissions = []", "permissions = [\"simulation_act\"]");
        let (_, agent) = config.split_once("[[agents]]").unwrap();
        let peer = agent.replace(AGENT, PEER).replace("Archivist", "Peer");
        let config_path = directory.write_config(&format!("{config}\n[[agents]]{peer}"));
        let database = directory.0.join(format!("{name}.sqlite3"));
        WorldStore::initialize(&database, &initial).unwrap();
        let output = execute(&[
            "world",
            "--database",
            database.to_str().unwrap(),
            "run",
            "--config",
            config_path.to_str().unwrap(),
            "--revision",
            "0",
            "--ticks",
            "5",
            "--reason-every",
            "2",
            "--tick-ms",
            "0",
            "--resident",
            AGENT,
            "--resident",
            PEER,
            "--resume",
            "--unload-after",
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["committed_ticks"], 5);
        assert_eq!(report["last_committed_revision"], expected_revision);
        let attempts = report["attempts"].as_array().unwrap();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0]["resident"], AGENT);
        assert_eq!(attempts[0]["observed_revision"], 2);
        assert_eq!(attempts[1]["resident"], PEER);
        let persisted = WorldStore::read(&database).unwrap();
        let snapshot = persisted.snapshot();
        assert_eq!(snapshot.tick, 5);
        assert_eq!(snapshot.revision, expected_revision);
        let replay = execute(&["world", "--database", database.to_str().unwrap(), "replay"]);
        assert!(
            replay.status.success(),
            "{}",
            String::from_utf8_lossy(&replay.stderr)
        );
        let replay: serde_json::Value = serde_json::from_slice(&replay.stdout).unwrap();
        assert_eq!(replay["world"], serde_json::to_value(&snapshot).unwrap());
        assert_eq!(replay["basis_revision"], 0);
        assert_eq!(
            WorldStore::replay(&database, None).unwrap().world,
            persisted
        );
        if name == "good" {
            assert_eq!(attempts[0]["committed_revision"], 3);
            assert_eq!(attempts[1]["committed_revision"], 6);
            assert_eq!(snapshot.residents[0].last_model_decision.unwrap().tick, 2);
            assert_eq!(snapshot.residents[1].last_model_decision.unwrap().tick, 4);
            let before = WorldStore::replay(&database, Some(2)).unwrap().world;
            assert!(before.snapshot().residents[0].last_model_decision.is_none());
            let after = WorldStore::replay(&database, Some(3)).unwrap().world;
            assert_eq!(
                after.snapshot().residents[0].last_model_decision.unwrap(),
                snapshot.residents[0].last_model_decision.unwrap()
            );
        } else {
            assert!(
                attempts.iter().all(|attempt| attempt["error"].is_string()
                    && attempt["committed_revision"].is_null())
            );
            let mut deterministic = initial.clone();
            for _ in 0..5 {
                deterministic.advance().unwrap();
            }
            assert_eq!(persisted, deterministic);
        }
        let requests = provider.captured();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/api/chat")
                .count(),
            2
        );
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.path == "/api/generate")
                .count(),
            2,
            "shared load and unload"
        );
    }
}

#[test]
fn world_run_observes_preparation_and_inference_without_advancing_the_world() {
    use std::time::{Duration, Instant};
    use vespera_world::{
        Position, ReasoningPhase, ResidentSpec, Role, World, WorldSpec, WorldStore,
    };

    let fixture = HttpFixture::with_ollama_answer(false, r#"{"choice":0}"#);
    fixture.hold_health.store(true, Ordering::SeqCst);
    fixture.hold_inference.store(true, Ordering::SeqCst);
    let directory = ScratchDirectory::new();
    let database = directory.0.join("world.sqlite3");
    let resident = AGENT.parse().unwrap();
    let initial = World::new(WorldSpec {
        seed: 0,
        width: 5,
        depth: 5,
        depot: Position { x: 0, z: 0 },
        residents: vec![ResidentSpec {
            id: resident,
            name: "Fixture".into(),
            role: Role::Forager,
            position: Position { x: 2, z: 2 },
        }],
        sites: vec![],
    })
    .unwrap();
    WorldStore::initialize(&database, &initial).unwrap();
    let config = directory.write_config(
        &runtime_config(&fixture, false, true)
            .replace("permissions = []", "permissions = [\"simulation_act\"]"),
    );
    let now = || u64::try_from(UNIX_EPOCH.elapsed().unwrap().as_millis()).unwrap();
    assert!(
        WorldStore::read_observed(&database, now())
            .unwrap()
            .1
            .is_none()
    );
    let task = std::thread::scope(|scope| {
        let child = scope.spawn(|| {
            execute(&[
                "world",
                "--database",
                database.to_str().unwrap(),
                "run",
                "--config",
                config.to_str().unwrap(),
                "--revision",
                "0",
                "--ticks",
                "1",
                "--reason-every",
                "1",
                "--tick-ms",
                "0",
                "--resident",
                AGENT,
                "--resume",
                "--unload-after",
            ])
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut preparing = None;
        let active = loop {
            let (world, status) = WorldStore::read_observed(&database, now()).unwrap();
            if let Some(status) = status {
                assert_eq!(status.resident, resident);
                let state = world.snapshot();
                assert_eq!((state.tick, state.revision), (1, 1));
                assert!(state.residents[0].last_model_decision.is_none());
                match status.phase {
                    ReasoningPhase::Preparing {} => {
                        preparing = Some(status.task_id);
                        fixture.hold_health.store(false, Ordering::SeqCst);
                    }
                    ReasoningPhase::InferenceActive { .. } => break Some(status),
                }
            }
            if child.is_finished() || Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        if active.is_some() {
            assert!(
                WorldStore::read_observed(&database, now() + 2500)
                    .unwrap()
                    .1
                    .is_none()
            );
        }
        fixture.hold_health.store(false, Ordering::SeqCst);
        fixture.hold_inference.store(false, Ordering::SeqCst);
        let output = child.join().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let active = active.expect("provider request never received an active heartbeat");
        assert_eq!(preparing, Some(active.task_id));
        active.task_id
    });
    let (committed, status) = WorldStore::read_observed(&database, now()).unwrap();
    assert!(status.is_none());
    let state = committed.snapshot();
    assert_eq!((state.tick, state.revision), (1, 2));
    assert_eq!(
        state.residents[0]
            .last_model_decision
            .unwrap()
            .source
            .task_id,
        task
    );
}

fn run_fixture_agent(directory: &ScratchDirectory, config: &str, extra: &[&str]) -> Output {
    let config = directory.write_config(config);
    let prompt = directory.0.join("prompt.txt");
    fs::write(&prompt, "bounded integrated task").unwrap();
    let mut args = vec![
        "run-agent",
        "--config",
        config.to_str().unwrap(),
        "--agent",
        AGENT,
        "--prompt-file",
        prompt.to_str().unwrap(),
        "--max-output-tokens",
        "32",
        "--json",
    ];
    args.extend_from_slice(extra);
    execute(&args)
}

#[test]
fn model_catalog_reopens_and_supplies_a_real_runtime_registration() {
    let provider = HttpFixture::new(false);
    let scratch = ScratchDirectory::new();
    let config = runtime_config(&provider, false, false);
    let config_file = scratch.write_config(&config);
    let database = scratch.0.join("catalog.sqlite3");
    let descriptor = scratch.0.join("model.json");
    let run = |extra: &[&str]| {
        let mut args = vec![
            "catalog",
            "--database",
            database.to_str().unwrap(),
            "--config",
            config_file.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        execute(&args)
    };
    let json = |result: Output| -> serde_json::Value {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice(&result.stdout).unwrap()
    };
    let put = |revision: &str, model: &serde_json::Value| {
        fs::write(&descriptor, serde_json::to_vec(model).unwrap()).unwrap();
        run(&[
            "put",
            "--model-file",
            descriptor.to_str().unwrap(),
            "--revision",
            revision,
        ])
    };
    assert!(!run(&["list"]).status.success());
    assert!(!database.exists());
    let foreign = scratch.0.join("foreign.sqlite3");
    assert!(
        execute(&["events", "--journal", foreign.to_str().unwrap(), "init"])
            .status
            .success()
    );
    let foreign_bytes = fs::read(&foreign).unwrap();
    assert!(
        !execute(&[
            "catalog",
            "--database",
            foreign.to_str().unwrap(),
            "--config",
            config_file.to_str().unwrap(),
            "init",
        ])
        .status
        .success()
    );
    assert_eq!(fs::read(&foreign).unwrap(), foreign_bytes);
    assert_eq!(json(run(&["init"]))["revision"], 0);
    assert!(!run(&["init"]).status.success());
    let mut model = json(run(&["list"]))["models"][0].clone();
    let original = model["id"].as_str().unwrap().to_owned();
    model["name"] = "Durable fixture".into();
    model["resources"] = serde_json::json!({ "ram_bytes": 100, "vram_bytes": 0 });
    model["context_window_tokens"] = 512.into();
    assert_eq!(json(put("0", &model))["revision"], 1);
    assert!(!put("0", &model).status.success());
    assert!(
        !run(&["remove", "--model", &original, "--revision", "1"])
            .status
            .success()
    );
    let mut extra = model.clone();
    extra["id"] = "b16b00b5-1111-4111-8111-111111111111".into();
    extra["identifier"] = "another-fixture-model".into();
    assert_eq!(json(put("1", &extra))["revision"], 2);
    let extra_id = extra["id"].as_str().unwrap();
    assert_eq!(
        json(run(&["remove", "--model", extra_id, "--revision", "2"]))["revision"],
        3
    );
    extra["identifier"] = "different-binding".into();
    assert!(!put("3", &extra).status.success());
    fs::write(&descriptor, serde_json::to_vec(&model).unwrap()).unwrap();
    let winners = std::thread::scope(|scope| {
        let commit = || {
            run(&[
                "put",
                "--model-file",
                descriptor.to_str().unwrap(),
                "--revision",
                "3",
            ])
        };
        let competing = scope.spawn(commit);
        u8::from(commit().status.success()) + u8::from(competing.join().unwrap().status.success())
    });
    assert_eq!(winners, 1);
    let recovered = json(run(&["list"]));
    assert_eq!(recovered["revision"], 4);
    assert_eq!(recovered["models"], serde_json::json!([model]));
    assert!(provider.captured().is_empty());
    let completed = json(run_fixture_agent(
        &scratch,
        &config,
        &[
            "--resume",
            "--model-catalog",
            database.to_str().unwrap(),
            "--unload-after",
        ],
    ));
    assert_eq!(completed["execution"]["model_id"], original);
    assert_eq!(completed["execution"]["task"]["state"], "completed");
    assert!(
        provider
            .captured()
            .iter()
            .any(|request| request.path == "/api/chat")
    );
    assert_eq!(json(run(&["list"])), recovered);
}

#[test]
fn startup_residency_fences_external_models_and_charges_memory() {
    let fixture = HttpFixture::new(false);
    let scratch = ScratchDirectory::new();
    let config =
        runtime_config(&fixture, false, true).replace("ram_bytes = 4294967296", "ram_bytes = 120");
    let execute = |model| {
        *fixture.residency.lock().unwrap() = serde_json::json!({
            "models": [{"name": model, "size": 90, "size_vram": 0}]
        })
        .to_string();
        run_fixture_agent(
            &scratch,
            &config,
            &["--resume", "--reconcile-residency", "--unload-after"],
        )
    };
    for (name, expected) in [
        (
            "external-model",
            "RAM bytes request/usage 190 exceeds budget 120",
        ),
        (
            "https://registry.ollama.ai/library/fixture-model:latest",
            "externally resident",
        ),
    ] {
        let result = execute(name);
        assert!(!result.status.success());
        let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert!(
            report["error"].as_str().unwrap().contains(expected),
            "{report}"
        );
        assert_eq!(
            report["startup_residency"],
            serde_json::json!({
                "providers_observed": 1, "external_models": 1, "ram_bytes": 90, "vram_bytes": 0
            })
        );
        assert_eq!(report["cleanup"]["remaining_models"], serde_json::json!([]));
    }
    assert!(
        fixture
            .captured()
            .iter()
            .all(|request| matches!(request.path.as_str(), "/api/ps" | "/api/version"))
    );

    *fixture.residency.lock().unwrap() = r#"{"models":[]}"#.into();
    let result = run_fixture_agent(
        &scratch,
        &config,
        &["--resume", "--reconcile-residency", "--unload-after"],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["startup_residency"]["external_models"], 0);
    assert_eq!(report["execution"]["task"]["state"], "completed");
    assert_eq!(
        fixture
            .captured()
            .iter()
            .filter(|request| request.path == "/api/generate")
            .count(),
        2
    );
}

#[test]
fn scheduler_telemetry_tracks_a_real_cli_lease_through_cleanup() {
    use std::time::{Duration, Instant};
    use vespera_core::{ReservationState, SchedulerSample};

    let fixture = HttpFixture::new(false);
    fixture.inference_delay_ms.store(2200, Ordering::SeqCst);
    fixture.lifecycle_delay_ms.store(900, Ordering::SeqCst);
    let directory = ScratchDirectory::new();
    let target = directory.0.join("observation");
    let path = target.join(SchedulerSample::FILE_NAME);
    let config = format!(
        "{}\n[inference_overhead]\nram_bytes = 20\nram_bytes_per_context_token = 2\n",
        runtime_config(&fixture, false, true)
    );
    let output = std::thread::scope(|scope| {
        let child = scope.spawn(|| {
            run_fixture_agent(
                &directory,
                &config,
                &[
                    "--resume",
                    "--unload-after",
                    "--context-tokens",
                    "512",
                    "--scheduler-telemetry",
                    target.to_str().unwrap(),
                ],
            )
        });
        let deadline = Instant::now() + Duration::from_secs(12);
        let mut saw_loading = false;
        let active = loop {
            if let Ok(bytes) = fs::read(&path) {
                let sample: SchedulerSample =
                    serde_json::from_slice(&bytes).expect("atomic sample");
                assert!(sample.is_supported());
                if let Some(model) = sample.models.as_ref().and_then(|models| models.first()) {
                    if model.state == ReservationState::Loading {
                        assert_eq!(sample.reserved.ram_bytes, 1144);
                        assert_eq!(model.charged_slots, 1);
                        assert_eq!(model.issued_leases, 0);
                        assert!(sample.tasks.as_ref().unwrap().is_empty());
                        saw_loading = true;
                    }
                    if model.issued_leases == 1
                        && sample
                            .tasks
                            .as_ref()
                            .unwrap()
                            .first()
                            .is_some_and(|task| task.memory_context_records.is_some())
                    {
                        break Some(sample);
                    }
                }
                if sample.finished {
                    break None;
                }
            }
            if child.is_finished() || Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        (child.join().unwrap(), active, saw_loading)
    });
    assert!(
        output.0.status.success(),
        "{}",
        String::from_utf8_lossy(&output.0.stderr)
    );
    let active = output
        .1
        .expect("inference was observed while its lease was active");
    assert!(output.2);
    assert_eq!(active.schema_version, SchedulerSample::SCHEMA_VERSION);
    assert!(!active.finished);
    assert_eq!(active.admitted, 1);
    assert_eq!(active.reserved.ram_bytes, 1144);
    assert_eq!(active.model_reservations, 1);
    assert_eq!(active.provider_health.healthy, 1);
    let model = active.models.as_ref().unwrap()[0];
    assert_eq!(model.state, ReservationState::Loaded);
    assert_eq!(
        model.model_id.to_string(),
        "22222222-2222-4222-8222-222222222222"
    );
    assert_eq!(
        model.provider_id.to_string(),
        "11111111-1111-4111-8111-111111111111"
    );
    assert_eq!((model.ram_bytes, model.vram_bytes), (100, 0));
    assert_eq!((model.charged_slots, model.issued_leases), (1, 1));
    let [owner] = active.tasks.as_deref().unwrap() else {
        panic!("expected one task owner");
    };
    let report: serde_json::Value = serde_json::from_slice(&output.0.stdout).unwrap();
    let execution = &report["execution"];
    assert_eq!(
        owner.task_id.to_string(),
        execution["task"]["id"].as_str().unwrap()
    );
    assert_eq!(owner.agent_id.to_string(), AGENT);
    assert_eq!(owner.memory_context_records, Some(0));
    assert_eq!(
        owner.lease_id.to_string(),
        execution["lease_id"].as_str().unwrap()
    );
    assert_eq!(
        (owner.model_id, owner.provider_id),
        (model.model_id, model.provider_id)
    );
    let mut mismatched = active.clone();
    mismatched.tasks.as_mut().unwrap()[0].provider_id = vespera_core::ProviderId::new();
    assert!(!mismatched.is_supported());
    let mut duplicated = active.clone();
    duplicated.tasks.as_mut().unwrap().push(*owner);
    assert!(!duplicated.is_supported());
    let mut v3 = active.clone();
    v3.schema_version = 3;
    assert!(!v3.is_supported());
    v3.tasks.as_mut().unwrap()[0].memory_context_records = None;
    assert!(v3.is_supported());
    let mut legacy = serde_json::to_value(&active).unwrap();
    legacy["schema_version"] = 2.into();
    legacy.as_object_mut().unwrap().remove("tasks");
    let v2: SchedulerSample = serde_json::from_value(legacy.clone()).unwrap();
    assert!(v2.is_supported() && v2.models.is_some() && v2.tasks.is_none());
    legacy["schema_version"] = 1.into();
    legacy.as_object_mut().unwrap().remove("models");
    let legacy: SchedulerSample = serde_json::from_value(legacy).unwrap();
    assert!(legacy.is_supported() && legacy.models.is_none());
    let final_bytes = fs::read(&path).unwrap();
    let final_sample: SchedulerSample = serde_json::from_slice(&final_bytes).unwrap();
    assert!(final_sample.finished && final_sample.queue_closed);
    assert_eq!(final_sample.admitted, 0);
    assert_eq!(final_sample.reserved, Default::default());
    assert_eq!(final_sample.model_reservations, 0);
    assert!(final_sample.models.unwrap().is_empty());
    assert!(final_sample.tasks.unwrap().is_empty());
    assert!(
        !String::from_utf8(final_bytes.clone())
            .unwrap()
            .contains("fixture")
    );
    let duplicate = run_fixture_agent(
        &directory,
        &config,
        &[
            "--resume",
            "--scheduler-telemetry",
            target.to_str().unwrap(),
        ],
    );
    assert!(!duplicate.status.success());
    assert_eq!(fs::read(path).unwrap(), final_bytes);
}

#[test]
fn agent_memory_context_and_generated_result_round_trip() {
    use std::time::{Duration, Instant};
    use vespera_core::{MemoryId, MemoryLayer, MemoryRecord, Permission, SchedulerSample};
    use vespera_memory::{MemoryOrigin, MemoryQuery, MemoryStore, StorageLimits};

    let fixture = HttpFixture::new(false);
    fixture.hold_inference.store(true, Ordering::SeqCst);
    let directory = ScratchDirectory::new();
    let config = runtime_config(&fixture, false, true).replace(
        "permissions = []",
        "permissions = [\"memory_read\", \"memory_write\"]",
    );
    let database = directory.0.join("memory.sqlite3");
    let phrase = directory.0.join("phrase.txt");
    fs::write(&phrase, "amber").unwrap();
    let actor = AGENT.parse().unwrap();
    let authority = vespera_app::memory::configured_authorizer(
        &vespera_config::AppConfig::load(directory.write_config(&config)).unwrap(),
        actor,
        &[Permission::MemoryRead, Permission::MemoryWrite],
    )
    .unwrap();
    let store = MemoryStore::open(&database, StorageLimits::default(), authority).unwrap();
    let id = MemoryId::new();
    for (memory_id, content) in [
        (MemoryId::new(), format!("amber {}", "x".repeat(8192))),
        (id, "amber \"operator record\"".to_owned()),
    ] {
        store
            .insert(
                actor,
                MemoryRecord {
                    id: memory_id,
                    agent_id: actor,
                    layer: MemoryLayer::Observation,
                    content,
                    created_at_unix_ms: 1,
                    source_task_id: None,
                },
                MemoryOrigin::Observation,
            )
            .unwrap();
    }
    let telemetry = directory.0.join("observation");
    let file = telemetry.join(SchedulerSample::FILE_NAME);
    let (output, observed) = std::thread::scope(|scope| {
        let child = scope.spawn(|| {
            run_fixture_agent(
                &directory,
                &config,
                &[
                    "--resume",
                    "--memory-database",
                    database.to_str().unwrap(),
                    "--memory-query-file",
                    phrase.to_str().unwrap(),
                    "--remember-result",
                    "--scheduler-telemetry",
                    telemetry.to_str().unwrap(),
                ],
            )
        });
        let deadline = Instant::now() + Duration::from_secs(12);
        let observed = loop {
            if let Ok(bytes) = fs::read(&file) {
                let sample: SchedulerSample = serde_json::from_slice(&bytes).unwrap();
                assert!(sample.is_supported());
                if let Some(task) = sample.tasks.as_ref().and_then(|tasks| tasks.first())
                    && task.memory_context_records.is_some()
                {
                    let data = String::from_utf8(bytes).unwrap();
                    assert!(!data.contains("amber"));
                    assert!(!data.contains("operator record"));
                    assert!(!data.contains(&id.to_string()));
                    break Some(*task);
                }
            }
            if child.is_finished() || Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        fixture.hold_inference.store(false, Ordering::SeqCst);
        (child.join().unwrap(), observed)
    });
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let observed = observed.expect("request memory was sampled during inference");
    assert_eq!(observed.memory_context_records, Some(1));
    assert_eq!(
        observed.task_id.to_string(),
        report["task_id"].as_str().unwrap()
    );
    assert_eq!(
        report["execution"]["memory_context_ids"],
        serde_json::json!([id])
    );
    assert_eq!(
        report["execution"]["task"]["prompt"],
        "bounded integrated task"
    );
    let requests = fixture.captured();
    let request = requests
        .iter()
        .find(|request| request.path == "/api/chat")
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&request.body).unwrap();
    let messages = payload["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "user");
    let context: serde_json::Value =
        serde_json::from_str(messages[0]["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        context["untrusted_memory"],
        serde_json::json!([{
            "id": id, "content": "amber \"operator record\""
        }])
    );
    assert_eq!(messages[1]["content"], "bounded integrated task");
    let memories = store
        .list(
            actor,
            actor,
            &MemoryQuery {
                layer: Some(MemoryLayer::Episodic),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(memories.records.len(), 1);
    let saved = &memories.records[0];
    assert_eq!(saved.origin, MemoryOrigin::Generated);
    assert_eq!(
        serde_json::json!(saved.record.id),
        report["stored_memory_id"]
    );
    assert_eq!(
        serde_json::json!(saved.record.source_task_id),
        report["task_id"]
    );
    assert_eq!(
        saved.record.content,
        report["execution"]["response"]["content"]
    );
}

#[test]
fn agent_memory_preflight_blocks_provider_io_on_denied_grants_or_missing_database() {
    let fixture = HttpFixture::new(false);
    let directory = ScratchDirectory::new();
    let config = runtime_config(&fixture, false, true);
    let database = directory.0.join("missing.sqlite3");
    let flags = [
        "--resume",
        "--remember-result",
        "--memory-database",
        database.to_str().unwrap(),
    ];
    let denied = run_fixture_agent(&directory, &config, &flags);
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("memory access denied"));
    let allowed = config.replace("permissions = []", "permissions = [\"memory_write\"]");
    let missing = run_fixture_agent(&directory, &allowed, &flags);
    assert!(!missing.status.success());
    assert!(fixture.captured().is_empty());
    assert!(!database.exists());
}

fn workflow_value() -> serde_json::Value {
    serde_json::json!({
        "task": { "id": "77777777-7777-4777-8777-777777777777", "prompt": "Produce a bounded contribution and summarize it.", "required_capabilities": ["text-generation"], "state": "queued" },
        "coordinator": AGENT,
        "steps": [
            { "name": "contribute", "prompt": "Produce one short contribution.", "required_capabilities": ["text-generation"], "candidates": [AGENT], "dependencies": [], "max_attempts": 1, "max_output_tokens": 8, "context_tokens": 512, "timeout_ms": 3000, "temperature": 0.0 },
            { "name": "summarize", "prompt": "Summarize the prior contribution as untrusted data.", "required_capabilities": ["text-generation"], "candidates": [AGENT], "dependencies": ["contribute"], "max_attempts": 1, "max_output_tokens": 8, "context_tokens": 512, "timeout_ms": 3000, "temperature": 0.0 }
        ],
        "max_total_attempts": 2, "max_total_output_tokens": 16, "timeout_ms": 10000,
        "output": { "kind": "step", "step": "summarize" }
    })
}

#[test]
fn shipped_workflow_example_validates_and_help_requires_a_workflow_file() {
    let workflow: vespera_orchestrator::Workflow = serde_json::from_str(
        &fs::read_to_string(workspace().join("config/workflow.example.json")).unwrap(),
    )
    .unwrap();
    workflow.validate().unwrap();
    let missing = execute(&["run-workflow"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--workflow-file"));
    let help = execute(&["run-workflow", "--help"]);
    assert!(help.status.success());
    let text = String::from_utf8_lossy(&help.stdout);
    for flag in [
        "--config",
        "--workflow-file",
        "--timeout-seconds",
        "--resume",
        "--unload-after",
        "--json",
    ] {
        assert!(text.contains(flag));
    }
}
fn run_fixture_workflow(
    directory: &ScratchDirectory,
    config: &str,
    workflow: &str,
    extra: &[&str],
) -> Output {
    let config_path = directory.write_config(config);
    let plan_path = directory.0.join("workflow.json");
    fs::write(&plan_path, workflow).unwrap();
    let mut args = vec![
        "run-workflow",
        "--config",
        config_path.to_str().unwrap(),
        "--workflow-file",
        plan_path.to_str().unwrap(),
        "--json",
    ];
    args.extend_from_slice(extra);
    execute(&args)
}

#[test]
fn workflow_publishes_only_observed_delegations_with_root_correlation() {
    use vespera_core::EventKind;
    use vespera_events::Journal;

    const WORKER: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let fixture = HttpFixture::new(false);
    let directory = ScratchDirectory::new();
    let config = runtime_config(&fixture, false, true)
        .replace("permissions = []", "permissions = [\"delegate\"]");
    let (_, coordinator) = config.split_once("[[agents]]").unwrap();
    let worker = coordinator
        .replace(AGENT, WORKER)
        .replace("Archivist", "Worker");
    let config = format!("{config}\n[[agents]]{worker}");
    let mut workflow = workflow_value();
    workflow["steps"][0]["candidates"] = serde_json::json!([WORKER]);
    let journal_path = directory.0.join("swarm.sqlite3");
    Journal::initialize(&journal_path, 16).unwrap();
    let output = run_fixture_workflow(
        &directory,
        &config,
        &workflow.to_string(),
        &[
            "--resume",
            "--event-journal",
            journal_path.to_str().unwrap(),
            "--record-delegations",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let entries = Journal::open_reader(&journal_path)
        .unwrap()
        .replay(None, 16)
        .unwrap()
        .entries;
    assert_eq!(entries.len(), 3);
    let root = workflow["task"]["id"].as_str().unwrap().parse().unwrap();
    for (index, entry) in entries.iter().take(2).enumerate() {
        let EventKind::TaskDelegated {
            task_id,
            from_agent_id,
            to_agent_id,
        } = &entry.event.kind
        else {
            panic!("expected delegation");
        };
        let attempt = &report["workflow"]["attempts"][index];
        assert_eq!(attempt["delegated"], true);
        assert_eq!(attempt["task_id"], task_id.to_string());
        assert_eq!(attempt["agent_id"], to_agent_id.to_string());
        assert_eq!(from_agent_id.to_string(), AGENT);
        assert_eq!(entry.event.correlation_task_id, Some(root));
    }
    assert_eq!(
        entries[2].event.kind,
        EventKind::TaskCompleted { task_id: root }
    );
    assert_eq!(report["event_publication"]["cursor"]["sequence"], 3);
    let evidence = serde_json::to_string(&entries).unwrap();
    assert!(!evidence.contains("fixture integrated answer"));
    assert!(!evidence.contains("Produce one short contribution"));
}

#[test]
fn live_workflow_evidence_arrives_before_completion_and_tracks_dependency_inputs() {
    use std::time::{Duration, Instant};
    use vespera_core::{EventKind, TaskState, WorkflowDependency};
    use vespera_events::Journal;

    let fixture = HttpFixture::new(false);
    fixture.inference_delay_ms.store(1100, Ordering::SeqCst);
    let directory = ScratchDirectory::new();
    let config = runtime_config(&fixture, false, true)
        .replace("permissions = []", "permissions = [\"delegate\"]");
    let workflow = workflow_value().to_string();
    let path = directory.0.join("live.sqlite3");
    Journal::initialize(&path, 16).unwrap();
    let output = std::thread::scope(|scope| {
        let run = scope.spawn(|| {
            run_fixture_workflow(
                &directory,
                &config,
                &workflow,
                &[
                    "--resume",
                    "--event-journal",
                    path.to_str().unwrap(),
                    "--observe-workflow",
                ],
            )
        });
        let mut reader = Journal::open_reader(&path).unwrap();
        let deadline = Instant::now() + Duration::from_secs(12);
        let live = loop {
            let page = reader.replay(None, 16).unwrap();
            if !page.entries.is_empty() {
                break page.entries.len() == 1
                    && matches!(
                        page.entries[0].event.kind,
                        EventKind::WorkflowAttemptAccepted { .. }
                    )
                    && !run.is_finished();
            }
            if run.is_finished() || Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        assert!(
            live,
            "expected accepted attempt while inference was in progress"
        );
        run.join().unwrap()
    });
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let entries = Journal::open_reader(&path)
        .unwrap()
        .replay(None, 16)
        .unwrap()
        .entries;
    assert_eq!(entries.len(), 5);
    let EventKind::WorkflowAttemptAccepted {
        task_id: upstream,
        to_agent_id,
        dependencies,
        ..
    } = &entries[0].event.kind
    else {
        panic!("expected first acceptance")
    };
    assert!(dependencies.is_empty());
    assert_eq!(
        entries[1].event.kind,
        EventKind::WorkflowAttemptFinished {
            task_id: *upstream,
            state: TaskState::Completed
        }
    );
    let EventKind::WorkflowAttemptAccepted {
        task_id: downstream,
        dependencies,
        ..
    } = &entries[2].event.kind
    else {
        panic!("expected downstream acceptance")
    };
    assert_eq!(
        dependencies,
        &[WorkflowDependency {
            task_id: *upstream,
            agent_id: *to_agent_id
        }]
    );
    assert_eq!(
        entries[3].event.kind,
        EventKind::WorkflowAttemptFinished {
            task_id: *downstream,
            state: TaskState::Completed
        }
    );
    assert!(matches!(
        entries[4].event.kind,
        EventKind::TaskCompleted { .. }
    ));
    assert!(
        entries
            .iter()
            .all(|entry| entry.event.correlation_task_id == entries[4].event.correlation_task_id)
    );
    let serialized = serde_json::to_string(&entries).unwrap();
    assert!(!serialized.contains("fixture integrated answer"));
    assert!(!serialized.contains("Produce one short contribution"));
}

#[test]
fn completion_events_replay_across_cli_processes_without_payload_text() {
    let fixture = HttpFixture::new(false);
    let directory = ScratchDirectory::new();
    let config = runtime_config(&fixture, false, true)
        .replace("permissions = []", "permissions = [\"delegate\"]");
    let journal = directory.0.join("events.sqlite3");
    let path = journal.to_str().unwrap();
    let missing = run_fixture_agent(&directory, &config, &["--resume", "--event-journal", path]);
    assert!(!missing.status.success());
    assert!(fixture.captured().is_empty());
    assert!(!journal.exists());
    assert!(
        execute(&["events", "--journal", path, "init", "--retention", "2"])
            .status
            .success()
    );
    let agent = run_fixture_agent(&directory, &config, &["--resume", "--event-journal", path]);
    assert!(
        agent.status.success(),
        "{}",
        String::from_utf8_lossy(&agent.stderr)
    );
    let workflow = run_fixture_workflow(
        &directory,
        &config,
        &workflow_value().to_string(),
        &[
            "--resume",
            "--event-journal",
            path,
            "--scheduler-telemetry",
            directory.0.join("workflow-observation").to_str().unwrap(),
        ],
    );
    assert!(
        workflow.status.success(),
        "{}",
        String::from_utf8_lossy(&workflow.stderr)
    );
    let telemetry: vespera_core::SchedulerSample = serde_json::from_slice(
        &fs::read(directory.0.join("workflow-observation/scheduler.json")).unwrap(),
    )
    .unwrap();
    assert!(telemetry.finished && telemetry.queue_closed);
    assert_eq!(telemetry.provider_health.healthy, 1);
    let agent: serde_json::Value = serde_json::from_slice(&agent.stdout).unwrap();
    let workflow: serde_json::Value = serde_json::from_slice(&workflow.stdout).unwrap();
    let replay = execute(&["events", "--journal", path, "replay", "--limit", "1"]);
    assert!(replay.status.success());
    let page: serde_json::Value = serde_json::from_slice(&replay.stdout).unwrap();
    let first = &page["entries"][0]["event"];
    assert_eq!(first["kind"]["kind"], "inference_completed");
    assert_eq!(first["correlation_task_id"], agent["task_id"]);
    assert!(first.get("prompt").is_none());
    assert!(first.get("response").is_none());
    assert_eq!(page["next_cursor"], agent["event_publication"]["cursor"]);
    assert_eq!(page["has_more"], true);
    let cursor = format!(
        "{}:{}",
        page["next_cursor"]["journal_id"].as_str().unwrap(),
        page["next_cursor"]["sequence"].as_i64().unwrap()
    );
    let replay = execute(&["events", "--journal", path, "replay", "--after", &cursor]);
    assert!(replay.status.success());
    let page: serde_json::Value = serde_json::from_slice(&replay.stdout).unwrap();
    let second = &page["entries"][0]["event"];
    assert_eq!(second["kind"]["kind"], "task_completed");
    assert_eq!(second["correlation_task_id"], workflow["task_id"]);
    assert_eq!(page["next_cursor"], workflow["event_publication"]["cursor"]);
    assert_eq!(page["has_more"], false);
}

fn assert_terminal_event(path: &Path, report: &serde_json::Value, kind: &str) {
    let replay = execute(&["events", "--journal", path.to_str().unwrap(), "replay"]);
    assert!(replay.status.success());
    let page: serde_json::Value = serde_json::from_slice(&replay.stdout).unwrap();
    let entries = page["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry["cursor"], report["event_publication"]["cursor"]);
    let event = &entry["event"];
    assert_eq!(event["schema_version"], 2);
    assert_eq!(event["correlation_task_id"], report["task_id"]);
    assert_eq!(
        event["kind"],
        serde_json::json!({"kind": kind, "task_id": report["task_id"]})
    );
}

#[test]
fn event_follower_streams_new_commits_and_resumes_after_disconnect() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use vespera_core::{EventEnvelope, EventKind, TaskId};
    use vespera_events::Journal;

    let directory = ScratchDirectory::new();
    let path = directory.0.join("events.sqlite3");
    let mut journal = Journal::initialize(&path, 3).unwrap();
    let event = || {
        EventEnvelope::new(
            "fixture",
            0,
            EventKind::TaskCompleted {
                task_id: TaskId::new(),
            },
        )
        .unwrap()
    };
    let first = journal.append(&event()).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_vespera"))
        .args([
            "events",
            "--journal",
            path.to_str().unwrap(),
            "follow",
            "--poll-ms",
            "10",
            "--max-events",
            "2",
            "--idle-timeout-ms",
            "5000",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let line = receiver
        .recv_timeout(std::time::Duration::from_secs(6))
        .unwrap();
    let first_line: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(first_line["cursor"], serde_json::json!(first));
    let second = journal.append(&event()).unwrap();
    let line = receiver
        .recv_timeout(std::time::Duration::from_secs(6))
        .unwrap();
    let second_line: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(second_line["cursor"], serde_json::json!(second));
    assert!(child.wait().unwrap().success());
    reader.join().unwrap();
    assert!(receiver.try_recv().is_err());

    let third = journal.append(&event()).unwrap();
    let cursor = format!("{}:{}", second.journal_id, second.sequence);
    let resumed = execute(&[
        "events",
        "--journal",
        path.to_str().unwrap(),
        "follow",
        "--after",
        &cursor,
        "--max-events",
        "1",
        "--idle-timeout-ms",
        "100",
    ]);
    assert!(resumed.status.success());
    let entry: serde_json::Value = serde_json::from_slice(&resumed.stdout).unwrap();
    assert_eq!(entry["cursor"], serde_json::json!(third));
}

#[test]
fn event_follower_reports_expired_cursors_and_can_exit_on_idle() {
    use vespera_core::{EventEnvelope, EventKind, TaskId};
    use vespera_events::Journal;

    let directory = ScratchDirectory::new();
    let path = directory.0.join("events.sqlite3");
    let mut journal = Journal::initialize(&path, 1).unwrap();
    let journal_path = path.to_str().unwrap();
    let idle = execute(&[
        "events",
        "--journal",
        journal_path,
        "follow",
        "--idle-timeout-ms",
        "20",
    ]);
    assert!(idle.status.success());
    assert!(idle.stdout.is_empty());
    let empty_cursor = journal.replay(None, 1).unwrap().next_cursor;
    for _ in 0..2 {
        journal
            .append(
                &EventEnvelope::new(
                    "fixture",
                    0,
                    EventKind::TaskCompleted {
                        task_id: TaskId::new(),
                    },
                )
                .unwrap(),
            )
            .unwrap();
    }
    let cursor = format!("{}:{}", empty_cursor.journal_id, empty_cursor.sequence);
    let expired = execute(&[
        "events",
        "--journal",
        journal_path,
        "follow",
        "--after",
        &cursor,
        "--max-events",
        "1",
    ]);
    assert!(!expired.status.success());
    assert!(expired.stdout.is_empty());
    assert!(String::from_utf8_lossy(&expired.stderr).contains("cursor expired"));
    let retained = execute(&[
        "events",
        "--journal",
        journal_path,
        "follow",
        "--max-events",
        "1",
    ]);
    assert!(retained.status.success());
    assert!(String::from_utf8_lossy(&retained.stderr).contains("discarded through sequence 1"));
}

#[test]
fn workflow_cli_executes_dependencies_and_reports_provenance_and_cleanup() {
    for compatible in [false, true] {
        let fixture = HttpFixture::new(false);
        let directory = ScratchDirectory::new();
        let config = runtime_config(&fixture, compatible, true)
            .replace("permissions = []", "permissions = [\"delegate\"]");
        let output = run_fixture_workflow(
            &directory,
            &config,
            &workflow_value().to_string(),
            if compatible {
                &["--resume"]
            } else {
                &["--resume", "--unload-after"]
            },
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["workflow"]["task"]["state"], "completed");
        assert_eq!(report["workflow"]["selected_step"], "summarize");
        assert_eq!(report["workflow"]["attempts"].as_array().unwrap().len(), 2);
        assert_eq!(report["workflow"]["reserved_output_tokens"], 16);
        assert_eq!(
            report["workflow"]["attempts"][1]["result"]["agent_id"],
            AGENT
        );
        assert_eq!(report["cleanup"]["drained"], true);
        assert_eq!(
            report["cleanup"]["remaining_models"]
                .as_array()
                .unwrap()
                .len(),
            usize::from(compatible)
        );
        let requests = fixture.captured();
        let inference: Vec<serde_json::Value> = requests
            .iter()
            .filter(|r| matches!(r.path.as_str(), "/api/chat" | "/v1/chat/completions"))
            .map(|r| serde_json::from_str(&r.body).unwrap())
            .collect();
        assert_eq!(inference.len(), 2);
        let second_prompt: serde_json::Value =
            serde_json::from_str(inference[1]["messages"][0]["content"].as_str().unwrap()).unwrap();
        assert_eq!(
            second_prompt["untrusted_dependency_outputs"][0]["content"],
            "fixture integrated answer"
        );
        assert_eq!(
            fs::read_to_string(directory.0.join("test.toml")).unwrap(),
            config
        );
    }
}

#[test]
fn workflow_denied_activation_permissions_and_unknown_agents_never_contact_providers() {
    for case in [0, 1, 2] {
        let fixture = HttpFixture::new(false);
        let directory = ScratchDirectory::new();
        let mut config = runtime_config(&fixture, false, true);
        if case != 0 {
            config = config.replace("permissions = []", "permissions = [\"delegate\"]");
        }
        let mut workflow = workflow_value();
        if case == 2 {
            workflow["steps"][0]["candidates"] =
                serde_json::json!(["88888888-8888-4888-8888-888888888888"]);
        }
        let output = run_fixture_workflow(
            &directory,
            &config,
            &workflow.to_string(),
            if case == 1 { &[] } else { &["--resume"] },
        );
        assert!(!output.status.success());
        assert!(fixture.captured().is_empty());
    }
}

#[test]
fn malformed_oversized_and_invalid_workflow_files_fail_before_provider_initialization() {
    let fixture = HttpFixture::new(false);
    let directory = ScratchDirectory::new();
    let config = runtime_config(&fixture, false, true).replace(
        "request_timeout_seconds = 30",
        "request_timeout_seconds = 30\ncredential_env = \"VESPERA_MISSING_WORKFLOW_FIXTURE_TOKEN\"",
    );
    let mut invalid = workflow_value();
    invalid["steps"][0]["dependencies"] = serde_json::json!(["summarize"]);
    for input in [
        "private-untrusted-invalid-json".into(),
        " ".repeat(1_048_577),
        invalid.to_string(),
    ] {
        let output = run_fixture_workflow(&directory, &config, &input, &["--resume"]);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!error.contains("private-untrusted-invalid-json"));
        assert!(!error.contains("VESPERA_MISSING_WORKFLOW_FIXTURE_TOKEN"));
        assert!(fixture.captured().is_empty());
    }
}

#[test]
fn workflow_composition_does_not_initialize_a_delegation_only_coordinator_model() {
    let fixture = HttpFixture::new(false);
    let directory = ScratchDirectory::new();
    let mut config = runtime_config(&fixture, false, true);
    config.push_str(
        r#"
[[providers]]
id = "44444444-4444-4444-8444-444444444444"
name = "Delegation only"
kind = "custom"
endpoint = "http://127.0.0.1:1"
credential_env = "VESPERA_UNRELATED_WORKFLOW_UNSET_KEY"
[[models]]
id = "55555555-5555-4555-8555-555555555555"
provider_id = "44444444-4444-4444-8444-444444444444"
name = "Never activated"
identifier = "unrelated"
capabilities = ["text-generation"]
availability = "unknown"
[[agents]]
permissions = ["delegate"]
status = "idle"
[agents.identity]
id = "66666666-6666-4666-8666-666666666666"
name = "Coordinator"
role = "Delegate only"
[agents.model_policy]
kind = "fixed"
model_id = "55555555-5555-4555-8555-555555555555"
"#,
    );
    let mut workflow = workflow_value();
    workflow["coordinator"] = serde_json::json!("66666666-6666-4666-8666-666666666666");
    let output = run_fixture_workflow(
        &directory,
        &config,
        &workflow.to_string(),
        &["--resume", "--unload-after"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fixture
            .captured()
            .iter()
            .filter(|r| r.path == "/api/chat")
            .count(),
        2
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("VESPERA_UNRELATED_WORKFLOW_UNSET_KEY")
    );
}

#[test]
fn workflow_command_deadline_preserves_cancelled_history_and_prevents_next_step() {
    let fixture = HttpFixture::new(false);
    fixture.inference_delay_ms.store(1500, Ordering::SeqCst);
    let directory = ScratchDirectory::new();
    let config = runtime_config(&fixture, false, true)
        .replace("permissions = []", "permissions = [\"delegate\"]");
    let journal = directory.0.join("events.sqlite3");
    vespera_events::Journal::initialize(&journal, 8).unwrap();
    let output = run_fixture_workflow(
        &directory,
        &config,
        &workflow_value().to_string(),
        &[
            "--resume",
            "--timeout-seconds",
            "1",
            "--event-journal",
            journal.to_str().unwrap(),
            "--observe-workflow",
        ],
    );
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["workflow"]["task"]["state"], "cancelled");
    let entries = vespera_events::Journal::open_reader(&journal)
        .unwrap()
        .replay(None, 8)
        .unwrap()
        .entries;
    assert_eq!(entries.len(), 3);
    let attempt = report["workflow"]["attempts"][0]["task_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        matches!(entries[0].event.kind, vespera_core::EventKind::WorkflowAttemptAccepted { task_id, .. } if task_id == attempt)
    );
    assert_eq!(
        entries[1].event.kind,
        vespera_core::EventKind::WorkflowAttemptFinished {
            task_id: attempt,
            state: vespera_core::TaskState::Cancelled
        }
    );
    let root = report["task_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        entries[2].event.kind,
        vespera_core::EventKind::TaskCancelled { task_id: root }
    );
    assert_eq!(report["event_publication"]["cursor"]["sequence"], 3);
    assert_eq!(report["workflow"]["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(report["workflow"]["attempts"][0]["task_state"], "cancelled");
    assert_eq!(report["cleanup"]["drained"], true);
    assert_eq!(
        fixture
            .captured()
            .iter()
            .filter(|r| r.path == "/api/chat")
            .count(),
        1
    );
}

#[test]
fn ollama_cli_executes_the_complete_path_and_confirms_opt_in_unload() {
    let fixture = HttpFixture::new(false);
    let directory = ScratchDirectory::new();
    let mut config = runtime_config(&fixture, false, true);
    // Unrelated missing credentials/unsupported adapters must not initialize for this fixed agent.
    config.push_str("\n[[providers]]\nid = \"44444444-4444-4444-8444-444444444444\"\nname = \"Unrelated\"\nkind = \"custom\"\nendpoint = \"http://127.0.0.1:1\"\ncredential_env = \"VESPERA_UNRELATED_UNSET_FIXTURE_KEY\"\n");
    let output = run_fixture_agent(&directory, &config, &["--resume", "--unload-after"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["task_state"], "completed");
    assert_eq!(
        report["execution"]["response"]["content"],
        "fixture integrated answer"
    );
    assert_eq!(report["cleanup"]["drained"], true);
    assert_eq!(
        report["cleanup"]["remaining_models"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        report["cleanup"]["unloaded_models"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let requests = fixture.captured();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.path.as_str())
            .collect::<Vec<_>>(),
        [
            "/api/version",
            "/api/generate",
            "/api/chat",
            "/api/generate"
        ]
    );
    let preload: serde_json::Value = serde_json::from_str(&requests[1].body).unwrap();
    assert_eq!(preload["options"]["num_ctx"], 512);
    let inference: serde_json::Value = serde_json::from_str(&requests[2].body).unwrap();
    assert_eq!(inference["model"], "fixture-model");
    assert_eq!(
        inference["messages"][0]["content"],
        "bounded integrated task"
    );
    assert_eq!(inference["options"]["num_predict"], 32);
    assert_eq!(inference["options"]["num_ctx"], 512);
    let unload: serde_json::Value = serde_json::from_str(&requests[3].body).unwrap();
    assert_eq!(unload["keep_alive"], 0);
    assert!(unload.get("options").is_none());
    assert_eq!(
        fs::read_to_string(directory.0.join("test.toml")).unwrap(),
        config
    );
}

#[test]
fn unusable_model_context_is_rejected_before_preload() {
    let fixture = HttpFixture::new(false);
    let directory = ScratchDirectory::new();
    let config = runtime_config(&fixture, false, true)
        .replace("context_window_tokens = 512", "context_window_tokens = 16");
    let journal = directory.0.join("events.sqlite3");
    vespera_events::Journal::initialize(&journal, 2).unwrap();
    let output = run_fixture_agent(
        &directory,
        &config,
        &["--resume", "--event-journal", journal.to_str().unwrap()],
    );
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["task_state"], "failed");
    assert_terminal_event(&journal, &report, "task_failed");
    assert!(
        report["cleanup"]["remaining_models"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fixture
            .captured()
            .iter()
            .map(|r| r.path.as_str())
            .collect::<Vec<_>>(),
        ["/api/version"]
    );
}

#[test]
fn compatible_cli_retains_honest_residency_and_rejects_unconfirmed_unload() {
    for unload in [false, true] {
        let fixture = HttpFixture::new(false);
        let directory = ScratchDirectory::new();
        let mut extra = vec!["--resume"];
        if unload {
            extra.push("--unload-after");
        }
        let output = run_fixture_agent(&directory, &runtime_config(&fixture, true, true), &extra);
        assert_eq!(output.status.success(), !unload);
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["task_state"], "completed");
        assert_eq!(
            report["execution"]["response"]["content"],
            "fixture integrated answer"
        );
        assert_eq!(
            report["cleanup"]["remaining_models"][0]["state"],
            "ProviderManaged"
        );
        assert_eq!(
            report["cleanup"]["unload_errors"]
                .as_object()
                .unwrap()
                .len(),
            usize::from(unload)
        );
        assert_eq!(
            fixture
                .captured()
                .iter()
                .map(|request| request.path.as_str())
                .collect::<Vec<_>>(),
            ["/v1/models", "/v1/chat/completions"]
        );
    }
}

#[test]
fn suspended_and_unknown_cost_agents_never_trigger_model_activation() {
    for resume in [false, true] {
        let fixture = HttpFixture::new(false);
        let directory = ScratchDirectory::new();
        let extra = if resume { vec!["--resume"] } else { vec![] };
        let output = run_fixture_agent(&directory, &runtime_config(&fixture, false, false), &extra);
        assert!(!output.status.success());
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(report["execution"].is_null());
        assert_eq!(
            report["cleanup"]["remaining_models"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        if resume {
            assert_eq!(report["task_state"], "failed");
            assert!(
                report["error"]
                    .as_str()
                    .unwrap()
                    .contains("unknown resource estimates")
            );
            assert_eq!(
                fixture
                    .captured()
                    .iter()
                    .map(|request| request.path.as_str())
                    .collect::<Vec<_>>(),
                ["/api/version"]
            );
        } else {
            assert!(fixture.captured().is_empty());
        }
    }
}

#[test]
fn offline_model_and_agent_inventory_do_not_resolve_credentials_or_connect() {
    let fixture = HttpFixture::new(false);
    let directory = ScratchDirectory::new();
    let config = runtime_config(&fixture, false, true).replace(
        "request_timeout_seconds = 30",
        "request_timeout_seconds = 30\ncredential_env = \"VESPERA_UNSET_INVENTORY_FIXTURE_KEY\"",
    );
    let path = directory.write_config(&config);
    for command in ["models", "agents"] {
        let output = execute(&[command, "--config", path.to_str().unwrap(), "--json"]);
        assert!(output.status.success());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(
            !String::from_utf8_lossy(&output.stdout)
                .contains("VESPERA_UNSET_INVENTORY_FIXTURE_KEY")
        );
    }
    assert!(fixture.captured().is_empty());
}

#[test]
fn provider_health_watch_reports_timeout_then_failure_and_recovery() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    let fixture = HttpFixture::new(true);
    fixture.hold_health.store(true, Ordering::SeqCst);
    let scratch = ScratchDirectory::new();
    let config = scratch.write_config(&runtime_config(&fixture, false, true));
    let mut child = Command::new(env!("CARGO_BIN_EXE_vespera"))
        .args([
            "provider-health",
            "--config",
            config.to_str().unwrap(),
            "--watch-seconds",
            "1",
            "--timeout-seconds",
            "1",
            "--samples",
            "3",
            "--json",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let first = lines.next();
    fixture.hold_health.store(false, Ordering::SeqCst);
    let second = lines.next();
    fixture.deny_health.store(false, Ordering::SeqCst);
    let third = lines.next();
    assert!(lines.next().is_none());
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for (index, (line, timeout, state)) in [
        (first, true, "unknown"),
        (second, false, "unreachable"),
        (third, false, "healthy"),
    ]
    .into_iter()
    .enumerate()
    {
        let line = line.expect("health round").unwrap();
        let report: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(report["round"], index + 1);
        assert_eq!(report["timed_out"], timeout);
        assert_eq!(report["providers"][0]["state"], state);
        assert!(!line.contains("fixture-private-error-body"));
        assert!(!line.contains("http://"));
    }
    let requests = fixture.captured();
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|request| request.path == "/api/version")
    );
}

#[test]
fn provider_health_reports_actual_failures_without_echoing_error_bodies() {
    for denied in [false, true] {
        let fixture = HttpFixture::new(denied);
        let directory = ScratchDirectory::new();
        // Health inspection must work independently of configured operational agent status.
        let config = runtime_config(&fixture, false, true)
            .replace("status = \"suspended\"", "status = \"running\"");
        let path = directory.write_config(&config);
        let output = execute(&[
            "provider-health",
            "--config",
            path.to_str().unwrap(),
            "--json",
        ]);
        assert_eq!(
            output.status.success(),
            !denied,
            "stdout={} stderr={} paths={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            fixture
                .captured()
                .iter()
                .map(|request| &request.path)
                .collect::<Vec<_>>()
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report[0]["state"],
            if denied { "unreachable" } else { "healthy" }
        );
        assert!(report[0]["checked_age_ms"].is_number());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("fixture-private-error-body"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("fixture-private-error-body"));
        assert_eq!(fixture.captured().len(), 1);
    }
}

#[test]
fn provider_credentials_are_resolved_only_in_child_process_and_never_reported() {
    let fixture = HttpFixture::new(false);
    let directory = ScratchDirectory::new();
    let config = runtime_config(&fixture, true, true).replace(
        "request_timeout_seconds = 30",
        "request_timeout_seconds = 30\ncredential_env = \"VESPERA_CLI_FIXTURE_CREDENTIAL\"",
    );
    let path = directory.write_config(&config);
    let output = Command::new(env!("CARGO_BIN_EXE_vespera"))
        .args([
            "provider-health",
            "--config",
            path.to_str().unwrap(),
            "--json",
        ])
        .env(
            "VESPERA_CLI_FIXTURE_CREDENTIAL",
            "nonfunctional-fixture-credential",
        )
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        fixture.captured()[0]
            .headers
            .to_ascii_lowercase()
            .contains("authorization: bearer nonfunctional-fixture-credential")
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("nonfunctional-fixture-credential"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("nonfunctional-fixture-credential"));
}

#[test]
fn cli_deadline_cancels_inference_and_reports_retained_provider_reservation() {
    let fixture = HttpFixture::new(false);
    fixture.inference_delay_ms.store(1500, Ordering::SeqCst);
    let directory = ScratchDirectory::new();
    let journal = directory.0.join("events.sqlite3");
    vespera_events::Journal::initialize(&journal, 2).unwrap();
    let output = run_fixture_agent(
        &directory,
        &runtime_config(&fixture, true, true),
        &[
            "--resume",
            "--timeout-seconds",
            "1",
            "--event-journal",
            journal.to_str().unwrap(),
        ],
    );
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["task_state"], "cancelled");
    assert_terminal_event(&journal, &report, "task_cancelled");
    assert!(report["execution"].is_null());
    assert!(
        report["error"]
            .as_str()
            .unwrap()
            .contains("deadline exceeded")
    );
    assert_eq!(report["cleanup"]["drained"], true);
    assert_eq!(
        report["cleanup"]["remaining_models"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fixture
            .captured()
            .iter()
            .map(|request| request.path.as_str())
            .collect::<Vec<_>>(),
        ["/v1/models", "/v1/chat/completions"]
    );
}

#[test]
fn invalid_prompt_files_are_rejected_before_provider_initialization_or_requests() {
    let fixture = HttpFixture::new(false);
    let directory = ScratchDirectory::new();
    let path = directory.write_config(&runtime_config(&fixture, false, true));
    let prompt = directory.0.join("prompt.txt");
    for bytes in [b"   \n".to_vec(), vec![b'x'; 65537], vec![0xff]] {
        fs::write(&prompt, bytes).unwrap();
        let output = execute(&[
            "run-agent",
            "--config",
            path.to_str().unwrap(),
            "--agent",
            AGENT,
            "--prompt-file",
            prompt.to_str().unwrap(),
            "--resume",
            "--json",
        ]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    assert!(fixture.captured().is_empty());
}
