//! Private workflow checkpoints. The connection holds an exclusive lock until dropped.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use thiserror::Error;
use vespera_core::{TaskId, TaskState};

use crate::{Workflow, WorkflowReport};

const APPLICATION_ID: i64 = 0x56535057;
const VERSION: i64 = 2;
const MAX_WORKFLOWS: i64 = 16;
const MAX_PLAN_BYTES: usize = 1024 * 1024;
const MAX_REPORT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum WorkflowStoreError {
    #[error("file is not a supported Vespera workflow database")]
    WrongDatabase,
    #[error("workflow database is in use")]
    Busy,
    #[error("workflow database operation failed")]
    Database,
    #[error("workflow identity already exists in the database")]
    Duplicate,
    #[error("workflow database capacity reached")]
    Capacity,
    #[error("workflow checkpoint is invalid or exceeds storage bounds")]
    InvalidCheckpoint,
    #[error("workflow identity is absent from the database")]
    NotFound,
    #[error("supplied plan differs from the stored workflow")]
    PlanChanged,
    #[error("workflow has a terminal outcome or an unresolved attempt")]
    UnsafeResume,
    #[error("workflow resume deadline is missing, expired, or affected by a clock change")]
    ResumeDeadline,
}

fn db(error: rusqlite::Error) -> WorkflowStoreError {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => WorkflowStoreError::Busy,
        Some(ErrorCode::NotADatabase) => WorkflowStoreError::WrongDatabase,
        _ => WorkflowStoreError::Database,
    }
}

#[derive(Debug, Serialize)]
pub struct StoredWorkflow {
    pub plan: Workflow,
    pub report: WorkflowReport,
    /// The exclusive connection was reopened with a running checkpoint still recorded.
    pub interrupted: bool,
}

pub struct WorkflowStore {
    connection: Mutex<Connection>,
    claimed: Mutex<BTreeSet<TaskId>>,
}

impl WorkflowStore {
    /// Open or initialize a private workflow database.
    pub fn open(path: &Path) -> Result<Self, WorkflowStoreError> {
        Self::connect(path, true, true)
    }

    /// Open an existing database for continuation without creating a missing file.
    pub fn open_for_resume(path: &Path) -> Result<Self, WorkflowStoreError> {
        Self::connect(path, false, true)
    }

