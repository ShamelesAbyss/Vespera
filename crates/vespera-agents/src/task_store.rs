//! Bounded private task ownership and execution history.
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use vespera_core::{AgentId, Task, TaskId, TaskState};

use crate::TaskExecutionResult;

const APPLICATION_ID: i64 = 0x56535041;
const VERSION: i64 = 2;
const MAX_TASKS: i64 = 256;
const MAX_SUSPENSIONS: i64 = 256;
const MAX_BYTES: i64 = 128 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum AgentTaskStoreError {
    #[error("file is not a supported Vespera agent task database")]
    WrongDatabase,
    #[error("agent task database is in use")]
    Busy,
    #[error("agent task database operation failed")]
    Database,
    #[error("agent task identity already exists in the database")]
    Duplicate,
    #[error("agent task database capacity reached")]
    Capacity,
    #[error("invalid or inconsistent agent task checkpoint")]
    InvalidCheckpoint,
    #[error("agent task identity is absent from the database")]
    NotFound,
}

fn db(error: rusqlite::Error) -> AgentTaskStoreError {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => AgentTaskStoreError::Busy,
        Some(ErrorCode::NotADatabase) => AgentTaskStoreError::WrongDatabase,
        _ => AgentTaskStoreError::Database,
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StoredAgentTask {
    pub task: Task,
    pub owner: AgentId,
    pub delegated_by: Option<AgentId>,
    pub result: Option<TaskExecutionResult>,
    /// A nonterminal checkpoint observed after the exclusive writer closed.
    pub interrupted: bool,
}

#[derive(Debug)]
pub struct AgentTaskStore(Mutex<Connection>);

impl AgentTaskStore {
    /// Open or initialize a private agent task database.
    pub fn open(path: &Path) -> Result<Self, AgentTaskStoreError> {
        Self::connect(path, true)
    }

    fn connect(path: &Path, create: bool) -> Result<Self, AgentTaskStoreError> {
        if path.as_os_str().is_empty() {
            return Err(AgentTaskStoreError::WrongDatabase);
        }
        let path = std::path::absolute(path).map_err(|_| AgentTaskStoreError::Database)?;
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
                return Err(AgentTaskStoreError::WrongDatabase);
            }
            tx.execute_batch(
                "CREATE TABLE agent_tasks (
                    id TEXT PRIMARY KEY NOT NULL,
                    owner TEXT NOT NULL,
                    delegated_by TEXT,
                    state TEXT NOT NULL CHECK(state IN ('queued','running','completed','failed','cancelled')),
                    task BLOB NOT NULL,
                    result BLOB
                ) STRICT;",
            ).map_err(db)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(db)?;
        } else if id != APPLICATION_ID || !matches!(version, 1 | VERSION) {
            return Err(AgentTaskStoreError::WrongDatabase);
        }
        if create && version < VERSION {
            tx.execute_batch(
                "CREATE TABLE agent_suspensions (id TEXT PRIMARY KEY NOT NULL) STRICT;",
            )
            .map_err(db)?;
            tx.pragma_update(None, "user_version", VERSION)
                .map_err(db)?;
        }
        tx.commit().map_err(db)?;
        Ok(Self(Mutex::new(connection)))
    }

    /// Read an operator hold from an existing database without migrating it.
    pub fn inspect_suspension(path: &Path, agent: AgentId) -> Result<bool, AgentTaskStoreError> {
        Self::connect(path, false)?.is_suspended(agent)
    }

    /// Clear a hold in an existing database. Version 1 has no holds to clear.
    pub fn release_suspension(path: &Path, agent: AgentId) -> Result<(), AgentTaskStoreError> {
        let store = Self::connect(path, false)?;
        if store.is_suspended(agent)? {
            store.set_suspended(agent, false)?;
        }
        Ok(())
    }

    /// Check whether the operator has placed an agent on hold.
    pub fn is_suspended(&self, agent: AgentId) -> Result<bool, AgentTaskStoreError> {
        let connection = self.0.lock().map_err(|_| AgentTaskStoreError::Database)?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(db)?;
        if version == 1 {
            return Ok(false);
        }
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_suspensions WHERE id=?1)",
                [agent.to_string()],
                |row| row.get(0),
            )
            .map_err(db)
    }

    /// Read the bounded set of operator holds before accepting work.
    pub fn suspensions(&self) -> Result<BTreeSet<AgentId>, AgentTaskStoreError> {
        let connection = self.0.lock().map_err(|_| AgentTaskStoreError::Database)?;
        let mut query = connection
            .prepare("SELECT id FROM agent_suspensions LIMIT ?1")
            .map_err(db)?;
        let mut rows = query.query([MAX_SUSPENSIONS + 1]).map_err(db)?;
        let mut agents = BTreeSet::new();
        while let Some(row) = rows.next().map_err(db)? {
            let id: String = row.get(0).map_err(db)?;
            let agent = id
                .parse()
                .map_err(|_| AgentTaskStoreError::InvalidCheckpoint)?;
            if !agents.insert(agent) || agents.len() > MAX_SUSPENSIONS as usize {
                return Err(AgentTaskStoreError::InvalidCheckpoint);
            }
        }
        Ok(agents)
    }

    /// Add or clear an explicit operator hold. Both operations are idempotent.
    pub fn set_suspended(
        &self,
        agent: AgentId,
        suspended: bool,
    ) -> Result<(), AgentTaskStoreError> {
        let id = agent.to_string();
        let connection = self.0.lock().map_err(|_| AgentTaskStoreError::Database)?;
        if !suspended {
            connection
                .execute("DELETE FROM agent_suspensions WHERE id=?1", [id])
                .map_err(db)?;
            return Ok(());
        }
        let written = connection
            .execute(
                "INSERT INTO agent_suspensions (id)
             SELECT ?1 WHERE (SELECT COUNT(*) FROM agent_suspensions) < ?2
             ON CONFLICT(id) DO NOTHING",
                params![id, MAX_SUSPENSIONS],
            )
            .map_err(db)?;
        if written == 0
            && !connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM agent_suspensions WHERE id=?1)",
                    [id],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(db)?
        {
            return Err(AgentTaskStoreError::Capacity);
        }
        Ok(())
    }

    pub fn contains(&self, task_id: TaskId) -> Result<bool, AgentTaskStoreError> {
        self.0
            .lock()
            .map_err(|_| AgentTaskStoreError::Database)?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_tasks WHERE id=?1)",
                [task_id.to_string()],
                |row| row.get(0),
            )
            .map_err(db)
    }

    pub(crate) fn insert(
        &self,
        task: &Task,
        owner: AgentId,
        delegated_by: Option<AgentId>,
    ) -> Result<(), AgentTaskStoreError> {
        let bytes = encode(task)?;
        let written = self.0.lock().map_err(|_| AgentTaskStoreError::Database)?.execute(
            "INSERT INTO agent_tasks SELECT ?1,?2,?3,?4,?5,NULL WHERE
             (SELECT count(*) FROM agent_tasks) < ?6 AND
             (SELECT coalesce(sum(length(task)+coalesce(length(result),0)),0) FROM agent_tasks) + length(?5) <= ?7
             ON CONFLICT(id) DO NOTHING",
            params![task.id.to_string(), owner.to_string(), delegated_by.map(|id| id.to_string()), task.state.to_string(), bytes, MAX_TASKS, MAX_BYTES],
        ).map_err(db)?;
        match written {
            1 => Ok(()),
            _ if self.contains(task.id)? => Err(AgentTaskStoreError::Duplicate),
            _ => Err(AgentTaskStoreError::Capacity),
        }
    }

    pub(crate) fn transition(
        &self,
        task: &Task,
        owner: AgentId,
        previous: TaskState,
        result: Option<&TaskExecutionResult>,
    ) -> Result<(), AgentTaskStoreError> {
        let bytes = encode(task)?;
        let result = result.map(encode).transpose()?;
        let written = self.0.lock().map_err(|_| AgentTaskStoreError::Database)?.execute(
            "UPDATE agent_tasks SET state=?4, task=?5, result=?6 WHERE id=?1 AND owner=?2 AND state=?3
             AND (SELECT coalesce(sum(length(task)+coalesce(length(result),0)),0) FROM agent_tasks)
                   - length(task) - coalesce(length(result),0) + length(?5) + coalesce(length(?6),0) <= ?7",
            params![task.id.to_string(), owner.to_string(), previous.to_string(), task.state.to_string(), bytes, result, MAX_BYTES],
        ).map_err(db)?;
        if written != 1 {
            return Err(AgentTaskStoreError::InvalidCheckpoint);
        }
        Ok(())
    }

    /// Inspect the retained owner and outcome after acquiring the exclusive connection.
    pub fn inspect(path: &Path, task_id: TaskId) -> Result<StoredAgentTask, AgentTaskStoreError> {
        let store = Self::connect(path, false)?;
        let connection = store.0.lock().map_err(|_| AgentTaskStoreError::Database)?;
        let (owner, delegated_by, state, bytes, result) = connection
            .query_row(
                "SELECT owner, delegated_by, state,
             CASE WHEN length(task)<=?2 THEN task END,
             CASE WHEN length(result)>?2 THEN 0 ELSE result END
             FROM agent_tasks WHERE id=?1",
                params![task_id.to_string(), MAX_BYTES],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, Option<Vec<u8>>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(db)?
            .ok_or(AgentTaskStoreError::NotFound)?;
        let owner = owner
            .parse()
            .map_err(|_| AgentTaskStoreError::InvalidCheckpoint)?;
        let delegated_by = delegated_by
            .map(|id| id.parse())
            .transpose()
            .map_err(|_| AgentTaskStoreError::InvalidCheckpoint)?;
        let task: Task =
            serde_json::from_slice(&bytes).map_err(|_| AgentTaskStoreError::InvalidCheckpoint)?;
        let result: Option<TaskExecutionResult> = result
            .as_deref()
            .map(serde_json::from_slice)
            .transpose()
            .map_err(|_| AgentTaskStoreError::InvalidCheckpoint)?;
        if task.id != task_id
            || task.validate().is_err()
            || task.state.to_string() != state
            || result.as_ref().is_some_and(|result| {
                result.agent_id != owner
                    || result.task != task
                    || task.state != TaskState::Completed
                    || result.provider_id != result.response.provider_id
            })
        {
            return Err(AgentTaskStoreError::InvalidCheckpoint);
        }
        let interrupted = matches!(task.state, TaskState::Queued | TaskState::Running);
        Ok(StoredAgentTask {
            task,
            owner,
            delegated_by,
            result,
            interrupted,
        })
    }
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>, AgentTaskStoreError> {
    let bytes = serde_json::to_vec(value).map_err(|_| AgentTaskStoreError::InvalidCheckpoint)?;
    if bytes.len() as i64 > MAX_BYTES {
        return Err(AgentTaskStoreError::Capacity);
    }
    Ok(bytes)
}
