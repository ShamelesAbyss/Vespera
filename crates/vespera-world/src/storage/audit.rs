//! Metadata-only evidence committed with authoritative simulation ticks.
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use vespera_core::{EventEnvelope, EventId, EventKind};

use super::db;
use crate::WorldError;

const RETAINED_ADVANCES: u32 = 512;
const SOURCE: &str = "world-runtime";

pub(super) fn append(
    connection: &Connection,
    revision: u64,
    tick: u64,
) -> Result<EventEnvelope, WorldError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS world_advance_events (
            revision BLOB PRIMARY KEY CHECK(length(revision) = 8),
            simulation_id TEXT NOT NULL,
            payload BLOB NOT NULL CHECK(length(payload) <= 4096)
        ) WITHOUT ROWID;",
        )
        .map_err(db)?;
    let simulation_id = connection
        .query_row(
            "SELECT simulation_id FROM world_advance_events ORDER BY revision DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(db)?;
    let simulation_id = match simulation_id {
        Some(id) => id.parse::<EventId>().map_err(|_| WorldError::Format)?,
        None => EventId::new(),
    }
    .to_string();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .ok_or(WorldError::Storage)?;
    let event = EventEnvelope::new(
        SOURCE,
        timestamp,
        EventKind::SimulationAdvanced {
            simulation_id: simulation_id.clone(),
            tick,
        },
    )
    .map_err(|_| WorldError::Format)?;
    let payload = serde_json::to_vec(&event).map_err(|_| WorldError::Format)?;
    connection
        .execute(
            "INSERT INTO world_advance_events VALUES (?1, ?2, ?3)",
            params![revision.to_be_bytes(), simulation_id, payload],
        )
        .map_err(db)?;
    connection
        .execute(
            "DELETE FROM world_advance_events WHERE revision <= (
            SELECT revision FROM world_advance_events ORDER BY revision DESC LIMIT 1 OFFSET ?1
        )",
            [RETAINED_ADVANCES],
        )
        .map_err(db)?;
    Ok(event)
}

pub(super) fn read(
    connection: &Connection,
    revision: u64,
) -> Result<Option<EventEnvelope>, WorldError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='world_advance_events')",
        [],
        |row| row.get(0),
    ).map_err(db)?;
    if !exists {
        return Ok(None);
    }
    let row = connection
        .query_row(
            "SELECT simulation_id, CASE WHEN length(payload) <= 4096 THEN payload END
         FROM world_advance_events WHERE revision=?1",
            [revision.to_be_bytes()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()
        .map_err(db)?;
    let Some((simulation, payload)) = row else {
        return Ok(None);
    };
    let event: EventEnvelope = serde_json::from_slice(&payload).map_err(|_| WorldError::Format)?;
    if event.source != SOURCE
        || event.correlation_task_id.is_some()
        || simulation.parse::<EventId>().is_err()
        || !matches!(&event.kind, EventKind::SimulationAdvanced { simulation_id, tick }
            if simulation_id == &simulation && *tick > 0 && *tick <= revision)
    {
        return Err(WorldError::Format);
    }
    Ok(Some(event))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_old_events_and_preserves_the_simulation_id() {
        let connection = Connection::open_in_memory().unwrap();
        let first = append(&connection, 1, 1).unwrap();
        let mut last = first.clone();
        let latest = u64::from(RETAINED_ADVANCES) + 1;
        for revision in 2..=latest {
            last = append(&connection, revision, revision).unwrap();
        }
        let simulation = |event: EventEnvelope| match event.kind {
            EventKind::SimulationAdvanced { simulation_id, .. } => simulation_id,
            _ => unreachable!(),
        };
        assert_eq!(simulation(first), simulation(last.clone()));
        assert!(read(&connection, 1).unwrap().is_none());
        assert!(read(&connection, 2).unwrap().is_some());
        assert_eq!(read(&connection, latest).unwrap(), Some(last));
    }
}