    fn connect(path: &Path, create: bool, migrate: bool) -> Result<Self, WorkflowStoreError> {
        if path.as_os_str().is_empty() {
            return Err(WorkflowStoreError::WrongDatabase);
        }
        let path = std::path::absolute(path).map_err(|_| WorkflowStoreError::Database)?;
        let mut flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        if create {
            flags |= OpenFlags::SQLITE_OPEN_CREATE;
        }
        let mut connection = Connection::open_with_flags(path, flags).map_err(db)?;
        connection.busy_timeout(Duration::ZERO).map_err(db)?;
        connection
            .pragma_update(None, "locking_mode", "EXCLUSIVE")
            .map_err(db)?;
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
        if create && (id, version) == (0, 0) {
            let empty: bool = tx
                .query_row(
                    "SELECT NOT EXISTS(SELECT 1 FROM sqlite_schema)",
                    [],
                    |row| row.get(0),
                )
                .map_err(db)?;
            if !empty {
                return Err(WorkflowStoreError::WrongDatabase);
            }
            tx.execute_batch(
                "CREATE TABLE workflows (
                    task_id TEXT PRIMARY KEY NOT NULL,
                    plan BLOB NOT NULL,
                    report BLOB NOT NULL,
                    terminal INTEGER NOT NULL CHECK(terminal IN (0, 1)),
                    started_unix_ms INTEGER,
                    observed_unix_ms INTEGER
                ) STRICT;",
            )
            .map_err(db)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(db)?;
            tx.pragma_update(None, "user_version", VERSION)
                .map_err(db)?;
        } else if (id, version) == (APPLICATION_ID, 1) {
            if migrate {
                tx.execute_batch(
                    "ALTER TABLE workflows ADD COLUMN started_unix_ms INTEGER;
                     ALTER TABLE workflows ADD COLUMN observed_unix_ms INTEGER;",
                )
                .map_err(db)?;
                tx.pragma_update(None, "user_version", VERSION)
                    .map_err(db)?;
            }
        } else if (id, version) != (APPLICATION_ID, VERSION) {
            return Err(WorkflowStoreError::WrongDatabase);
        }
        tx.commit().map_err(db)?;
        Ok(Self {
            connection: Mutex::new(connection),
            claimed: Mutex::new(BTreeSet::new()),
        })
    }

    pub fn contains(&self, task_id: TaskId) -> Result<bool, WorkflowStoreError> {
        self.connection
            .lock()
            .map_err(|_| WorkflowStoreError::Database)?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workflows WHERE task_id=?1)",
                [task_id.to_string()],
                |row| row.get(0),
            )
            .map_err(db)
    }

    /// Check the plan, deadline, and step boundary before runtime composition.
    pub fn ensure_resumable(&self, workflow: &Workflow) -> Result<(), WorkflowStoreError> {
        if self
            .claimed
            .lock()
            .map_err(|_| WorkflowStoreError::Database)?
            .contains(&workflow.task.id)
        {
            return Err(WorkflowStoreError::Busy);
        }
        let (report, _) = self.resumable_report(workflow)?;
        crate::runtime::committed_steps(workflow, &report)?;
        Ok(())
    }

    pub(crate) fn begin(
        &self,
        workflow: &Workflow,
        report: &WorkflowReport,
    ) -> Result<(), WorkflowStoreError> {
        let plan = encode(workflow, MAX_PLAN_BYTES)?;
        let snapshot = encode(report, MAX_REPORT_BYTES)?;
        let id = workflow.task.id.to_string();
        let now = unix_ms()?;
        let mut claimed = self
            .claimed
            .lock()
            .map_err(|_| WorkflowStoreError::Database)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| WorkflowStoreError::Database)?;
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workflows WHERE task_id=?1)",
                [&id],
                |row| row.get(0),
            )
            .map_err(db)?;
        if exists {
            return Err(WorkflowStoreError::Duplicate);
        }
        let written = connection.execute(
            "INSERT INTO workflows SELECT ?1, ?2, ?3, 0, ?4, ?4 WHERE (SELECT count(*) FROM workflows) < ?5",
            params![id, plan, snapshot, now, MAX_WORKFLOWS],
        ).map_err(db)?;
        if written != 1 {
            return Err(WorkflowStoreError::Capacity);
        }
        claimed.insert(workflow.task.id);
        Ok(())
    }

    pub(crate) fn checkpoint(&self, report: &WorkflowReport) -> Result<(), WorkflowStoreError> {
        let snapshot = encode(report, MAX_REPORT_BYTES)?;
        let terminal = report.task.state != TaskState::Running;
        let written = self
            .connection
            .lock()
            .map_err(|_| WorkflowStoreError::Database)?
            .execute(
                "UPDATE workflows SET report=?2, terminal=?3, observed_unix_ms=max(observed_unix_ms,?4)
                 WHERE task_id=?1 AND terminal=0",
                params![report.task.id.to_string(), snapshot, terminal, unix_ms()?],
            )
            .map_err(db)?;
        if written != 1 {
            return Err(WorkflowStoreError::InvalidCheckpoint);
        }
        Ok(())
    }

    /// Read a checkpoint after acquiring the exclusive database connection.
    pub fn inspect(path: &Path, task_id: TaskId) -> Result<StoredWorkflow, WorkflowStoreError> {
        Self::connect(path, false, false)?.read(task_id)
    }

    pub(crate) fn claim_resume(
        &self,
        workflow: &Workflow,
    ) -> Result<(WorkflowReport, Duration, BTreeMap<String, usize>), WorkflowStoreError> {
        let mut claimed = self
            .claimed
            .lock()
            .map_err(|_| WorkflowStoreError::Database)?;
        if claimed.contains(&workflow.task.id) {
            return Err(WorkflowStoreError::Busy);
        }
        let (report, elapsed) = self.resumable_report(workflow)?;
        let completed = crate::runtime::committed_steps(workflow, &report)?;
        self.checkpoint(&report)?;
        claimed.insert(workflow.task.id);
        Ok((report, elapsed, completed))
    }

    fn resumable_report(
        &self,
        workflow: &Workflow,
    ) -> Result<(WorkflowReport, Duration), WorkflowStoreError> {
        let stored = self.read(workflow.task.id)?;
        if !stored.interrupted {
            return Err(WorkflowStoreError::UnsafeResume);
        }
        if encode(&stored.plan, MAX_PLAN_BYTES)? != encode(workflow, MAX_PLAN_BYTES)? {
            return Err(WorkflowStoreError::PlanChanged);
        }
        let (started, observed): (Option<i64>, Option<i64>) = self
            .connection
            .lock()
            .map_err(|_| WorkflowStoreError::Database)?
            .query_row(
                "SELECT started_unix_ms, observed_unix_ms FROM workflows WHERE task_id=?1",
                [workflow.task.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(db)?;
        let (Some(started), Some(observed)) = (started, observed) else {
            return Err(WorkflowStoreError::ResumeDeadline);
        };
        let now = unix_ms()?;
        if started < 0 || observed < started || now < observed {
            return Err(WorkflowStoreError::ResumeDeadline);
        }
        let elapsed =
            u64::try_from(now - started).map_err(|_| WorkflowStoreError::ResumeDeadline)?;
        if elapsed >= workflow.timeout_ms {
            return Err(WorkflowStoreError::ResumeDeadline);
        }
        Ok((stored.report, Duration::from_millis(elapsed)))
    }

    fn read(&self, task_id: TaskId) -> Result<StoredWorkflow, WorkflowStoreError> {
        let (plan, report, terminal): (Option<Vec<u8>>, Option<Vec<u8>>, bool) = self
            .connection
            .lock()
            .map_err(|_| WorkflowStoreError::Database)?
            .query_row(
                "SELECT CASE WHEN length(plan)<=?2 THEN plan END,
                        CASE WHEN length(report)<=?3 THEN report END, terminal
                 FROM workflows WHERE task_id=?1",
                params![
                    task_id.to_string(),
                    MAX_PLAN_BYTES as i64,
                    MAX_REPORT_BYTES as i64
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(db)?
            .ok_or(WorkflowStoreError::NotFound)?;
        let plan: Workflow = decode(plan)?;
        let report: WorkflowReport = decode(report)?;
        plan.validate()
            .map_err(|_| WorkflowStoreError::InvalidCheckpoint)?;
        if plan.task.id != task_id
            || report.task.id != task_id
            || report.coordinator != plan.coordinator
            || report.task.state == TaskState::Queued
            || terminal == (report.task.state == TaskState::Running)
        {
            return Err(WorkflowStoreError::InvalidCheckpoint);
        }
        Ok(StoredWorkflow {
            plan,
            report,
            interrupted: !terminal,
        })
    }
}

fn unix_ms() -> Result<i64, WorkflowStoreError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| WorkflowStoreError::ResumeDeadline)?;
    i64::try_from(elapsed.as_millis()).map_err(|_| WorkflowStoreError::ResumeDeadline)
}

fn encode(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, WorkflowStoreError> {
    let bytes = serde_json::to_vec(value).map_err(|_| WorkflowStoreError::InvalidCheckpoint)?;
    if bytes.len() > limit {
        return Err(WorkflowStoreError::InvalidCheckpoint);
    }
    Ok(bytes)
}

fn decode<T: serde::de::DeserializeOwned>(bytes: Option<Vec<u8>>) -> Result<T, WorkflowStoreError> {
    serde_json::from_slice(&bytes.ok_or(WorkflowStoreError::InvalidCheckpoint)?)
        .map_err(|_| WorkflowStoreError::InvalidCheckpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_rejects_expired_rolled_back_and_legacy_deadlines() {
        let mut workflow: Workflow =
            serde_json::from_str(include_str!("../../../config/workflow.example.json")).unwrap();
        workflow.task.id = TaskId::new();
        let path =
            std::env::temp_dir().join(format!("vespera-deadline-{}.sqlite3", workflow.task.id));
        let mut task = workflow.task.clone();
        task.state = TaskState::Running;
        let report = WorkflowReport {
            task,
            coordinator: workflow.coordinator,
            output_policy: workflow.output.clone(),
            attempts: vec![],
            reserved_output_tokens: 0,
            accepted_output_bytes: 0,
            selected_step: None,
            conflicting_steps: vec![],
            failure: None,
            elapsed_ms: 0,
        };
        let store = WorkflowStore::open(&path).unwrap();
        store.begin(&workflow, &report).unwrap();
        drop(store);
        let store = WorkflowStore::open_for_resume(&path).unwrap();
        store.ensure_resumable(&workflow).unwrap();
        for update in [
            "UPDATE workflows SET started_unix_ms=0, observed_unix_ms=0",
            "UPDATE workflows SET observed_unix_ms=9223372036854775807",
        ] {
            store
                .connection
                .lock()
                .unwrap()
                .execute_batch(update)
                .unwrap();
            assert!(matches!(
                store.ensure_resumable(&workflow),
                Err(WorkflowStoreError::ResumeDeadline)
            ));
        }
        store
            .connection
            .lock()
            .unwrap()
            .execute_batch(
                "ALTER TABLE workflows DROP COLUMN observed_unix_ms;
             ALTER TABLE workflows DROP COLUMN started_unix_ms;
             PRAGMA user_version=1;",
            )
            .unwrap();
        drop(store);
        let legacy = WorkflowStore::inspect(&path, workflow.task.id).unwrap();
        assert!(legacy.interrupted);
        let migrated = WorkflowStore::open_for_resume(&path).unwrap();
        assert!(matches!(
            migrated.ensure_resumable(&workflow),
            Err(WorkflowStoreError::ResumeDeadline)
        ));
        drop(migrated);
        std::fs::remove_file(path).unwrap();
    }
}
