use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

use crate::{ReasoningStatus, World, WorldError};
use vespera_core::TaskId;

const APPLICATION_ID: i64 = 0x5653574c;
const VERSION: i64 = 1;
const MAX_BYTES: i64 = 512 * 1024;

/// Single-world SQLite storage. Each update loads and commits one validated state.
pub struct WorldStore;

impl WorldStore {
    pub fn initialize(path: &Path, world: &World) -> Result<(), WorldError> {
        let bytes = encode(world)?;
        let mut connection = open(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        match identity(&tx)? {
            (APPLICATION_ID, VERSION) => return Err(WorldError::AlreadyExists),
            (0, 0) => {}
            _ => return Err(WorldError::Format),
        }
        let objects: i64 = tx
            .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
            .map_err(db)?;
        if objects != 0 {
            return Err(WorldError::Format);
        }
        tx.execute_batch(
            "CREATE TABLE world_state (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                payload BLOB NOT NULL CHECK(length(payload) <= 524288)
             );",
        )
        .map_err(db)?;
        tx.execute("INSERT INTO world_state VALUES (1, ?1)", [bytes])
            .map_err(db)?;
        tx.pragma_update(None, "application_id", APPLICATION_ID)
            .map_err(db)?;
        tx.pragma_update(None, "user_version", VERSION)
            .map_err(db)?;
        tx.commit().map_err(db)
    }

    pub fn read(path: &Path) -> Result<World, WorldError> {
        let mut connection = open(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let tx = connection.transaction().map_err(db)?;
        let world = read_state(&tx)?;
        tx.commit().map_err(db)?;
        Ok(world)
    }

    /// Read authoritative state and fresh runtime evidence from the same SQLite snapshot.
    pub fn read_observed(
        path: &Path,
        unix_ms: u64,
    ) -> Result<(World, Option<ReasoningStatus>), WorldError> {
        let mut connection = open(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let tx = connection.transaction().map_err(db)?;
        let world = read_state(&tx)?;
        let reasoning = read_reasoning(&tx)?.filter(|status| {
            status.revision == world.revision
                && status.is_fresh(unix_ms)
                && world
                    .residents
                    .iter()
                    .any(|resident| resident.id == status.resident && resident.active)
        });
        tx.commit().map_err(db)?;
        Ok((world, reasoning))
    }

    /// Publish a heartbeat without changing the world revision or tick.
    pub fn observe_reasoning(
        path: &Path,
        observed: &World,
        status: ReasoningStatus,
    ) -> Result<(), WorldError> {
        let mut connection = open(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        if read_state(&tx)? != *observed || status.revision != observed.revision {
            return Err(WorldError::StaleRevision);
        }
        if !observed
            .residents
            .iter()
            .any(|resident| resident.id == status.resident && resident.active)
        {
            return Err(WorldError::Invalid("reasoning resident must be active"));
        }
        if read_reasoning(&tx)?.is_some_and(|prior| {
            prior.revision == status.revision
                && prior.task_id != status.task_id
                && prior.is_fresh(status.observed_unix_ms)
        }) {
            return Err(WorldError::Invalid(
                "world already has an observed reasoning request",
            ));
        }
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS world_reasoning (
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
            task_id TEXT NOT NULL,
            payload BLOB NOT NULL CHECK(length(payload) <= 2048)
        );",
        )
        .map_err(db)?;
        let payload = serde_json::to_vec(&status).map_err(|_| WorldError::Format)?;
        tx.execute(
            "INSERT OR REPLACE INTO world_reasoning VALUES (1, ?1, ?2)",
            params![status.task_id.to_string(), payload],
        )
        .map_err(db)?;
        tx.commit().map_err(db)
    }

    /// Remove only this task's heartbeat. A newer writer's observation is unaffected.
    pub fn clear_reasoning(path: &Path, task_id: TaskId) -> Result<(), WorldError> {
        let connection = open(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        if identity(&connection)? != (APPLICATION_ID, VERSION) {
            return Err(WorldError::Format);
        }
        if has_reasoning_table(&connection)? {
            connection
                .execute(
                    "DELETE FROM world_reasoning WHERE singleton = 1 AND task_id = ?1",
                    [task_id.to_string()],
                )
                .map_err(db)?;
        }
        Ok(())
    }

    pub fn update(
        path: &Path,
        operation: impl FnOnce(&mut World) -> Result<(), WorldError>,
    ) -> Result<World, WorldError> {
        let mut connection = open(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let mut world = read_state(&tx)?;
        operation(&mut world)?;
        let bytes = encode(&world)?;
        tx.execute(
            "UPDATE world_state SET payload = ?1 WHERE singleton = 1",
            [bytes],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(world)
    }
}

fn has_reasoning_table(connection: &Connection) -> Result<bool, WorldError> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'world_reasoning')",
        [], |row| row.get(0),
    ).map_err(db)
}

fn read_reasoning(connection: &Connection) -> Result<Option<ReasoningStatus>, WorldError> {
    if !has_reasoning_table(connection)? {
        return Ok(None);
    }
    let bytes: Option<Vec<u8>> = connection.query_row(
        "SELECT CASE WHEN length(payload) <= 2048 THEN payload END FROM world_reasoning WHERE singleton = 1",
        [], |row| row.get(0),
    ).optional().map_err(db)?;
    bytes
        .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| WorldError::Format))
        .transpose()
}

fn open(path: &Path, flags: OpenFlags) -> Result<Connection, WorldError> {
    let connection = Connection::open_with_flags(path, flags).map_err(db)?;
    connection
        .busy_timeout(Duration::from_secs(2))
        .map_err(db)?;
    connection
        .pragma_update(None, "trusted_schema", "OFF")
        .map_err(db)?;
    Ok(connection)
}

fn identity(connection: &Connection) -> Result<(i64, i64), WorldError> {
    let id = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(db)?;
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db)?;
    Ok((id, version))
}

fn read_state(connection: &Connection) -> Result<World, WorldError> {
    if identity(connection)? != (APPLICATION_ID, VERSION) {
        return Err(WorldError::Format);
    }
    let bytes: Vec<u8> = connection.query_row(
        "SELECT CASE WHEN length(payload) <= ?1 THEN payload END FROM world_state WHERE singleton = 1",
        [MAX_BYTES],
        |row| row.get(0),
    ).map_err(db)?;
    let world: World = serde_json::from_slice(&bytes).map_err(|_| WorldError::Format)?;
    world.validate().map_err(|_| WorldError::Format)?;
    Ok(world)
}

fn encode(world: &World) -> Result<Vec<u8>, WorldError> {
    world.validate()?;
    let bytes = serde_json::to_vec(world).map_err(|_| WorldError::Format)?;
    if bytes.len() as i64 > MAX_BYTES {
        return Err(WorldError::Invalid("snapshot size"));
    }
    Ok(bytes)
}

fn db(_: rusqlite::Error) -> WorldError {
    WorldError::Storage
}
