//! Real subprocesses, persistent SQLite, offline declarations and explicit grants.
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use vespera_core::MemoryId;
const AGENT: &str = "33333333-3333-4333-8333-333333333333";
const OTHER: &str = "88888888-8888-4888-8888-888888888888";
const ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
struct Fixture {
    dir: PathBuf,
    config: PathBuf,
    database: PathBuf,
}
impl Fixture {
    fn new(grants: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vespera-memory-cli-{}", MemoryId::new()));
        fs::create_dir(&dir).unwrap();
        let config = dir.join("private.toml");
        let database = dir.join("database").join("memory.sqlite3");
        let sample = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/local.toml"),
        )
        .unwrap()
        .replace("permissions = []", &format!("permissions = [{grants}]"))
        .replace("kind = \"ollama\"", "kind = \"custom\"")
        .replace(
            "request_timeout_seconds = 30",
            "request_timeout_seconds = 30\ncredential_env = \"VESPERA_MEMORY_CLI_UNSET_TOKEN\"",
        );
        fs::write(&config, sample).unwrap();
        Self {
            dir,
            config,
            database,
        }
    }
    fn run(&self, agent: &str, action: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_vespera"))
            .args([
                "memory",
                "--config",
                self.config.to_str().unwrap(),
                "--database",
                self.database.to_str().unwrap(),
                "--agent",
                agent,
                "--json",
            ])
            .args(action)
            .env_remove("VESPERA_MEMORY_CLI_UNSET_TOKEN")
            .output()
            .unwrap()
    }
    fn ok(&self, agent: &str, action: &[&str]) -> Value {
        let output = self.run(agent, action);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn input(&self, name: &str, content: &[u8]) -> PathBuf {
        let path = self.dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }
    fn add(&self, agent: &str, id: &str, text: &str) -> Value {
        let file = self.input("content.txt", text.as_bytes());
        self.ok(
            agent,
            &[
                "add",
                "--content-file",
                file.to_str().unwrap(),
                "--id",
                id,
                "--created-at-unix-ms",
                "42",
            ],
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn memory_admin_copies_and_restores_a_snapshot_without_replacing_files() {
    let mut fixture = Fixture::new("\"memory_read\",\"memory_write\"");
    fixture.ok(AGENT, &["init"]);
    fixture.add(AGENT, ID, "retained backup observation");
    let backup = fixture.dir.join("backup with spaces.sqlite3");
    let restored = fixture.dir.join("restored.sqlite3");
    let admin = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_vespera"))
            .arg("memory-admin")
            .args(args)
            .output()
            .unwrap()
    };
    let backup_args = [
        "backup",
        "--database",
        fixture.database.to_str().unwrap(),
        "--output",
        backup.to_str().unwrap(),
    ];
    let output = admin(&backup_args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["verified"],
        true
    );
    assert!(!admin(&backup_args).status.success());
    let restore_args = [
        "restore",
        "--backup",
        backup.to_str().unwrap(),
        "--database",
        restored.to_str().unwrap(),
    ];
    let output = admin(&restore_args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!admin(&restore_args).status.success());
    let original_audit = vespera_memory::read_memory_audit(&fixture.database, None, 8).unwrap();
    let restored_audit = vespera_memory::read_memory_audit(&restored, None, 8).unwrap();
    assert_eq!(original_audit.events, restored_audit.events);
    assert!(matches!(
        vespera_memory::read_memory_audit(&restored, original_audit.next_cursor, 8),
        Err(vespera_memory::StorageError::AuditForeignCursor)
    ));
    fixture.database = restored;
    assert_eq!(
        fixture.ok(AGENT, &["get", "--id", ID])["record"]["record"]["content"],
        "retained backup observation"
    );
}

#[test]
fn memory_mutation_audit_recovers_journal_failure_and_rolls_back_with_memory() {
    use vespera_core::EventKind;
    use vespera_events::Journal;

    let fixture = Fixture::new("\"memory_read\",\"memory_write\"");
    fixture.ok(AGENT, &["init"]);
    let path = fixture.dir.join("events.sqlite3");
    let journal = path.to_str().unwrap();
    drop(Journal::initialize(&path, 8).unwrap());
    let failure = rusqlite::Connection::open(&path).unwrap();
    failure
        .execute_batch(
            "CREATE TRIGGER reject_fixture_event BEFORE INSERT ON journal_events
         BEGIN SELECT RAISE(ABORT, 'fixture journal failure'); END;",
        )
        .unwrap();
    let content = fixture.input("private.txt", b"private memory fixture");
    let insert = |id| {
        fixture.run(
            AGENT,
            &[
                "--event-journal",
                journal,
                "add",
                "--content-file",
                content.to_str().unwrap(),
                "--id",
                id,
            ],
        )
    };
    let failed = insert(ID);
    assert!(!failed.status.success());
    let report: Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert_eq!(report["record"]["record"]["id"], ID);
    assert!(report["event_publication"]["error"].is_string());
    failure
        .execute_batch("DROP TRIGGER reject_fixture_event")
        .unwrap();
    let audit_failure = rusqlite::Connection::open(&fixture.database).unwrap();
    audit_failure
        .execute_batch(
            "CREATE TRIGGER reject_fixture_memory_audit BEFORE INSERT ON memory_audit
         BEGIN SELECT RAISE(ABORT, 'fixture audit failure'); END;",
        )
        .unwrap();
    let other_id = MemoryId::new().to_string();
    assert!(!insert(&other_id).status.success());
    assert!(
        !fixture
            .run(AGENT, &["get", "--id", &other_id])
            .status
            .success()
    );
    audit_failure
        .execute_batch("DROP TRIGGER reject_fixture_memory_audit")
        .unwrap();

    let recover = |after: Option<&str>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_vespera"));
        command
            .args([
                "events",
                "--journal",
                journal,
                "recover-memory",
                "--database",
            ])
            .arg(&fixture.database)
            .args(["--limit", "1"]);
        if let Some(cursor) = after {
            command.args(["--after", cursor]);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let first = recover(None);
    assert_eq!(first["submitted_events"], 1);
    assert_eq!(first, recover(None));
    assert!(
        fixture
            .run(AGENT, &["--event-journal", journal, "delete", "--id", ID])
            .status
            .success()
    );
    assert!(
        fixture
            .run(AGENT, &["--event-journal", journal, "delete", "--id", ID])
            .status
            .success()
    );
    let second = recover(first["next_source_cursor"].as_str());
    assert_eq!(second["submitted_events"], 1);
    assert_eq!(
        recover(second["next_source_cursor"].as_str())["submitted_events"],
        0
    );
    let entries = Journal::open_reader(&path)
        .unwrap()
        .replay(None, 8)
        .unwrap()
        .entries;
    assert_eq!(entries.len(), 2);
    assert!(
        matches!(entries[0].event.kind, EventKind::AgentMemoryUpdated { memory_id, .. } if memory_id.to_string() == ID)
    );
    assert!(
        matches!(entries[1].event.kind, EventKind::AgentMemoryDeleted { memory_id, .. } if memory_id.to_string() == ID)
    );
    assert!(
        !serde_json::to_string(&entries)
            .unwrap()
            .contains("private memory fixture")
    );
}

#[test]
fn memory_commands_persist_retrieve_filter_and_search_without_provider_initialization() {
    let fixture = Fixture::new("\"memory_read\",\"memory_write\"");
    let original = fs::read(&fixture.config).unwrap();
    assert_eq!(fixture.ok(AGENT, &["init"])["database_ready"], true);
    let content = "amber observation é";
    let added = fixture.add(AGENT, ID, content);
    assert_eq!(added["record"]["record"]["id"], ID);
    assert_eq!(added["record"]["record"]["created_at_unix_ms"], 42);
    assert_eq!(added["record"]["sequence"], 1);
    let before = fs::read(&fixture.database).unwrap();
    let get = fixture.ok(AGENT, &["get", "--id", ID]);
    assert_eq!(get["record"]["record"]["content"], content);
    let page = fixture.ok(
        AGENT,
        &[
            "list",
            "--layer",
            "observation",
            "--created-since-unix-ms",
            "42",
            "--created-before-unix-ms",
            "43",
            "--limit",
            "1",
        ],
    );
    assert_eq!(page["page"]["records"].as_array().unwrap().len(), 1);
    assert!(page["page"]["next_after_sequence"].is_null());
    let query = fixture.input("query.txt", b"amber");
    let search = fixture.ok(AGENT, &["search", "--query-file", query.to_str().unwrap()]);
    assert_eq!(search["page"]["records"].as_array().unwrap().len(), 1);
    let usage = fixture.ok(AGENT, &["usage"]);
    assert_eq!(usage["usage"]["records"], 1);
    assert_eq!(usage["usage"]["content_bytes"], content.len());
    assert_eq!(fs::read(&fixture.database).unwrap(), before);
    assert_eq!(fs::read(&fixture.config).unwrap(), original);
    // Success with a custom adapter and deliberately absent credentials proves no provider construction.
    assert!(!String::from_utf8_lossy(&original).contains("status = \"idle\""));
}

#[test]
fn namespace_queries_cannot_retrieve_other_agent_content() {
    let fixture = Fixture::new("\"memory_read\",\"memory_write\"");
    let mut config = fs::read_to_string(&fixture.config).unwrap();
    config.push_str(&format!("\n[[agents]]\npermissions = [\"memory_read\",\"memory_write\"]\nstatus = \"suspended\"\n[agents.identity]\nid = \"{OTHER}\"\nname = \"Archivist\"\nrole = \"Separate namespace\"\n[agents.model_policy]\nkind = \"fixed\"\nmodel_id = \"22222222-2222-4222-8222-222222222222\"\n"));
    fs::write(&fixture.config, config).unwrap();
    fixture.ok(AGENT, &["init"]);
    fixture.add(AGENT, ID, "private amber");
    let output = fixture.run(OTHER, &["get", "--id", ID]);
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value["record"].is_null());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private amber"));
    let other_id = MemoryId::new().to_string();
    assert_eq!(
        fixture.add(OTHER, &other_id, "private violet")["record"]["sequence"],
        1
    );
    let page = fixture.ok(AGENT, &["list"]);
    assert_eq!(page["page"]["records"].as_array().unwrap().len(), 1);
    assert_eq!(
        page["page"]["records"][0]["record"]["content"],
        "private amber"
    );
}

#[test]
fn missing_databases_and_permission_failures_do_not_create_storage_or_read_input() {
    let denied = Fixture::new("");
    let output = denied.run(AGENT, &["init"]);
    assert!(!output.status.success());
    assert!(!denied.database.parent().unwrap().exists());
    let output = denied.run(
        AGENT,
        &["add", "--content-file", "missing-private-input.txt"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("access denied"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("missing-private-input"));
    let read_only = Fixture::new("\"memory_read\"");
    assert!(!read_only.run(AGENT, &["init"]).status.success());
    assert!(!read_only.run(AGENT, &["list"]).status.success());
    assert!(!read_only.database.exists());
    let allowed = Fixture::new("\"memory_read\",\"memory_write\"");
    for action in [&["list"][..], &["usage"][..], &["get", "--id", ID][..]] {
        assert!(!allowed.run(AGENT, action).status.success());
        assert!(!allowed.database.parent().unwrap().exists());
    }
    let file = allowed.input("valid.txt", b"valid input");
    assert!(
        !allowed
            .run(AGENT, &["add", "--content-file", file.to_str().unwrap()])
            .status
            .success()
    );
    assert!(!allowed.database.exists());
    assert!(!allowed.run(OTHER, &["init"]).status.success());
    assert!(!allowed.database.exists());
}

#[test]
fn consolidation_and_explicit_deletion_preserve_lineage_and_search_consistency() {
    let fixture = Fixture::new("\"memory_read\",\"memory_write\"");
    fixture.ok(AGENT, &["init"]);
    fixture.add(AGENT, ID, "source amber");
    let summary_file = fixture.input("summary.txt", b"derived violet");
    let summary_id = MemoryId::new().to_string();
    let summary = fixture.ok(
        AGENT,
        &[
            "consolidate",
            "--content-file",
            summary_file.to_str().unwrap(),
            "--id",
            &summary_id,
            "--source",
            ID,
            "--method",
            "manual-summary",
        ],
    );
    assert_eq!(summary["record"]["source_memories"][0], ID);
    assert_eq!(summary["record"]["origin"], "derived");
    assert_eq!(summary["record"]["record"]["layer"], "long_term");
    assert!(!fixture.run(AGENT, &["delete", "--id", ID]).status.success());
    assert_eq!(
        fixture.ok(AGENT, &["get", "--id", ID])["record"]["record"]["content"],
        "source amber"
    );
    assert_eq!(
        fixture.ok(AGENT, &["delete", "--id", &summary_id])["memory_absent"],
        summary_id
    );
    let query = fixture.input("query.txt", b"violet");
    assert!(
        fixture.ok(AGENT, &["search", "--query-file", query.to_str().unwrap()])["page"]["records"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    fixture.ok(AGENT, &["delete", "--id", ID]);
    fixture.ok(AGENT, &["delete", "--id", ID]);
    assert_eq!(fixture.ok(AGENT, &["usage"])["usage"]["records"], 0);
}

#[test]
fn invalid_content_timestamps_and_unsupported_origins_fail_without_private_excerpts() {
    let fixture = Fixture::new("\"memory_read\",\"memory_write\"");
    for contents in [vec![], b"   ".to_vec(), vec![b'x'; 65_537], vec![0xff]] {
        let path = fixture.input("private-content.txt", &contents);
        let output = fixture.run(AGENT, &["add", "--content-file", path.to_str().unwrap()]);
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("private-content.txt"));
        assert!(!fixture.database.exists());
    }
    let path = fixture.input("valid.txt", b"bounded");
    for extra in [
        &["--created-at-unix-ms", "18446744073709551615"][..],
        &["--origin", "generated"][..],
        &["--layer", "identity"][..],
    ] {
        let mut args = vec!["add", "--content-file", path.to_str().unwrap()];
        args.extend_from_slice(extra);
        assert!(!fixture.run(AGENT, &args).status.success());
        assert!(!fixture.database.exists());
    }
}

#[test]
fn read_commands_reject_empty_or_foreign_databases_without_changing_them() {
    let fixture = Fixture::new("\"memory_read\",\"memory_write\"");
    fs::create_dir_all(fixture.database.parent().unwrap()).unwrap();
    for bytes in [Vec::new(), b"unrelated private file".to_vec()] {
        fs::write(&fixture.database, &bytes).unwrap();
        let output = fixture.run(AGENT, &["list"]);
        assert!(!output.status.success());
        assert_eq!(fs::read(&fixture.database).unwrap(), bytes);
        assert!(!String::from_utf8_lossy(&output.stderr).contains("unrelated private file"));
    }
}

#[test]
fn configured_model_replacement_does_not_move_durable_memory() {
    let fixture = Fixture::new("\"memory_read\",\"memory_write\"");
    fixture.ok(AGENT, &["init"]);
    fixture.add(AGENT, ID, "model independent");
    let mut config = fs::read_to_string(&fixture.config).unwrap().replace(
        "model_id = \"22222222-2222-4222-8222-222222222222\"",
        "model_id = \"99999999-9999-4999-8999-999999999999\"",
    );
    config.push_str("\n[[models]]\nid = \"99999999-9999-4999-8999-999999999999\"\nprovider_id = \"11111111-1111-4111-8111-111111111111\"\nname = \"Replacement\"\nidentifier = \"replacement\"\ncapabilities = [\"text-generation\"]\navailability = \"unknown\"\n");
    fs::write(&fixture.config, config).unwrap();
    assert_eq!(
        fixture.ok(AGENT, &["get", "--id", ID])["record"]["record"]["content"],
        "model independent"
    );
}

#[test]
fn help_exposes_all_actual_memory_commands_and_bounded_flags() {
    let fixture = Fixture::new("");
    let output = fixture.run(AGENT, &["--help"]);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    for command in [
        "init",
        "add",
        "get",
        "list",
        "search",
        "usage",
        "consolidate",
        "delete",
    ] {
        assert!(text.contains(command));
    }
    let output = fixture.run(AGENT, &["search", "--help"]);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    for flag in [
        "--query-file",
        "--layer",
        "--after-sequence",
        "--limit",
        "--max-content-bytes",
        "--created-since-unix-ms",
        "--created-before-unix-ms",
    ] {
        assert!(text.contains(flag));
    }
    assert!(
        !fixture
            .run(AGENT, &["list", "--limit", "0"])
            .status
            .success()
    );
    assert!(!fixture.run(AGENT, &["get"]).status.success());
    assert!(!fixture.run(AGENT, &["consolidate"]).status.success());
}
