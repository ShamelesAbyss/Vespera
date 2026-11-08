use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use vespera_core::{AgentId, MemoryId, MemoryLayer, MemoryRecord, Permission, TaskId};
use vespera_memory::{
    MemoryAuthorizer, MemoryOrigin, MemoryQuery, MemoryStore, StorageError, StorageLimits,
};

#[derive(Default)]
struct Authority {
    grants: RwLock<BTreeMap<AgentId, BTreeSet<Permission>>>,
    tasks: RwLock<BTreeMap<TaskId, AgentId>>,
}
impl MemoryAuthorizer for Authority {
    fn allowed(&self, actor: AgentId, permission: Permission) -> bool {
        self.grants
            .read()
            .unwrap()
            .get(&actor)
            .is_some_and(|grants| grants.contains(&permission))
    }
    fn owns_task(&self, actor: AgentId, task: TaskId) -> bool {
        self.tasks.read().unwrap().get(&task) == Some(&actor)
    }
}
struct Database {
    directory: PathBuf,
    authority: Arc<Authority>,
    owners: [AgentId; 2],
}
impl Database {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("vespera-memory-{}", MemoryId::new()));
        std::fs::create_dir(&directory).unwrap();
        let authority = Arc::new(Authority::default());
        let owners = [AgentId::new(), AgentId::new()];
        for owner in owners {
            authority.grants.write().unwrap().insert(
                owner,
                [Permission::MemoryRead, Permission::MemoryWrite].into(),
            );
        }
        Self {
            directory,
            authority,
            owners,
        }
    }
    fn path(&self) -> PathBuf {
        self.directory.join("memory.sqlite3")
    }
    fn open(&self, limits: StorageLimits) -> MemoryStore {
        MemoryStore::open(self.path(), limits, self.authority.clone()).unwrap()
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
fn record(owner: AgentId, content: &str) -> MemoryRecord {
    MemoryRecord {
        id: MemoryId::new(),
        agent_id: owner,
        layer: MemoryLayer::Observation,
        content: content.into(),
        created_at_unix_ms: 123,
        source_task_id: None,
    }
}

#[test]
fn snapshot_and_restore_preserve_wal_commits_namespaces_fts_and_lineage() {
    use vespera_memory::copy_memory_database;

    let database = Database::new();
    let [a, b] = database.owners;
    let store = database.open(StorageLimits::default());
    let wal = rusqlite::Connection::open(database.path()).unwrap();
    wal.pragma_update(None, "journal_mode", "WAL").unwrap();
    let first = record(a, "amber source");
    let private = record(b, "violet private");
    let summary = record(a, "amber summary");
    store
        .insert(a, first.clone(), MemoryOrigin::Observation)
        .unwrap();
    store
        .insert(b, private.clone(), MemoryOrigin::Observation)
        .unwrap();
    store
        .consolidate(a, summary.clone(), &[first.id], "manual")
        .unwrap();
    assert!(
        database
            .directory
            .join("memory.sqlite3-wal")
            .metadata()
            .unwrap()
            .len()
            > 32
    );

    let backup = database.directory.join("backup.sqlite3");
    let restored = database.directory.join("restored.sqlite3");
    copy_memory_database(&database.path(), &backup).unwrap();
    let after = record(a, "later amber observation");
    store
        .insert(a, after.clone(), MemoryOrigin::Observation)
        .unwrap();
    copy_memory_database(&backup, &restored).unwrap();

    let restored_store = MemoryStore::open_existing(
        &restored,
        StorageLimits::default(),
        database.authority.clone(),
    )
    .unwrap();
    assert!(restored_store.get(a, a, after.id).unwrap().is_none());
    assert_eq!(
        restored_store.get(a, b, private.id),
        Err(StorageError::Denied)
    );
    assert_eq!(
        restored_store
            .get(b, b, private.id)
            .unwrap()
            .unwrap()
            .record,
        private
    );
    assert_eq!(
        restored_store
            .get(a, a, summary.id)
            .unwrap()
            .unwrap()
            .source_memories,
        vec![first.id]
    );
    assert_eq!(
        restored_store
            .search(a, a, "amber", &MemoryQuery::default())
            .unwrap()
            .records
            .len(),
        2
    );
    assert_eq!(
        restored_store
            .insert(a, record(a, "new record"), MemoryOrigin::Observation)
            .unwrap()
            .sequence,
        3
    );
}

#[test]
fn snapshot_refuses_existing_targets_sidecars_and_corrupt_search_index() {
    use vespera_memory::copy_memory_database;

    let database = Database::new();
    let [owner, _] = database.owners;
    database
        .open(StorageLimits::default())
        .insert(
            owner,
            record(owner, "searchable"),
            MemoryOrigin::Observation,
        )
        .unwrap();
    let destination = database.directory.join("snapshot.sqlite3");
    std::fs::write(&destination, b"keep").unwrap();
    assert_eq!(
        copy_memory_database(&database.path(), &destination),
        Err(StorageError::SnapshotTargetExists)
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"keep");
    std::fs::remove_file(&destination).unwrap();
    let sidecar = database.directory.join("snapshot.sqlite3-wal");
    std::fs::write(&sidecar, b"keep sidecar").unwrap();
    assert_eq!(
        copy_memory_database(&database.path(), &destination),
        Err(StorageError::SnapshotTargetExists)
    );
    assert_eq!(std::fs::read(&sidecar).unwrap(), b"keep sidecar");
    std::fs::remove_file(&sidecar).unwrap();

    let connection = rusqlite::Connection::open(database.path()).unwrap();
    connection
        .execute(
            "INSERT INTO memory_fts(memory_fts) VALUES('delete-all')",
            [],
        )
        .unwrap();
    drop(connection);
    assert_eq!(
        copy_memory_database(&database.path(), &destination),
        Err(StorageError::SnapshotInvalid)
    );
    assert!(!destination.exists());
    assert_eq!(std::fs::read_dir(&database.directory).unwrap().count(), 1);
}

#[test]
fn committed_namespaces_and_text_search_survive_reopen_without_inference() {
    let database = Database::new();
    let [a, b] = database.owners;
    let first = record(a, "durable amber observation");
    let second = record(b, "durable violet observation");
    {
        let store = database.open(StorageLimits::default());
        assert_eq!(
            store
                .insert(a, first.clone(), MemoryOrigin::Observation)
                .unwrap()
                .sequence,
            1
        );
        assert_eq!(
            store
                .insert(b, second.clone(), MemoryOrigin::Import)
                .unwrap()
                .sequence,
            1
        );
        assert_eq!(
            store
                .insert(a, record(a, "second amber note"), MemoryOrigin::Observation)
                .unwrap()
                .sequence,
            2
        );
    }
    let store = database.open(StorageLimits::default());
    assert_eq!(store.get(a, a, first.id).unwrap().unwrap().record, first);
    assert_eq!(store.get(b, b, second.id).unwrap().unwrap().record, second);
    assert!(store.get(a, a, second.id).unwrap().is_none());
    let results = store
        .search(a, a, "amber", &MemoryQuery::default())
        .unwrap();
    assert_eq!(results.records.len(), 2);
    assert!(
        store
            .search(a, a, "violet", &MemoryQuery::default())
            .unwrap()
            .records
            .is_empty()
    );
    assert_eq!(store.namespace_usage(a, a).unwrap().records, 2);
}

#[test]
fn all_namespace_operations_deny_cross_owner_access_and_current_grants_are_checked() {
    let database = Database::new();
    let [a, b] = database.owners;
    let store = database.open(StorageLimits::default());
    let private = record(b, "private violet");
    store
        .insert(b, private.clone(), MemoryOrigin::Observation)
        .unwrap();
    assert_eq!(store.get(a, b, private.id), Err(StorageError::Denied));
    assert!(matches!(
        store.list(a, b, &MemoryQuery::default()),
        Err(StorageError::Denied)
    ));
    assert!(matches!(
        store.search(a, b, "private", &MemoryQuery::default()),
        Err(StorageError::Denied)
    ));
    assert_eq!(store.namespace_usage(a, b), Err(StorageError::Denied));
    assert_eq!(store.delete(a, b, private.id), Err(StorageError::Denied));
    assert_eq!(
        store.insert(a, record(b, "wrong owner"), MemoryOrigin::Import),
        Err(StorageError::Denied)
    );
    database.authority.grants.write().unwrap().remove(&b);
    assert_eq!(store.get(b, b, private.id), Err(StorageError::Denied));
    assert_eq!(
        store.insert(b, record(b, "revoked"), MemoryOrigin::Observation),
        Err(StorageError::Denied)
    );
}

#[test]
fn source_tasks_must_be_accepted_for_owner_and_generated_origin_requires_a_source() {
    let database = Database::new();
    let [a, b] = database.owners;
    let store = database.open(StorageLimits::default());
    let task = TaskId::new();
    let mut memory = record(a, "generated observation");
    memory.source_task_id = Some(task);
    assert_eq!(
        store.insert(a, memory.clone(), MemoryOrigin::Generated),
        Err(StorageError::SourceUnavailable)
    );
    database.authority.tasks.write().unwrap().insert(task, b);
    assert_eq!(
        store.insert(a, memory.clone(), MemoryOrigin::Generated),
        Err(StorageError::SourceUnavailable)
    );
    database.authority.tasks.write().unwrap().insert(task, a);
    assert_eq!(
        store
            .insert(a, memory.clone(), MemoryOrigin::Generated)
            .unwrap()
            .record,
        memory
    );
    assert!(matches!(
        store.insert(a, record(a, "no task"), MemoryOrigin::Generated),
        Err(StorageError::Invalid(_))
    ));
    let mut bad = record(a, "timestamp");
    bad.created_at_unix_ms = u64::MAX;
    assert!(matches!(
        store.insert(a, bad, MemoryOrigin::Observation),
        Err(StorageError::Invalid(_))
    ));
}

#[test]
fn consolidation_preserves_sources_and_atomic_lineage_across_restart_and_deletion() {
    let database = Database::new();
    let [a, b] = database.owners;
    let first = record(a, "original episode");
    let other = record(b, "private episode");
    let summary = record(a, "derived concise knowledge");
    {
        let store = database.open(StorageLimits::default());
        store
            .insert(a, first.clone(), MemoryOrigin::Observation)
            .unwrap();
        store
            .insert(b, other.clone(), MemoryOrigin::Observation)
            .unwrap();
        assert_eq!(
            store.consolidate(a, summary.clone(), &[other.id], "operator-summary"),
            Err(StorageError::SourceUnavailable)
        );
        assert!(store.get(a, a, summary.id).unwrap().is_none());
        let derived = store
            .consolidate(a, summary.clone(), &[first.id], "operator-summary")
            .unwrap();
        assert_eq!(derived.source_memories, [first.id]);
        assert_eq!(derived.origin, MemoryOrigin::Derived);
        assert_eq!(store.delete(a, a, first.id), Err(StorageError::Referenced));
        assert_eq!(
            store.consolidate(a, summary.clone(), &[first.id], "operator-summary"),
            Err(StorageError::Conflict)
        );
    }
    let store = database.open(StorageLimits::default());
    let derived = store.get(a, a, summary.id).unwrap().unwrap();
    assert_eq!(
        derived.derivation_method.as_deref(),
        Some("operator-summary")
    );
    assert_eq!(derived.source_memories, [first.id]);
    assert!(store.get(a, a, first.id).unwrap().is_some());
    store.delete(a, a, summary.id).unwrap();
    assert!(
        store
            .search(a, a, "derived", &MemoryQuery::default())
            .unwrap()
            .records
            .is_empty()
    );
    store.delete(a, a, first.id).unwrap();
    assert_eq!(store.namespace_usage(a, a).unwrap().records, 0);
    let replacement = store
        .insert(a, record(a, "new episode"), MemoryOrigin::Observation)
        .unwrap();
    assert!(replacement.sequence > derived.sequence); // deletion does not reset cursors
}

#[test]
fn injected_lineage_failure_rolls_back_memory_fts_counter_and_quota() {
    let database = Database::new();
    let a = database.owners[0];
    let store = database.open(StorageLimits::default());
    let source = store
        .insert(a, record(a, "original"), MemoryOrigin::Observation)
        .unwrap();
    let connection = rusqlite::Connection::open(database.path()).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_lineage BEFORE INSERT ON memory_sources BEGIN SELECT RAISE(ABORT,'fixture-private-diagnostic'); END;").unwrap();
    let summary = record(a, "rolledbackword");
    let error = store
        .consolidate(a, summary.clone(), &[source.record.id], "fixture")
        .unwrap_err();
    assert!(!error.to_string().contains("fixture-private-diagnostic"));
    assert!(store.get(a, a, summary.id).unwrap().is_none());
    assert!(
        store
            .search(a, a, "rolledbackword", &MemoryQuery::default())
            .unwrap()
            .records
            .is_empty()
    );
    assert_eq!(store.namespace_usage(a, a).unwrap().records, 1);
    connection
        .execute_batch("DROP TRIGGER reject_lineage;")
        .unwrap();
    let committed = store
        .consolidate(a, summary, &[source.record.id], "fixture")
        .unwrap();
    assert_eq!(committed.sequence, 2);
}

#[test]
fn quotas_are_atomic_across_independent_connections_and_rejected_ids_are_reusable() {
    let database = Database::new();
    let a = database.owners[0];
    let limits = StorageLimits {
        max_namespace_records: 1,
        max_total_records: 1,
        ..Default::default()
    };
    let first = database.open(limits);
    let second = database.open(limits);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for store in [first, second] {
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            store.insert(a, record(a, "competing"), MemoryOrigin::Observation)
        }));
    }
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(StorageError::Capacity)))
            .count(),
        1
    );
    let store = database.open(limits);
    let existing = store.list(a, a, &MemoryQuery::default()).unwrap().records[0]
        .record
        .id;
    let pending = record(a, "later");
    assert_eq!(
        store.insert(a, pending.clone(), MemoryOrigin::Observation),
        Err(StorageError::Capacity)
    );
    store.delete(a, a, existing).unwrap();
    assert_eq!(
        store
            .insert(a, pending, MemoryOrigin::Observation)
            .unwrap()
            .sequence,
        2
    );
}

