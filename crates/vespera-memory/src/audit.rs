//! Bounded, metadata-only evidence committed alongside memory mutations.
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OpenFlags, params};
use vespera_core::{EventEnvelope, EventId, EventKind};

use crate::{StorageError, schema, types::db};

const RETENTION: i64 = 4096;
const MAX_PAGE: u32 = 256;
const SOURCE: &str = "memory-store";

pub(super) fn renew_source(connection: &Connection) -> Result<(), StorageError> {
    if has_source_table(connection)? {
        connection
            .execute(
                "UPDATE memory_audit_source SET id=?1 WHERE singleton=1",
                [EventId::new().to_string()],
            )
            .map_err(db)?;
    }
    Ok(())
}

fn has_source_table(connection: &Connection) -> Result<bool, StorageError> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='memory_audit_source')",
        [], |row| row.get(0),
    ).map_err(db)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryAuditCursor {
    pub source_id: EventId,
    pub sequence: i64,
}

impl std::str::FromStr for MemoryAuditCursor {
    type Err = StorageError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = StorageError::Invalid("memory audit cursor");
        let (id, sequence) = value.split_once(':').ok_or(invalid)?;
        Ok(Self {
            source_id: id
                .parse()
                .map_err(|_| StorageError::Invalid("memory audit cursor"))?,
            sequence: sequence
                .parse()
                .map_err(|_| StorageError::Invalid("memory audit cursor"))?,
        })
    }
}

impl std::fmt::Display for MemoryAuditCursor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}", self.source_id, self.sequence)
    }
}

pub struct MemoryAuditPage {
    pub events: Vec<EventEnvelope>,
    pub next_cursor: Option<MemoryAuditCursor>,
    pub discarded_through: i64,
    pub has_more: bool,
}

pub(super) fn append(
    connection: &Connection,
    kind: EventKind,
) -> Result<EventEnvelope, StorageError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_audit_source (
            singleton INTEGER PRIMARY KEY CHECK(singleton=1),
            id TEXT NOT NULL
        ) STRICT;
        CREATE TABLE IF NOT EXISTS memory_audit (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            payload BLOB NOT NULL CHECK(length(payload)<=4096)
        ) STRICT;",
        )
        .map_err(db)?;
    let has_source: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_audit_source WHERE singleton=1)",
            [],
            |row| row.get(0),
        )
        .map_err(db)?;
    if !has_source {
        connection
            .execute(
                "INSERT INTO memory_audit_source VALUES (1, ?1)",
                [EventId::new().to_string()],
            )
            .map_err(db)?;
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .ok_or(StorageError::Database)?;
    let event = EventEnvelope::new(SOURCE, timestamp, kind).map_err(|_| StorageError::Database)?;
    let bytes = serde_json::to_vec(&event).map_err(|_| StorageError::Database)?;
    connection
        .execute("INSERT INTO memory_audit(payload) VALUES (?1)", [bytes])
        .map_err(db)?;
    let sequence = connection.last_insert_rowid();
    connection
        .execute(
            "DELETE FROM memory_audit WHERE sequence<=?1",
            [sequence - RETENTION],
        )
        .map_err(db)?;
    Ok(event)
}

