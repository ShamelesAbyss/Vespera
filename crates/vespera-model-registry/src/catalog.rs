use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use serde::Serialize;
use thiserror::Error;
use vespera_core::ModelDescriptor;

const APPLICATION_ID: i64 = 0x56534d43;
const VERSION: i64 = 1;
const MAX_DESCRIPTOR_BYTES: usize = 64 * 1024;

/// Failure to load or commit the durable catalog. Diagnostics omit model payloads.
#[derive(Debug, Error)]
pub enum CatalogError {
    /// SQLite operation failed.
    #[error("model catalog database operation failed")]
    Database,
    /// The file is foreign, unsupported or contains invalid catalog rows.
    #[error("invalid or unsupported model catalog")]
    Format,
    /// Initialization would overwrite an existing catalog.
    #[error("model catalog is already initialized")]
    AlreadyExists,
    /// Input failed descriptor validation or uniqueness checks.
    #[error("invalid, duplicate or oversized model descriptor")]
    InvalidModel,
    /// Another writer committed before this edit.
    #[error("model catalog revision changed (expected {expected}, actual {actual})")]
    StaleRevision {
        /// Revision the caller edited.
        expected: u64,
        /// Current committed revision.
        actual: u64,
    },
    /// A stable UUID was reused for a different provider binding, including after removal.
    #[error("model catalog edit changes an existing provider binding")]
    BindingChanged,
}

/// Persisted operator metadata at one committed revision.
#[derive(Debug, Serialize)]
pub struct CatalogSnapshot {
    /// Revision used to fence subsequent edits.
    pub revision: u64,
    /// The complete ordered inventory of declared models.
    pub models: Vec<ModelDescriptor>,
}

/// SQLite storage for declared model metadata, separate from health and residency.
pub struct Catalog;