#[test]
fn namespace_and_byte_caps_do_not_silently_prune_or_leave_partial_fts() {
    let database = Database::new();
    let [a, b] = database.owners;
    let limits = StorageLimits {
        max_namespaces: 1,
        max_record_bytes: 8,
        max_namespace_bytes: 8,
        max_total_bytes: 8,
        ..Default::default()
    };
    let store = database.open(limits);
    assert_eq!(
        store.insert(a, record(a, "too large"), MemoryOrigin::Observation),
        Err(StorageError::Capacity)
    );
    let memory = store
        .insert(a, record(a, "12345678"), MemoryOrigin::Observation)
        .unwrap();
    assert_eq!(
        store.insert(a, record(a, "x"), MemoryOrigin::Observation),
        Err(StorageError::Capacity)
    );
    store.delete(a, a, memory.record.id).unwrap();
    assert_eq!(
        store.insert(b, record(b, "other"), MemoryOrigin::Observation),
        Err(StorageError::Capacity)
    );
    assert_eq!(
        store
            .insert(a, record(a, "same"), MemoryOrigin::Observation)
            .unwrap()
            .sequence,
        2
    );
}

#[test]
fn structured_and_phrase_retrieval_use_bounded_owner_local_cursors() {
    let database = Database::new();
    let [a, b] = database.owners;
    let store = database.open(StorageLimits::default());
    for i in 0..4 {
        let mut memory = record(
            a,
            if i == 2 {
                "amber OR violet"
            } else {
                "amber phrase"
            },
        );
        memory.layer = if i == 3 {
            MemoryLayer::Goal
        } else {
            MemoryLayer::Observation
        };
        memory.created_at_unix_ms = i;
        store.insert(a, memory, MemoryOrigin::Observation).unwrap();
        store
            .insert(
                b,
                record(b, "unrelated global activity"),
                MemoryOrigin::Observation,
            )
            .unwrap();
    }
    let mut query = MemoryQuery {
        limit: 2,
        ..Default::default()
    };
    let page = store.list(a, a, &query).unwrap();
    assert_eq!(
        page.records.iter().map(|r| r.sequence).collect::<Vec<_>>(),
        [1, 2]
    );
    query.after_sequence = page.next_after_sequence.unwrap();
    let next = store.list(a, a, &query).unwrap();
    assert_eq!(
        next.records.iter().map(|r| r.sequence).collect::<Vec<_>>(),
        [3, 4]
    );
    assert!(next.next_after_sequence.is_none());
    let filtered = MemoryQuery {
        layer: Some(MemoryLayer::Observation),
        created_since_unix_ms: Some(1),
        created_before_unix_ms: Some(3),
        ..Default::default()
    };
    assert_eq!(store.list(a, a, &filtered).unwrap().records.len(), 2);
    assert_eq!(
        store
            .search(a, a, "amber OR violet", &MemoryQuery::default())
            .unwrap()
            .records
            .len(),
        1
    );
    assert!(
        store
            .search(a, a, "\" OR content:* --", &MemoryQuery::default())
            .unwrap()
            .records
            .is_empty()
    );
    assert!(
        store
            .search(a, a, "unrelated", &MemoryQuery::default())
            .unwrap()
            .records
            .is_empty()
    );
    assert!(matches!(
        store.list(
            a,
            a,
            &MemoryQuery {
                limit: 0,
                ..Default::default()
            }
        ),
        Err(StorageError::Invalid(_))
    ));
    assert!(matches!(
        store.search(a, a, &"x".repeat(257), &query),
        Err(StorageError::Invalid(_))
    ));
}