/// Read retained, content-free events across namespaces as a trusted filesystem operator.
pub fn read_memory_audit(
    path: &Path,
    after: Option<MemoryAuditCursor>,
    limit: u32,
) -> Result<MemoryAuditPage, StorageError> {
    if !(1..=MAX_PAGE).contains(&limit) || after.is_some_and(|cursor| cursor.sequence < 0) {
        return Err(StorageError::Invalid("memory audit page"));
    }
    let path = std::path::absolute(path).map_err(|_| StorageError::Invalid("path"))?;
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db)?;
    schema::open_existing(&mut connection)?;
    let tx = connection.transaction().map_err(db)?;
    if !has_source_table(&tx)? {
        if after.is_some() {
            return Err(StorageError::AuditForeignCursor);
        }
        return Ok(MemoryAuditPage {
            events: Vec::new(),
            next_cursor: None,
            discarded_through: 0,
            has_more: false,
        });
    }
    let source: String = tx
        .query_row(
            "SELECT id FROM memory_audit_source WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(db)?;
    let source_id: EventId = source.parse().map_err(|_| StorageError::Database)?;
    let (oldest, latest): (i64, i64) = tx
        .query_row(
            "SELECT coalesce(min(sequence),1),coalesce(max(sequence),0) FROM memory_audit",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(db)?;
    if oldest < 1 || latest < 0 {
        return Err(StorageError::Database);
    }
    let discarded_through = oldest - 1;
    let mut sequence = match after {
        Some(cursor) if cursor.source_id != source_id => {
            return Err(StorageError::AuditForeignCursor);
        }
        Some(cursor) if cursor.sequence > latest => return Err(StorageError::AuditFutureCursor),
        Some(cursor) if cursor.sequence < discarded_through => {
            return Err(StorageError::AuditExpired { discarded_through });
        }
        Some(cursor) => cursor.sequence,
        None => discarded_through,
    };
    let mut events = Vec::new();
    {
        let mut statement = tx
            .prepare(
                "SELECT sequence,CASE WHEN length(payload)<=4096 THEN payload END
             FROM memory_audit WHERE sequence>?1 ORDER BY sequence LIMIT ?2",
            )
            .map_err(db)?;
        let mut rows = statement.query(params![sequence, limit]).map_err(db)?;
        while let Some(row) = rows.next().map_err(db)? {
            sequence = row.get(0).map_err(db)?;
            let bytes: Vec<u8> = row.get(1).map_err(db)?;
            let event: EventEnvelope =
                serde_json::from_slice(&bytes).map_err(|_| StorageError::Database)?;
            if event.source != SOURCE
                || event.correlation_task_id.is_some()
                || !matches!(
                    event.kind,
                    EventKind::AgentMemoryUpdated { .. } | EventKind::AgentMemoryDeleted { .. }
                )
            {
                return Err(StorageError::Database);
            }
            events.push(event);
        }
    }
    tx.commit().map_err(db)?;
    Ok(MemoryAuditPage {
        events,
        next_cursor: Some(MemoryAuditCursor {
            source_id,
            sequence,
        }),
        discarded_through,
        has_more: sequence < latest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vespera_core::{AgentId, MemoryId};

    #[test]
    fn audit_bounds_pages_and_reports_expired_cursors() {
        let path = std::env::temp_dir().join(format!("vespera-audit-{}.sqlite3", EventId::new()));
        let mut connection = Connection::open(&path).unwrap();
        schema::initialize(&mut connection).unwrap();
        let tx = connection.transaction().unwrap();
        let kind = EventKind::AgentMemoryDeleted {
            agent_id: AgentId::new(),
            memory_id: MemoryId::new(),
        };
        for _ in 0..=RETENTION {
            append(&tx, kind.clone()).unwrap();
        }
        tx.commit().unwrap();
        drop(connection);
        let page = read_memory_audit(&path, None, 2).unwrap();
        assert_eq!(page.events.len(), 2);
        assert_eq!(page.discarded_through, 1);
        assert!(page.has_more);
        let cursor = page.next_cursor.unwrap();
        assert_eq!(cursor.sequence, 3);
        let expired = MemoryAuditCursor {
            sequence: 0,
            ..cursor
        };
        assert!(matches!(
            read_memory_audit(&path, Some(expired), 2),
            Err(StorageError::AuditExpired {
                discarded_through: 1
            })
        ));
        let next = read_memory_audit(&path, Some(cursor), 2).unwrap();
        assert_eq!(next.next_cursor.unwrap().sequence, 5);
        std::fs::remove_file(path).unwrap();
    }
}
