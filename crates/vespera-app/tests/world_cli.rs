use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use vespera_core::TaskId;

struct Database(PathBuf);

impl Database {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("vespera-world-{}.sqlite3", TaskId::new())))
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn world(database: &Database, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vespera"))
        .args(["world", "--database"])
        .arg(&database.0)
        .args(arguments)
        .output()
        .unwrap()
}

fn json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn world_advance_event_recovers_after_journal_failure_and_rolls_back_with_world() {
    use vespera_core::EventKind;
    use vespera_events::Journal;
    use vespera_world::WorldStore;

    let database = Database::new();
    let journal = Database::new();
    let journal_path = journal.0.to_str().unwrap();
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/world.example.json");
    json(world(
        &database,
        &["init", "--spec", spec.to_str().unwrap()],
    ));
    assert!(WorldStore::advance_event(&database.0, 0).unwrap().is_none());
    drop(Journal::initialize(&journal.0, 8).unwrap());
    let injector = rusqlite::Connection::open(&journal.0).unwrap();
    injector
        .execute_batch(
            "CREATE TRIGGER reject_fixture_event BEFORE INSERT ON journal_events
         BEGIN SELECT RAISE(ABORT, 'fixture journal failure'); END;",
        )
        .unwrap();
    let failed = world(
        &database,
        &["advance", "--ticks", "2", "--event-journal", journal_path],
    );
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("world revision 2 committed"));
    let committed: Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert_eq!(committed["tick"], 2);
    assert_eq!(json(world(&database, &["inspect"])), committed);
    let mut reader = Journal::open_reader(&journal.0).unwrap();
    assert!(reader.replay(None, 8).unwrap().entries.is_empty());
    injector
        .execute_batch("DROP TRIGGER reject_fixture_event")
        .unwrap();
    let recover = |revision| {
        Command::new(env!("CARGO_BIN_EXE_vespera"))
            .args([
                "events",
                "--journal",
                journal_path,
                "recover-world-advance",
                "--database",
            ])
            .arg(&database.0)
            .args(["--revision", revision])
            .output()
            .unwrap()
    };
    assert_eq!(json(recover("2")), json(recover("2")));
    assert!(!recover("1").status.success());
    assert_eq!(
        json(world(
            &database,
            &["advance", "--event-journal", journal_path]
        ))["tick"],
        3
    );
    let entries = reader.replay(None, 8).unwrap().entries;
    assert_eq!(entries.len(), 2);
    let mut identities = Vec::new();
    for (revision, entry) in [2, 3].into_iter().zip(entries) {
        assert_eq!(
            WorldStore::advance_event(&database.0, revision).unwrap(),
            Some(entry.event.clone())
        );
        let EventKind::SimulationAdvanced {
            simulation_id,
            tick,
        } = entry.event.kind
        else {
            panic!("expected simulation advance")
        };
        assert_eq!(tick, revision);
        assert!(entry.event.correlation_task_id.is_none());
        identities.push(simulation_id);
    }
    assert_eq!(identities[0], identities[1]);

    let world_injector = rusqlite::Connection::open(&database.0).unwrap();
    world_injector
        .execute_batch(
            "CREATE TRIGGER reject_fixture_world BEFORE UPDATE ON world_state
         BEGIN SELECT RAISE(ABORT, 'fixture world failure'); END;",
        )
        .unwrap();
    assert!(!world(&database, &["advance"]).status.success());
    assert_eq!(json(world(&database, &["inspect"]))["revision"], 3);
    assert!(WorldStore::advance_event(&database.0, 4).unwrap().is_none());
}