#[test]
fn content_byte_pagination_never_exceeds_requested_budget() {
    let database = Database::new();
    let a = database.owners[0];
    let store = database.open(StorageLimits::default());
    for _ in 0..3 {
        store
            .insert(
                a,
                record(a, &"é".repeat(16 * 1024)),
                MemoryOrigin::Observation,
            )
            .unwrap();
    }
    let query = MemoryQuery {
        max_content_bytes: 64 * 1024,
        ..Default::default()
    };
    let page = store.list(a, a, &query).unwrap();
    assert_eq!(page.records.len(), 2);
    assert_eq!(page.content_bytes, 64 * 1024);
    assert_eq!(page.next_after_sequence, Some(2));
}

#[test]
fn duplicate_ids_cannot_overwrite_another_namespace_or_consume_its_counter() {
    let database = Database::new();
    let [a, b] = database.owners;
    let store = database.open(StorageLimits::default());
    let original = record(a, "original");
    store
        .insert(a, original.clone(), MemoryOrigin::Observation)
        .unwrap();
    let mut duplicate = original.clone();
    duplicate.agent_id = b;
    duplicate.content = "replacement".into();
    assert_eq!(
        store.insert(b, duplicate, MemoryOrigin::Observation),
        Err(StorageError::Conflict)
    );
    assert_eq!(
        store.get(a, a, original.id).unwrap().unwrap().record,
        original
    );
    assert_eq!(
        store
            .insert(b, record(b, "own record"), MemoryOrigin::Observation)
            .unwrap()
            .sequence,
        1
    );
}

