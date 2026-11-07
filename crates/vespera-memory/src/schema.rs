use crate::StorageError;
use crate::types::db;
use rusqlite::{Connection, TransactionBehavior};
pub(crate) const APPLICATION_ID: i64 = 0x5653504d; // VSPM
pub(crate) const VERSION: i64 = 1;

pub(crate) fn initialize(connection: &mut Connection) -> Result<(), StorageError> {
    setup(connection, true)
}
pub(crate) fn open_existing(connection: &mut Connection) -> Result<(), StorageError> {
    setup(connection, false)
}
fn setup(connection: &mut Connection, allow_initialize: bool) -> Result<(), StorageError> {
    connection
        .busy_timeout(std::time::Duration::from_secs(1))
        .map_err(db)?;
    // Inspect identity before any persistent pragma/schema change to an existing file.
    let version = {
        // Header identity and schema must come from one snapshot: an initializer
        // can otherwise commit between separate application_id/user_version reads.
        let tx = connection.transaction().map_err(db)?;
        let version = inspect(&tx)?;
        tx.commit().map_err(db)?;
        version
    };
    if version == 0 && !allow_initialize {
        return Err(StorageError::WrongDatabase);
    }
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA synchronous=FULL;",
        )
        .map_err(db)?;
    if version == 0 {
        migrate(connection, SCHEMA)?;
    }
    // Fail closed when expected tables are missing instead of silently recreating data.
    connection.prepare("SELECT sequence,owner_sequence,id,owner,layer,content,content_bytes,created_at,source_task_id,origin,method FROM memories LIMIT 0").map_err(db)?;
    connection
        .prepare("SELECT owner,next_sequence FROM namespace_counters LIMIT 0")
        .map_err(db)?;
    connection
        .prepare("SELECT summary_id,source_id FROM memory_sources LIMIT 0")
        .map_err(db)?;
    connection
        .prepare("SELECT rowid FROM memory_fts LIMIT 0")
        .map_err(db)?;
    Ok(())
}

fn inspect(connection: &Connection) -> Result<i64, StorageError> {
    let application: i64 = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(db)?;
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db)?;
    if application != 0 && application != APPLICATION_ID {
        return Err(StorageError::WrongDatabase);
    }
    if !(0..=VERSION).contains(&version) {
        return Err(StorageError::UnsupportedSchema);
    }
    if application == 0 || version == 0 {
        let objects: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )
            .map_err(db)?;
        if version != 0 || objects != 0 {
            return Err(StorageError::WrongDatabase);
        }
    }
    Ok(version)
}

fn migrate(connection: &mut Connection, ddl: &str) -> Result<(), StorageError> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db)?;
    // Recheck after acquiring the writer lock: another opener may have initialized
    // the file since the initial inspection. Never label concurrently created foreign data.
    if inspect(&tx)? == 0 {
        tx.execute_batch(ddl).map_err(db)?;
        tx.pragma_update(None, "application_id", APPLICATION_ID)
            .map_err(db)?;
        tx.pragma_update(None, "user_version", VERSION)
            .map_err(db)?;
    }
    tx.commit().map_err(db)
}

const SCHEMA: &str = "
CREATE TABLE namespace_counters(owner TEXT PRIMARY KEY,next_sequence INTEGER NOT NULL CHECK(next_sequence>0));
CREATE TABLE memories (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 owner_sequence INTEGER NOT NULL,
 id TEXT NOT NULL UNIQUE,
 owner TEXT NOT NULL,
 layer TEXT NOT NULL CHECK(layer IN ('long_term','episodic','working','relationship','goal','observation')),
 content TEXT NOT NULL,
 content_bytes INTEGER NOT NULL CHECK(content_bytes > 0 AND content_bytes=length(CAST(content AS BLOB))),
 created_at INTEGER NOT NULL CHECK(created_at >= 0),
 source_task_id TEXT,
 origin TEXT NOT NULL CHECK(origin IN ('observation','import','generated','derived')),
 method TEXT,
 CHECK((origin='derived' AND method IS NOT NULL) OR (origin!='derived' AND method IS NULL)),
 UNIQUE(owner,owner_sequence)
);
CREATE INDEX memories_owner_sequence ON memories(owner,owner_sequence);
CREATE INDEX memories_owner_layer ON memories(owner,layer,owner_sequence);
CREATE TABLE memory_sources (
 summary_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
 source_id TEXT NOT NULL REFERENCES memories(id) ON DELETE RESTRICT,
 PRIMARY KEY(summary_id,source_id), CHECK(summary_id != source_id)
) WITHOUT ROWID;
CREATE INDEX memory_source_reverse ON memory_sources(source_id);
CREATE VIRTUAL TABLE memory_fts USING fts5(content,content='memories',content_rowid='sequence',tokenize='unicode61');
";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_failure_rolls_back_tables_indexes_and_identity() {
        let mut connection = Connection::open_in_memory().unwrap();
        let invalid = format!("{SCHEMA}\nTHIS IS DELIBERATELY INVALID SQL;");
        assert!(migrate(&mut connection, &invalid).is_err());
        assert_eq!(inspect(&connection).unwrap(), 0);
        let objects: i64 = connection
            .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
            .unwrap();
        assert_eq!(objects, 0);
        initialize(&mut connection).unwrap();
        assert_eq!(inspect(&connection).unwrap(), VERSION);
    }
}
