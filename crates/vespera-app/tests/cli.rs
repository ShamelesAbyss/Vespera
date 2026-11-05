use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
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
fn unimplemented_runtime_commands_are_not_advertised_or_faked() {
    let output = execute(&["run-agent"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"));
    let help = execute(&["--help"]);
    assert!(help.status.success());
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("validate-config"));
    assert!(text.contains("doctor"));
    assert!(!text.contains("connect-provider"));
}

#[test]
fn version_is_available() {
    let output = execute(&["--version"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("vespera 0.1.0"));
}
