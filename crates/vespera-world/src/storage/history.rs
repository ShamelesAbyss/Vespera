use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{ReplayDirection, db, encode};
use crate::{World, WorldChange, WorldError};

const MAX_COMMITS: i64 = 512;
const MAX_REVISION_SPAN: u64 = 4096;

pub struct Replay {
    pub world: World,
    pub earliest_revision: u64,
    pub latest_revision: u64,
    /// The checkpoint from which this state was reconstructed.
    pub basis_revision: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Entry {
    Change(WorldChange),
    Snapshot(Box<World>),
}

impl Entry {
    fn decode(bytes: &[u8]) -> Result<Self, WorldError> {
        serde_json::from_slice(bytes).map_err(|_| WorldError::Format)
    }

    fn apply(self, world: &mut World, revision: u64) -> Result<bool, WorldError> {
        let anchored = match self {
            Self::Change(change) => {
                change.apply(world).map_err(|_| WorldError::Format)?;
                false
            }
            Self::Snapshot(snapshot) => {
                snapshot.validate().map_err(|_| WorldError::Format)?;
                *world = *snapshot;
                true
            }
        };
        if world.revision != revision {
            return Err(WorldError::Format);
        }
        Ok(anchored)
    }
}

pub(super) fn append(
    tx: &Connection,
    before: &World,
    after: &World,
    change: Option<WorldChange>,
) -> Result<(), WorldError> {
    if after.revision <= before.revision {
        return Err(WorldError::Invalid("history requires increasing revisions"));
    }
    let mut baseline = match base(tx)? {
        Some(base) => base,
        None => {
            tx.execute_batch(
                "CREATE TABLE world_history_base (
                singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                payload BLOB NOT NULL CHECK(length(payload) <= 524288)
            );
            CREATE TABLE world_history (
                revision BLOB PRIMARY KEY CHECK(length(revision) = 8),
                payload BLOB NOT NULL CHECK(length(payload) <= 524320)
            ) WITHOUT ROWID;",
            )
            .map_err(db)?;
            tx.execute(
                "INSERT INTO world_history_base VALUES (1, ?1)",
                [encode(before)?],
            )
            .map_err(db)?;
            before.clone()
        }
    };
    let tip = tx
        .query_row(
            "SELECT revision FROM world_history ORDER BY revision DESC LIMIT 1",
            [],
            |row| row.get::<_, [u8; 8]>(0),
        )
        .optional()
        .map_err(db)?
        .map_or(baseline.revision, u64::from_be_bytes);
    if baseline.revision > tip || tip > before.revision {
        return Err(WorldError::Format);
    }
    if tip < before.revision {
        insert(
            tx,
            before.revision,
            Entry::Snapshot(Box::new(before.clone())),
        )?;
    }
    let entry = change.map_or_else(|| Entry::Snapshot(Box::new(after.clone())), Entry::Change);
    insert(tx, after.revision, entry)?;