#[test]
fn read_only_and_existing_modes_never_create_or_initialize_a_missing_or_empty_file() {
    let database = Database::new();
    let a = database.owners[0];
    let record = record(a, "read only observation");
    assert!(
        MemoryStore::open_read_only(
            database.path(),
            StorageLimits::default(),
            database.authority.clone()
        )
        .is_err()
    );
    assert!(!database.path().exists());
    assert!(
        MemoryStore::open_existing(
            database.path(),
            StorageLimits::default(),
            database.authority.clone()
        )
        .is_err()
    );
    assert!(!database.path().exists());
    std::fs::write(database.path(), []).unwrap();
    assert!(matches!(
        MemoryStore::open_read_only(
            database.path(),
            StorageLimits::default(),
            database.authority.clone()
        ),
        Err(StorageError::WrongDatabase)
    ));
    assert_eq!(std::fs::metadata(database.path()).unwrap().len(), 0);
    {
        let store = database.open(StorageLimits::default());
        store
            .insert(a, record.clone(), MemoryOrigin::Observation)
            .unwrap();
    }
    let before = std::fs::read(database.path()).unwrap();
    {
        let store = MemoryStore::open_read_only(
            database.path(),
            StorageLimits::default(),
            database.authority.clone(),
        )
        .unwrap();
        assert_eq!(store.get(a, a, record.id).unwrap().unwrap().record, record);
        assert_eq!(
            store.insert(
                a,
                crate::record(a, "attempted change"),
                MemoryOrigin::Observation
            ),
            Err(StorageError::ReadOnly)
        );
        assert_eq!(store.delete(a, a, record.id), Err(StorageError::ReadOnly));
        assert_eq!(
            store
                .search(a, a, "observation", &MemoryQuery::default())
                .unwrap()
                .records
                .len(),
            1
        );
    }
    assert_eq!(std::fs::read(database.path()).unwrap(), before);
}