#[test]
fn replay_retention_keeps_a_deterministic_checkpoint_and_labels_older_gaps() {
    use vespera_world::{ReplayDirection, World, WorldStore};

    let database = Database::new();
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/world.example.json");
    let spec = serde_json::from_slice(&std::fs::read(spec).unwrap()).unwrap();
    let mut existing = World::new(spec).unwrap();
    existing.advance().unwrap();
    WorldStore::initialize(&database.0, &existing).unwrap();
    assert!(
        !world(&database, &["replay", "--revision", "0"])
            .status
            .success()
    );
    let mut earliest = Value::Null;
    for index in 0..5 {
        let state = json(world(&database, &["advance", "--ticks", "1000"]));
        if index == 0 {
            earliest = state;
        }
    }
    let boundary = json(world(&database, &["replay", "--revision", "1001"]));
    assert_eq!(boundary["earliest_revision"], 1001);
    assert_eq!(boundary["basis_revision"], 1001);
    assert_eq!(boundary["latest_revision"], 5001);
    assert_eq!(boundary["world"], earliest);
    assert!(
        !world(&database, &["replay", "--revision", "1"])
            .status
            .success()
    );
    assert!(
        WorldStore::replay_neighbor(&database.0, 1001, ReplayDirection::Previous)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        WorldStore::replay_neighbor(&database.0, 1, ReplayDirection::Next)
            .unwrap()
            .unwrap()
            .world
            .revision(),
        1001
    );
    assert_eq!(
        WorldStore::replay_neighbor(&database.0, 5001, ReplayDirection::Previous)
            .unwrap()
            .unwrap()
            .world
            .revision(),
        4001
    );
    let current = json(world(&database, &["inspect"]));
    assert_eq!(json(world(&database, &["replay"]))["world"], current);
}

#[test]
fn persists_deterministic_world_and_fences_stale_resident_controls() {
    let first = Database::new();
    let second = Database::new();
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/world.example.json");
    let spec = spec.to_str().unwrap();
    let init = ["init", "--spec", spec];
    assert_eq!(json(world(&first, &init)), json(world(&second, &init)));
    assert!(!world(&first, &init).status.success());

    let first_snapshot = json(world(&first, &["advance", "--ticks", "60"]));
    assert_eq!(
        json(world(&first, &["replay", "--revision", "60"]))["world"],
        first_snapshot
    );
    let between = world(&first, &["replay", "--revision", "20"]);
    assert!(!between.status.success());
    assert!(String::from_utf8_lossy(&between.stderr).contains("not a retained commit"));
    for _ in 0..3 {
        json(world(&second, &["advance", "--ticks", "20"]));
    }
    assert_eq!(first_snapshot, json(world(&second, &["inspect"])));
    assert_eq!(first_snapshot["tick"], 60);
    assert!(first_snapshot["shelters"].as_u64().unwrap() > 0);
    let resident = first_snapshot["residents"][0]["id"].as_str().unwrap();
    assert!(first_snapshot["residents"][0].get("memory").is_none());
    assert!(
        !first_snapshot["residents"][0]["relationships"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    let memories = json(world(&first, &["memory", "--resident", resident]));
    assert_eq!(
        memories,
        json(world(&second, &["memory", "--resident", resident]))
    );
    assert!((1..=24).contains(&memories.as_array().unwrap().len()));

    let moved = &first_snapshot["residents"][0]["position"];
    let x = moved["x"].as_u64().unwrap();
    let next_x = if x == 0 { 1 } else { x - 1 };
    let next_x = next_x.to_string();
    let z = moved["z"].as_u64().unwrap().to_string();
    let move_args = [
        "move",
        "--resident",
        resident,
        "--x",
        &next_x,
        "--z",
        &z,
        "--revision",
        "60",
    ];
    let moved = json(world(&first, &move_args));
    assert_eq!(moved["tick"], 60);
    assert_eq!(moved["revision"], 61);
    let stale = world(&first, &move_args);
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("world changed"));
    assert_eq!(json(world(&first, &["inspect"])), moved);

    json(world(
        &first,
        &[
            "set-active",
            "--resident",
            resident,
            "--active",
            "false",
            "--revision",
            "61",
        ],
    ));
    let advanced = json(world(&first, &["advance", "--ticks", "5"]));
    assert_eq!(advanced["tick"], 65);
    assert_eq!(advanced["residents"][0]["active"], false);
    assert_eq!(
        advanced["residents"][0]["position"],
        moved["residents"][0]["position"]
    );
}