impl Catalog {
    /// Seed a new catalog from an operator inventory without starting providers.
    pub fn initialize(path: &Path, models: &[ModelDescriptor]) -> Result<(), CatalogError> {
        let payloads = encode(models)?;
        let mut connection = open(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        match identity(&tx)? {
            (APPLICATION_ID, VERSION) => return Err(CatalogError::AlreadyExists),
            (0, 0) => {}
            _ => return Err(CatalogError::Format),
        }
        let objects: i64 = tx
            .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
            .map_err(db)?;
        if objects != 0 {
            return Err(CatalogError::Format);
        }
        tx.execute_batch(
            "CREATE TABLE catalog_state (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), revision INTEGER NOT NULL CHECK(revision >= 0));
             INSERT INTO catalog_state VALUES (1, 0);
             CREATE TABLE catalog_models (position INTEGER PRIMARY KEY, payload BLOB NOT NULL CHECK(length(payload) <= 65536));
             CREATE TABLE catalog_bindings (model_id TEXT PRIMARY KEY NOT NULL, provider_id TEXT NOT NULL, identifier TEXT NOT NULL);"
        ).map_err(db)?;
        write_models(&tx, models, &payloads)?;
        tx.pragma_update(None, "application_id", APPLICATION_ID)
            .map_err(db)?;
        tx.pragma_update(None, "user_version", VERSION)
            .map_err(db)?;
        tx.commit().map_err(db)
    }

    /// Read one committed snapshot without creating a missing database.
    pub fn read(path: &Path) -> Result<CatalogSnapshot, CatalogError> {
        let mut connection = open(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let tx = connection.transaction().map_err(db)?;
        let snapshot = read_snapshot(&tx)?;
        tx.commit().map_err(db)?;
        Ok(snapshot)
    }

    /// Commit a complete validated inventory if its expected revision still matches.
    pub fn replace(
        path: &Path,
        expected: u64,
        models: &[ModelDescriptor],
    ) -> Result<u64, CatalogError> {
        let payloads = encode(models)?;
        let mut connection = open(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        let prior = read_snapshot(&tx)?;
        if prior.revision != expected {
            return Err(CatalogError::StaleRevision {
                expected,
                actual: prior.revision,
            });
        }
        let next = i64::try_from(expected + 1).map_err(|_| CatalogError::Format)?;
        tx.execute("DELETE FROM catalog_models", []).map_err(db)?;
        write_models(&tx, models, &payloads)?;
        tx.execute(
            "UPDATE catalog_state SET revision = ?1 WHERE singleton = 1",
            [next],
        )
        .map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(next as u64)
    }
}

fn open(path: &Path, flags: OpenFlags) -> Result<Connection, CatalogError> {
    let connection =
        Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NOFOLLOW).map_err(db)?;
    connection
        .busy_timeout(Duration::from_secs(2))
        .map_err(db)?;
    connection
        .pragma_update(None, "trusted_schema", "OFF")
        .map_err(db)?;
    Ok(connection)
}

fn identity(connection: &Connection) -> Result<(i64, i64), CatalogError> {
    let id = connection
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(db)?;
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db)?;
    Ok((id, version))
}

fn read_snapshot(connection: &Connection) -> Result<CatalogSnapshot, CatalogError> {
    if identity(connection)? != (APPLICATION_ID, VERSION) {
        return Err(CatalogError::Format);
    }
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM catalog_state WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(db)?;
    let mut statement = connection.prepare(
        "SELECT CASE WHEN length(payload) <= ?1 THEN payload END FROM catalog_models ORDER BY position"
    ).map_err(db)?;
    let mut rows = statement.query([MAX_DESCRIPTOR_BYTES as i64]).map_err(db)?;
    let mut models = Vec::new();
    while let Some(row) = rows.next().map_err(db)? {
        let payload: Vec<u8> = row.get(0).map_err(db)?;
        models.push(serde_json::from_slice(&payload).map_err(|_| CatalogError::Format)?);
    }
    validate(&models).map_err(|_| CatalogError::Format)?;
    Ok(CatalogSnapshot {
        revision: u64::try_from(revision).map_err(|_| CatalogError::Format)?,
        models,
    })
}

fn validate(models: &[ModelDescriptor]) -> Result<(), CatalogError> {
    let mut ids = BTreeSet::new();
    let mut bindings = BTreeSet::new();
    if models.iter().all(|model| {
        model.validate().is_ok()
            && ids.insert(model.id)
            && bindings.insert((model.provider_id, model.identifier.as_str()))
    }) {
        Ok(())
    } else {
        Err(CatalogError::InvalidModel)
    }
}

fn encode(models: &[ModelDescriptor]) -> Result<Vec<Vec<u8>>, CatalogError> {
    validate(models)?;
    models
        .iter()
        .map(|model| {
            let payload = serde_json::to_vec(model).map_err(|_| CatalogError::InvalidModel)?;
            (payload.len() <= MAX_DESCRIPTOR_BYTES)
                .then_some(payload)
                .ok_or(CatalogError::InvalidModel)
        })
        .collect()
}

fn write_models(
    connection: &Connection,
    models: &[ModelDescriptor],
    payloads: &[Vec<u8>],
) -> Result<(), CatalogError> {
    let mut bind = connection
        .prepare(
            "INSERT INTO catalog_bindings VALUES (?1, ?2, ?3)
         ON CONFLICT(model_id) DO UPDATE SET model_id = excluded.model_id
         WHERE provider_id = excluded.provider_id AND identifier = excluded.identifier",
        )
        .map_err(db)?;
    let mut insert = connection
        .prepare("INSERT INTO catalog_models (payload) VALUES (?1)")
        .map_err(db)?;
    for (model, payload) in models.iter().zip(payloads) {
        if bind
            .execute(rusqlite::params![
                model.id.to_string(),
                model.provider_id.to_string(),
                model.identifier
            ])
            .map_err(db)?
            == 0
        {
            return Err(CatalogError::BindingChanged);
        }
        insert.execute([payload]).map_err(db)?;
    }
    Ok(())
}

fn db(_: rusqlite::Error) -> CatalogError {
    CatalogError::Database
}
