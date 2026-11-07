use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use crate::{World, WorldError};

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
