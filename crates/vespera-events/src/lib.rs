//! Local operator-owned event history. Calls perform synchronous database I/O.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use vespera_core::{EventEnvelope, EventKind};

const APPLICATION_ID: i64 = 0x56535045;
const VERSION: i64 = 1;
pub const MAX_RETENTION: u32 = 100_000;
pub const MAX_PAGE: u32 = 256;

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("invalid event journal input: {0}")]
    Invalid(&'static str),
    #[error("file is not a supported Vespera event journal")]
    WrongDatabase,
    #[error("event journal operation failed")]
    Database,
    #[error("event ID conflicts with a retained event")]
    Conflict,
    #[error("cursor belongs to another journal")]
    ForeignCursor,
    #[error("cursor is ahead of the journal")]
    FutureCursor,
    #[error("cursor expired after retention removed events through sequence {discarded_through}")]
    Expired { discarded_through: i64 },
}

fn db(_: rusqlite::Error) -> JournalError {
    JournalError::Database
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub journal_id: Uuid,
    pub sequence: i64,
}

impl std::str::FromStr for Cursor {
    type Err = JournalError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (id, sequence) = value
            .split_once(':')
            .ok_or(JournalError::Invalid("cursor"))?;
        Ok(Self {
            journal_id: Uuid::parse_str(id)
                .map_err(|_| JournalError::Invalid("cursor journal ID"))?,
            sequence: sequence
                .parse()
                .map_err(|_| JournalError::Invalid("cursor sequence"))?,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub cursor: Cursor,
    pub event: EventEnvelope,
}

#[derive(Debug, Serialize)]
pub struct Page {
    pub entries: Vec<Entry>,
    pub next_cursor: Cursor,
    pub discarded_through: i64,
    pub has_more: bool,
}

pub struct Journal(Connection);

impl Journal {
    pub fn initialize(path: &Path, retention: u32) -> Result<Self, JournalError> {
        if !(1..=MAX_RETENTION).contains(&retention) {
            return Err(JournalError::Invalid("retention"));
        }
        let mut connection = open(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let identity = identity(&tx)?;
        if identity == (0, 0) {
            let objects: i64 = tx
                .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
                .map_err(db)?;
            if objects != 0 {
                return Err(JournalError::WrongDatabase);
            }
            tx.execute_batch(
                "CREATE TABLE journal_meta (
                    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                    journal_id TEXT NOT NULL,
                    retention INTEGER NOT NULL CHECK(retention BETWEEN 1 AND 100000),
                    discarded_through INTEGER NOT NULL CHECK(discarded_through>=0)
                 );
                 CREATE TABLE journal_events (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    event_id TEXT NOT NULL UNIQUE,
                    payload TEXT NOT NULL CHECK(length(CAST(payload AS BLOB))<=4096)
                 );",
            )
            .map_err(db)?;
            tx.execute(
                "INSERT INTO journal_meta VALUES (1, ?1, ?2, 0)",
                params![Uuid::new_v4().to_string(), retention],
            )
            .map_err(db)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(db)?;
            tx.pragma_update(None, "user_version", VERSION)
                .map_err(db)?;
        } else if identity != (APPLICATION_ID, VERSION) {
            return Err(JournalError::WrongDatabase);
        }
        let (_, configured, _) = metadata(&tx)?;
        if configured != retention {
            return Err(JournalError::Invalid("existing journal retention"));
        }
        tx.prepare("SELECT sequence,event_id,payload FROM journal_events LIMIT 0")
            .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(Self(connection))
    }

    pub fn open_writer(path: &Path) -> Result<Self, JournalError> {
        Self::open_existing(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
    }

    pub fn open_reader(path: &Path) -> Result<Self, JournalError> {
        Self::open_existing(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
    }

    fn open_existing(path: &Path, flags: OpenFlags) -> Result<Self, JournalError> {
        let mut connection = open(path, flags)?;
        let tx = connection.transaction().map_err(db)?;
        if identity(&tx)? != (APPLICATION_ID, VERSION) {
            return Err(JournalError::WrongDatabase);
        }
        metadata(&tx)?;
        tx.prepare("SELECT sequence,event_id,payload FROM journal_events LIMIT 0")
            .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(Self(connection))
    }

    /// Atomically append and trim history. Equal retained events are idempotent.
    pub fn append(&mut self, event: &EventEnvelope) -> Result<Cursor, JournalError> {
        self.append_batch(std::slice::from_ref(event))
    }

    /// Append one bounded batch and trim in a single transaction. Returns the greatest
    /// sequence of the submitted events, including any matching retained entries.
    pub fn append_batch(&mut self, events: &[EventEnvelope]) -> Result<Cursor, JournalError> {
        if events.is_empty() || events.len() > MAX_PAGE as usize {
            return Err(JournalError::Invalid("batch size"));
        }
        let prepared = events
            .iter()
            .map(prepare_event)
            .collect::<Result<Vec<_>, _>>()?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let (journal_id, retention, floor) = metadata(&tx)?;
        let mut newest = 0;
        for (id, payload) in prepared {
            let existing: Option<(i64, String)> = tx
                .query_row(
                    "SELECT sequence,payload FROM journal_events WHERE event_id=?1",
                    [&id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(db)?;
            let sequence = if let Some((sequence, previous)) = existing {
                if previous != payload {
                    return Err(JournalError::Conflict);
                }
                sequence
            } else {
                tx.execute(
                    "INSERT INTO journal_events(event_id,payload) VALUES(?1,?2)",
                    params![id, payload],
                )
                .map_err(db)?;
                tx.last_insert_rowid()
            };
            newest = newest.max(sequence);
        }
        let cutoff = newest - i64::from(retention);
        if cutoff > floor {
            tx.execute("DELETE FROM journal_events WHERE sequence<=?1", [cutoff])
                .map_err(db)?;
            tx.execute(
                "UPDATE journal_meta SET discarded_through=?1 WHERE singleton=1",
                [cutoff],
            )
            .map_err(db)?;
        }
        tx.commit().map_err(db)?;
        Ok(Cursor {
            journal_id,
            sequence: newest,
        })
    }

    /// Read a bounded page from one snapshot. No subscriber queue is allocated.
    pub fn replay(&mut self, after: Option<Cursor>, limit: u32) -> Result<Page, JournalError> {
        if !(1..=MAX_PAGE).contains(&limit) {
            return Err(JournalError::Invalid("page limit"));
        }
        let tx = self.0.transaction().map_err(db)?;
        let (journal_id, _, discarded_through) = metadata(&tx)?;
        let latest: i64 = tx
            .query_row(
                "SELECT coalesce(max(sequence),0) FROM journal_events",
                [],
                |row| row.get(0),
            )
            .map_err(db)?;
        let start = match after {
            Some(cursor) if cursor.journal_id != journal_id => {
                return Err(JournalError::ForeignCursor);
            }
            Some(cursor) if cursor.sequence < 0 => {
                return Err(JournalError::Invalid("cursor sequence"));
            }
            Some(cursor) if cursor.sequence < discarded_through => {
                return Err(JournalError::Expired { discarded_through });
            }
            Some(cursor) if cursor.sequence > latest => return Err(JournalError::FutureCursor),
            Some(cursor) => cursor.sequence,
            None => discarded_through,
        };
        let mut statement = tx.prepare("SELECT sequence,payload FROM journal_events WHERE sequence>?1 ORDER BY sequence LIMIT ?2").map_err(db)?;
        let mut rows = statement.query(params![start, limit + 1]).map_err(db)?;
        let mut entries = Vec::new();
        let mut has_more = false;
        while let Some(row) = rows.next().map_err(db)? {
            if entries.len() == limit as usize {
                has_more = true;
                break;
            }
            let sequence = row.get(0).map_err(db)?;
            let payload: String = row.get(1).map_err(db)?;
            let event = serde_json::from_str(&payload).map_err(|_| JournalError::Database)?;
            entries.push(Entry {
                cursor: Cursor {
                    journal_id,
                    sequence,
                },
                event,
            });
        }
        let next_cursor = entries.last().map_or(
            Cursor {
                journal_id,
                sequence: start,
            },
            |entry| entry.cursor,
        );
        Ok(Page {
            entries,
            next_cursor,
            discarded_through,
            has_more,
        })
    }
}

fn prepare_event(event: &EventEnvelope) -> Result<(String, String), JournalError> {
    event
        .validate()
        .map_err(|_| JournalError::Invalid("envelope"))?;
    if !label(&event.source) {
        return Err(JournalError::Invalid("source label"));
    }
    if let EventKind::SimulationAdvanced { simulation_id, .. } = &event.kind
        && Uuid::parse_str(simulation_id).is_err()
    {
        return Err(JournalError::Invalid("simulation ID"));
    }
    let payload = serde_json::to_string(event).map_err(|_| JournalError::Invalid("payload"))?;
    if payload.len() > 4096 {
        return Err(JournalError::Invalid("payload size"));
    }
    Ok((event.event_id.to_string(), payload))
}

fn open(path: &Path, flags: OpenFlags) -> Result<Connection, JournalError> {
    if path.as_os_str().is_empty()
        || path == Path::new(":memory:")
        || path.to_string_lossy().starts_with("file:")
    {
        return Err(JournalError::Invalid("path"));
    }
    let connection =
        Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(db)?;
    connection
        .busy_timeout(Duration::from_secs(1))
        .map_err(db)?;
    connection
        .execute_batch("PRAGMA trusted_schema=OFF; PRAGMA synchronous=FULL;")
        .map_err(db)?;
    Ok(connection)
}

fn identity(connection: &Connection) -> Result<(i64, i64), JournalError> {
    let application = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(db)?;
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db)?;
    Ok((application, version))
}

fn metadata(connection: &Connection) -> Result<(Uuid, u32, i64), JournalError> {
    let (id, retention, floor): (String, u32, i64) = connection
        .query_row(
            "SELECT journal_id,retention,discarded_through FROM journal_meta WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(db)?;
    let id = Uuid::parse_str(&id).map_err(|_| JournalError::Database)?;
    if id.is_nil() || !(1..=MAX_RETENTION).contains(&retention) || floor < 0 {
        return Err(JournalError::Database);
    }
    Ok((id, retention, floor))
}

fn label(value: &str) -> bool {
    (1..=48).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