#[test]
fn simultaneous_initializers_preserve_one_complete_schema() {
    let database = Database::new();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let barrier = barrier.clone();
        let path = database.path();
        let authority = database.authority.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            MemoryStore::open(path, StorageLimits::default(), authority)
        }));
    }
    let stores: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().unwrap())
        .collect();
    let a = database.owners[0];
    stores[0]
        .insert(a, record(a, "initialized"), MemoryOrigin::Observation)
        .unwrap();
    assert_eq!(
        stores[1]
            .search(a, a, "initialized", &MemoryQuery::default())
            .unwrap()
            .records
            .len(),
        1
    );
}

#[test]
fn foreign_and_future_databases_are_not_modified_and_invalid_limits_create_no_file() {
    let database = Database::new();
    let raw = rusqlite::Connection::open(database.path()).unwrap();
    raw.execute_batch(
        "CREATE TABLE private_data(value TEXT); INSERT INTO private_data VALUES('preserve me');",
    )
    .unwrap();
    drop(raw);
    let before = std::fs::read(database.path()).unwrap();
    assert!(matches!(
        MemoryStore::open(
            database.path(),
            StorageLimits::default(),
            database.authority.clone()
        ),
        Err(StorageError::WrongDatabase)
    ));
    assert_eq!(std::fs::read(database.path()).unwrap(), before);
    let future = database.directory.join("future.sqlite3");
    drop(
        MemoryStore::open(
            &future,
            StorageLimits::default(),
            database.authority.clone(),
        )
        .unwrap(),
    );
    let raw = rusqlite::Connection::open(&future).unwrap();
    raw.pragma_update(None, "user_version", 999).unwrap();
    drop(raw);
    let before = std::fs::read(&future).unwrap();
    assert!(matches!(
        MemoryStore::open(
            &future,
            StorageLimits::default(),
            database.authority.clone()
        ),
        Err(StorageError::UnsupportedSchema)
    ));
    assert_eq!(std::fs::read(&future).unwrap(), before);
    let absent = database.directory.join("absent.sqlite3");
    let limits = StorageLimits {
        max_record_bytes: 0,
        ..Default::default()
    };
    assert!(MemoryStore::open(&absent, limits, database.authority.clone()).is_err());
    assert!(!absent.exists());
}

