use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use vespera_core::{AgentId, Task, TaskState};

use crate::{
    AcceptedResult, ActiveLease, Fence, LeaseBroker, MAX_CAPABILITIES, MAX_LIFETIME, MAX_TASKS,
    MAX_TEXT, MAX_WORKERS, Phase, Record,
};

const APPLICATION_ID: i64 = 0x56534252;
const SCHEMA_VERSION: i64 = 1;
const MAX_RECORD_BYTES: usize = 1024 * 1024;
const MAX_REVOCATIONS: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("broker database is busy, inaccessible or failed a write")]
    Database,
    #[error("file is not a supported broker database")]
    Format,
    #[error("broker storage requires a Unix wall clock")]
    Clock,
}

fn db(_: rusqlite::Error) -> StoreError {
    StoreError::Database
}

pub(super) fn unix_ms() -> Option<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRecord {
    task: Task,
    eligible: BTreeSet<AgentId>,
    deadline_unix_ms: i64,
    max_attempts: u32,
    generation: u32,
    phase: StoredPhase,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum StoredPhase {
    Queued {},
    Leased { worker: AgentId, fence: Fence },
    Completed { result: AcceptedResult },
    Failed {},
    Cancelled {},
}

impl From<&Record> for StoredRecord {
    fn from(record: &Record) -> Self {
        let phase = match &record.phase {
            Phase::Queued => StoredPhase::Queued {},
            Phase::Leased(active) => StoredPhase::Leased {
                worker: active.worker,
                fence: active.fence,
            },
            Phase::Completed(result) => StoredPhase::Completed {
                result: result.clone(),
            },
            Phase::Failed => StoredPhase::Failed {},
            Phase::Cancelled => StoredPhase::Cancelled {},
        };
        Self {
            task: record.task.clone(),
            eligible: record.eligible.clone(),
            deadline_unix_ms: record.deadline_unix_ms,
            max_attempts: record.max_attempts,
            generation: record.generation,
            phase,
        }
    }
}

impl StoredRecord {
    fn recover(self, clock: i64, watermark: i64, now: Instant) -> Result<Record, StoreError> {
        if self.task.validate().is_err()
            || self.task.state != TaskState::Queued
            || self.task.prompt.len() > MAX_TEXT
            || self.task.required_capabilities.len() > MAX_CAPABILITIES
            || self.eligible.is_empty()
            || self.eligible.len() > MAX_WORKERS
            || self.deadline_unix_ms < 0
            || !(1..=8).contains(&self.max_attempts)
            || self.generation > self.max_attempts
        {
            return Err(StoreError::Format);
        }
        let valid_fence = |worker, fence: Fence| {
            self.eligible.contains(&worker)
                && fence.task_id == self.task.id
                && !fence.attempt_id.is_nil()
                && fence.generation > 0
                && fence.generation == self.generation
        };
        let phase = match self.phase {
            StoredPhase::Queued {} if self.generation < self.max_attempts => Phase::Queued,
            StoredPhase::Leased { worker, fence } if valid_fence(worker, fence) => {
                // Monotonic lease time cannot cross a process boundary.
                Phase::Leased(ActiveLease {
                    worker,
                    fence,
                    expires: now,
                })
            }
            StoredPhase::Completed { result }
                if valid_fence(result.worker, result.fence)
                    && result.content.len() <= MAX_TEXT
                    && !result.content.trim().is_empty() =>
            {
                Phase::Completed(result)
            }
            StoredPhase::Failed {} => Phase::Failed,
            StoredPhase::Cancelled {} => Phase::Cancelled,
            _ => return Err(StoreError::Format),
        };
        let remaining = self.deadline_unix_ms.saturating_sub(clock);
        let deadline =
            if clock >= watermark && (1..=MAX_LIFETIME.as_millis() as i64).contains(&remaining) {
                now + Duration::from_millis(remaining as u64)
            } else {
                now
            };
        Ok(Record {
            task: self.task,
            eligible: self.eligible,
            deadline,
            deadline_unix_ms: self.deadline_unix_ms,
            max_attempts: self.max_attempts,
            generation: self.generation,
            phase,
            dirty: false,
        })
    }
}

pub(super) struct Store(Connection);

impl Store {
    pub(super) fn open(
        path: &Path,
        broker: &mut LeaseBroker,
    ) -> Result<(Self, Vec<AgentId>), StoreError> {
        let mut connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(db)?;
        connection
            .busy_timeout(Duration::from_millis(250))
            .map_err(db)?;
        connection
            .pragma_update(None, "trusted_schema", "OFF")
            .map_err(db)?;
        connection
            .pragma_update(None, "locking_mode", "EXCLUSIVE")
            .map_err(db)?;
        let mode: String = connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .map_err(db)?;
        if mode != "delete" {
            return Err(StoreError::Format);
        }
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(db)?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Exclusive)
            .map_err(db)?;
        let id: i64 = tx
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .map_err(db)?;
        let version: i64 = tx
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(db)?;
        let now = Instant::now();
        let clock = unix_ms().ok_or(StoreError::Clock)?;
        if (id, version) == (0, 0) {
            let objects: i64 = tx
                .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
                .map_err(db)?;
            if objects != 0 {
                return Err(StoreError::Format);
            }
            tx.execute_batch(
                "CREATE TABLE broker_meta (
                    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                    wall_high_water INTEGER NOT NULL CHECK(wall_high_water >= 0)
                 );
                 CREATE TABLE broker_task (
                    task_id TEXT NOT NULL UNIQUE,
                    payload BLOB NOT NULL CHECK(length(payload) <= 1048576)
                 );
                 CREATE TABLE broker_revocation (worker_id TEXT PRIMARY KEY NOT NULL);",
            )
            .map_err(db)?;
            tx.execute("INSERT INTO broker_meta VALUES (1, ?1)", [clock])
                .map_err(db)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(db)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(db)?;
        } else if (id, version) != (APPLICATION_ID, SCHEMA_VERSION) {
            return Err(StoreError::Format);
        }
        let watermark: i64 = tx
            .query_row(
                "SELECT wall_high_water FROM broker_meta WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(db)?;
        if watermark < 0 {
            return Err(StoreError::Format);
        }
        let revoked = {
            let mut statement = tx
                .prepare("SELECT worker_id FROM broker_revocation LIMIT ?1")
                .map_err(db)?;
            let mut rows = statement.query([MAX_REVOCATIONS as i64 + 1]).map_err(db)?;
            let mut revoked = Vec::new();
            while let Some(row) = rows.next().map_err(db)? {
                let id: String = row.get(0).map_err(db)?;
                revoked.push(id.parse().map_err(|_| StoreError::Format)?);
            }
            if revoked.len() > MAX_REVOCATIONS {
                return Err(StoreError::Format);
            }
            revoked
        };
        for worker in &revoked {
            if broker.workers.contains_key(worker) {
                broker
                    .revoke_worker(*worker)
                    .map_err(|_| StoreError::Format)?;
            }
        }
        {
            let mut statement = tx
                .prepare(
                    "SELECT task_id, CASE WHEN length(payload) <= ?1 THEN payload END
                 FROM broker_task ORDER BY rowid LIMIT ?2",
                )
                .map_err(db)?;
            let mut rows = statement
                .query(params![MAX_RECORD_BYTES as i64, MAX_TASKS as i64 + 1])
                .map_err(db)?;
            while let Some(row) = rows.next().map_err(db)? {
                let id: String = row.get(0).map_err(db)?;
                let bytes: Vec<u8> = row.get(1).map_err(|_| StoreError::Format)?;
                let record: StoredRecord =
                    serde_json::from_slice(&bytes).map_err(|_| StoreError::Format)?;
                if broker.tasks.len() == MAX_TASKS || id != record.task.id.to_string() {
                    return Err(StoreError::Format);
                }
                broker.tasks.push(record.recover(clock, watermark, now)?);
            }
        }
        tx.commit().map_err(db)?;
        broker.reconcile(Instant::now());
        let mut store = Self(connection);
        store.flush(broker, None, true)?;
        Ok((store, revoked))
    }

    pub(super) fn flush(
        &mut self,
        broker: &mut LeaseBroker,
        revoke: Option<AgentId>,
        startup: bool,
    ) -> Result<(), StoreError> {
        if !startup && revoke.is_none() && !broker.tasks.iter().any(|task| task.dirty) {
            return Ok(());
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        if let Some(worker) = revoke {
            tx.execute(
                "INSERT OR IGNORE INTO broker_revocation (worker_id)
                 SELECT ?1 WHERE (SELECT count(*) FROM broker_revocation) < ?2",
                params![worker.to_string(), MAX_REVOCATIONS as i64],
            )
            .map_err(db)?;
            let exists = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM broker_revocation WHERE worker_id = ?1)",
                    [worker.to_string()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(db)?;
            if !exists {
                return Err(StoreError::Database);
            }
        }
        {
            let mut upsert = tx
                .prepare(
                    "INSERT INTO broker_task (task_id, payload) VALUES (?1, ?2)
                 ON CONFLICT (task_id) DO UPDATE SET payload = excluded.payload",
                )
                .map_err(db)?;
            for task in broker.tasks.iter().filter(|task| task.dirty) {
                let bytes = serde_json::to_vec(&StoredRecord::from(task))
                    .map_err(|_| StoreError::Format)?;
                upsert
                    .execute(params![task.task.id.to_string(), bytes])
                    .map_err(db)?;
            }
        }
        tx.execute(
            "UPDATE broker_meta SET wall_high_water = max(wall_high_water, ?1) WHERE singleton = 1",
            [unix_ms().ok_or(StoreError::Clock)?],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        for task in &mut broker.tasks {
            task.dirty = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use vespera_core::TaskId;

    fn submit(broker: &mut LeaseBroker, worker: AgentId) -> TaskId {
        let id = TaskId::new();
        broker
            .submit(
                Task {
                    id,
                    prompt: "fixture".into(),
                    required_capabilities: BTreeSet::new(),
                    state: TaskState::Queued,
                },
                BTreeSet::from([worker]),
                Duration::from_secs(60),
                2,
            )
            .unwrap();
        id
    }

    #[test]
    fn clock_rollback_expires_live_work_without_extending_its_deadline() {
        let mut broker = LeaseBroker::default();
        let worker = AgentId::new();
        broker.approve_worker(worker, BTreeSet::new()).unwrap();
        submit(&mut broker, worker);
        let mut stored = StoredRecord::from(&broker.tasks[0]);
        stored.deadline_unix_ms = 11_000;
        let now = Instant::now();
        let mut recovered = stored.recover(9_000, 10_000, now).unwrap();
        assert!(recovered.expire(now).is_some());
        assert_eq!(recovered.state(), TaskState::Failed);
    }

    #[test]
    fn failed_batch_keeps_the_last_committed_state_and_revocations() {
        let path =
            std::env::temp_dir().join(format!("vespera-broker-store-{}.sqlite3", Uuid::new_v4()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Cleanup(path.clone());
        let mut broker = LeaseBroker::default();
        let worker = AgentId::new();
        broker.approve_worker(worker, BTreeSet::new()).unwrap();
        let (mut store, _) = Store::open(&path, &mut broker).unwrap();
        let original = submit(&mut broker, worker);
        store.flush(&mut broker, None, false).unwrap();
        broker.cancel(original).unwrap();
        let next = submit(&mut broker, worker);
        broker.revoke_worker(worker).unwrap();
        store
            .0
            .execute_batch(&format!(
                "CREATE TRIGGER reject_fixture BEFORE INSERT ON broker_task
             WHEN NEW.task_id = '{next}' BEGIN SELECT RAISE(ABORT, 'fixture'); END;"
            ))
            .unwrap();
        assert!(matches!(
            store.flush(&mut broker, Some(worker), false),
            Err(StoreError::Database)
        ));
        assert!(broker.tasks.iter().all(|task| task.dirty));
        drop(store);
        let mut restored = LeaseBroker::default();
        restored.approve_worker(worker, BTreeSet::new()).unwrap();
        let (_store, revocations) = Store::open(&path, &mut restored).unwrap();
        assert!(revocations.is_empty());
        assert_eq!(restored.state(original).unwrap(), TaskState::Queued);
        assert_eq!(restored.state(next), Err(crate::BrokerError::Missing));
    }
}