    let original = baseline.revision;
    let mut count: i64 = tx
        .query_row("SELECT count(*) FROM world_history", [], |row| row.get(0))
        .map_err(db)?;
    while count > MAX_COMMITS || after.revision - baseline.revision > MAX_REVISION_SPAN {
        let (revision, bytes) = tx
            .query_row(
                "SELECT revision,payload FROM world_history ORDER BY revision LIMIT 1",
                [],
                |row| Ok((row.get::<_, [u8; 8]>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .map_err(db)?;
        if u64::from_be_bytes(revision) <= baseline.revision {
            return Err(WorldError::Format);
        }
        Entry::decode(&bytes)?.apply(&mut baseline, u64::from_be_bytes(revision))?;
        tx.execute("DELETE FROM world_history WHERE revision=?1", [revision])
            .map_err(db)?;
        count -= 1;
    }
    if baseline.revision != original {
        tx.execute(
            "UPDATE world_history_base SET payload=?1 WHERE singleton=1",
            [encode(&baseline)?],
        )
        .map_err(db)?;
    }
    Ok(())
}

pub(super) fn replay(
    tx: &Connection,
    current: World,
    requested: Option<u64>,
) -> Result<Replay, WorldError> {
    let latest = current.revision;
    let revision = requested.unwrap_or(latest);
    let baseline = base(tx)?;
    let earliest = baseline.as_ref().map_or(latest, |world| world.revision);
    let unavailable = || WorldError::ReplayUnavailable {
        revision,
        earliest,
        latest,
    };
    if !(earliest..=latest).contains(&revision) {
        return Err(unavailable());
    }
    let Some(mut world) = baseline else {
        return Ok(Replay {
            world: current,
            earliest_revision: earliest,
            latest_revision: latest,
            basis_revision: latest,
        });
    };
    let mut basis_revision = world.revision;
    let mut statement = tx
        .prepare("SELECT revision,payload FROM world_history WHERE revision<=?1 ORDER BY revision")
        .map_err(db)?;
    let mut rows = statement.query([revision.to_be_bytes()]).map_err(db)?;
    while let Some(row) = rows.next().map_err(db)? {
        let boundary = u64::from_be_bytes(row.get(0).map_err(db)?);
        if boundary <= world.revision {
            return Err(WorldError::Format);
        }
        let bytes: Vec<u8> = row.get(1).map_err(db)?;
        if Entry::decode(&bytes)?.apply(&mut world, boundary)? {
            basis_revision = boundary;
        }
    }
    if world.revision != revision {
        if revision != latest {
            return Err(unavailable());
        }
        // A legacy writer may have committed a newer live snapshot without recording changes.
        world = current.clone();
        basis_revision = latest;
    }
    if revision == latest && world != current {
        return Err(WorldError::Format);
    }
    Ok(Replay {
        world,
        earliest_revision: earliest,
        latest_revision: latest,
        basis_revision,
    })
}

pub(super) fn neighbor(
    tx: &Connection,
    current: World,
    from: u64,
    direction: ReplayDirection,
) -> Result<Option<Replay>, WorldError> {
    let latest = current.revision;
    let baseline = base(tx)?;
    let revision = match direction {
        ReplayDirection::Previous if from > latest => Some(latest),
        ReplayDirection::Previous => {
            if let Some(baseline) = baseline.filter(|base| base.revision < from) {
                tx.query_row(
                    "SELECT revision FROM world_history WHERE revision < ?1 ORDER BY revision DESC LIMIT 1",
                    [from.to_be_bytes()], |row| row.get::<_, [u8; 8]>(0),
                ).optional().map_err(db)?
                    .map(u64::from_be_bytes).or(Some(baseline.revision))
            } else {
                None
            }
        }
        ReplayDirection::Next if from >= latest => None,
        ReplayDirection::Next => {
            if let Some(baseline) = baseline {
                if from < baseline.revision {
                    Some(baseline.revision)
                } else {
                    tx.query_row(
                        "SELECT revision FROM world_history WHERE revision > ?1 ORDER BY revision LIMIT 1",
                        [from.to_be_bytes()], |row| row.get::<_, [u8; 8]>(0),
                    ).optional().map_err(db)?
                        .map(u64::from_be_bytes).or(Some(latest))
                }
            } else {
                Some(latest)
            }
        }
    };
    revision
        .map(|target| replay(tx, current, Some(target)))
        .transpose()
}

fn insert(tx: &Connection, revision: u64, entry: Entry) -> Result<(), WorldError> {
    let bytes = serde_json::to_vec(&entry).map_err(|_| WorldError::Format)?;
    tx.execute(
        "INSERT INTO world_history VALUES (?1, ?2)",
        params![revision.to_be_bytes(), bytes],
    )
    .map_err(db)?;
    Ok(())
}

fn base(tx: &Connection) -> Result<Option<World>, WorldError> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='world_history_base')",
        [], |row| row.get(0),
    ).map_err(db)?;
    if !exists {
        return Ok(None);
    }
    let bytes: Vec<u8> = tx.query_row("SELECT CASE WHEN length(payload)<=524288 THEN payload END FROM world_history_base WHERE singleton=1",
        [], |row| row.get(0)).map_err(db)?;
    let world: World = serde_json::from_slice(&bytes).map_err(|_| WorldError::Format)?;
    world.validate().map_err(|_| WorldError::Format)?;
    Ok(Some(world))
}