#[test]
fn bounded_busy_failure_does_not_claim_a_successful_write() {
    let database = Database::new();
    let a = database.owners[0];
    let store = database.open(StorageLimits::default());
    let raw = rusqlite::Connection::open(database.path()).unwrap();
    raw.execute_batch("BEGIN IMMEDIATE;").unwrap();
    let pending = record(a, "after lock");
    let start = std::time::Instant::now();
    assert_eq!(
        store.insert(a, pending.clone(), MemoryOrigin::Observation),
        Err(StorageError::Busy)
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    raw.execute_batch("ROLLBACK;").unwrap();
    assert_eq!(store.namespace_usage(a, a).unwrap().records, 0);
    store.insert(a, pending, MemoryOrigin::Observation).unwrap();
}

#[test]
fn abrupt_writer_exit_rolls_back_uncommitted_memory_and_fts_after_reopen() {
    let database = Database::new();
    let a = database.owners[0];
    {
        let store = database.open(StorageLimits::default());
        store
            .insert(
                a,
                record(a, "committed survivor"),
                MemoryOrigin::Observation,
            )
            .unwrap();
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_writer_child", "--ignored"])
        .env("VESPERA_TEST_CRASH_DATABASE", database.path())
        .env("VESPERA_TEST_CRASH_OWNER", a.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut journal = database.path().into_os_string();
    journal.push("-journal");
    assert!(
        std::fs::metadata(PathBuf::from(journal)).unwrap().len() > 512,
        "child must actually leave an uncommitted rollback journal"
    );
    let store = database.open(StorageLimits::default());
    assert_eq!(store.namespace_usage(a, a).unwrap().records, 1);
    assert!(
        store
            .search(a, a, "uncommitted", &MemoryQuery::default())
            .unwrap()
            .records
            .is_empty()
    );
    assert_eq!(
        store
            .search(a, a, "survivor", &MemoryQuery::default())
            .unwrap()
            .records
            .len(),
        1
    );
}

#[test]
#[ignore = "Only invoked by the abrupt-writer parent test with private child-process environment"]
fn crash_writer_child() {
    let path =
        std::env::var_os("VESPERA_TEST_CRASH_DATABASE").expect("parent supplies private database");
    let owner = std::env::var("VESPERA_TEST_CRASH_OWNER").unwrap();
    let connection = rusqlite::Connection::open(PathBuf::from(path)).unwrap();
    connection
        .execute_batch("PRAGMA cache_size=1; PRAGMA synchronous=FULL; BEGIN IMMEDIATE;")
        .unwrap();
    for i in 0..20 {
        let content = format!("uncommitted {}", "x".repeat(4096));
        connection.execute("INSERT INTO memories(id,owner,layer,content,content_bytes,created_at,origin,owner_sequence) VALUES(?1,?2,'observation',?3,?4,0,'observation',?5)",rusqlite::params![MemoryId::new().to_string(),owner,content,content.len() as i64,i+2]).unwrap();
        connection
            .execute(
                "INSERT INTO memory_fts(rowid,content) VALUES(?1,?2)",
                rusqlite::params![connection.last_insert_rowid(), content],
            )
            .unwrap();
    }
    std::process::exit(0); // Deliberately bypass Connection/Transaction destructors.
}
