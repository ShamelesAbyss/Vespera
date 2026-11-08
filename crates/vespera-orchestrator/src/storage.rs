//! Private workflow checkpoints. The connection holds an exclusive lock until dropped.
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use thiserror::Error;
use vespera_core::{TaskId, TaskState};

use crate::{Workflow, WorkflowReport};

const APPLICATION_ID: i64 = 0x56535057;
const VERSION: i64 = 1;
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

pub struct WorkflowStore(Mutex<Connection>);

impl WorkflowStore {
    /// Open or initialize a private workflow database.
    pub fn open(path: &Path) -> Result<Self, WorkflowStoreError> {
        Self::connect(path, true)
    }

    fn connect(path: &Path, create: bool) -> Result<Self, WorkflowStoreError> {
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
                    terminal INTEGER NOT NULL CHECK(terminal IN (0, 1))
                ) STRICT;",
            )
            .map_err(db)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(db)?;
            tx.pragma_update(None, "user_version", VERSION)
                .map_err(db)?;
        } else if (id, version) != (APPLICATION_ID, VERSION) {
            return Err(WorkflowStoreError::WrongDatabase);
        }
        tx.commit().map_err(db)?;
        Ok(Self(Mutex::new(connection)))
    }

    pub fn contains(&self, task_id: TaskId) -> Result<bool, WorkflowStoreError> {
        self.0
            .lock()
            .map_err(|_| WorkflowStoreError::Database)?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workflows WHERE task_id=?1)",
                [task_id.to_string()],
                |row| row.get(0),
            )
            .map_err(db)
    }

    pub(crate) fn begin(
        &self,
        workflow: &Workflow,
        report: &WorkflowReport,
    ) -> Result<(), WorkflowStoreError> {
        let plan = encode(workflow, MAX_PLAN_BYTES)?;
        let snapshot = encode(report, MAX_REPORT_BYTES)?;
        let id = workflow.task.id.to_string();
        let connection = self.0.lock().map_err(|_| WorkflowStoreError::Database)?;
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
            "INSERT INTO workflows SELECT ?1, ?2, ?3, 0 WHERE (SELECT count(*) FROM workflows) < ?4",
            params![id, plan, snapshot, MAX_WORKFLOWS],
        ).map_err(db)?;
        if written != 1 {
            return Err(WorkflowStoreError::Capacity);
        }
        Ok(())
    }

    pub(crate) fn checkpoint(&self, report: &WorkflowReport) -> Result<(), WorkflowStoreError> {
        let snapshot = encode(report, MAX_REPORT_BYTES)?;
        let terminal = report.task.state != TaskState::Running;
        let written = self
            .0
            .lock()
            .map_err(|_| WorkflowStoreError::Database)?
            .execute(
                "UPDATE workflows SET report=?2, terminal=?3 WHERE task_id=?1 AND terminal=0",
                params![report.task.id.to_string(), snapshot, terminal],
            )
            .map_err(db)?;
        if written != 1 {
            return Err(WorkflowStoreError::InvalidCheckpoint);
        }
        Ok(())
    }

    /// Read a checkpoint after acquiring the exclusive database connection.
    pub fn inspect(path: &Path, task_id: TaskId) -> Result<StoredWorkflow, WorkflowStoreError> {
        Self::connect(path, false)?.read(task_id)
    }

    fn read(&self, task_id: TaskId) -> Result<StoredWorkflow, WorkflowStoreError> {
        let (plan, report, terminal): (Option<Vec<u8>>, Option<Vec<u8>>, bool) = self
            .0
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
