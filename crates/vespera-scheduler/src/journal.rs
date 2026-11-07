use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, ErrorCode, OpenFlags, TransactionBehavior, params};
use thiserror::Error;
use vespera_core::ProviderId;

const APPLICATION_ID: i64 = 0x56535053;
const VERSION: i64 = 1;
const MAX_RECORDS: i64 = 4096;

#[derive(Debug, Error)]
pub enum ResidencyJournalError {
    #[error("file is not a supported Vespera residency journal")]
    WrongDatabase,
    #[error("residency journal is in use by another connection")]
    Busy,
    #[error("residency journal operation failed")]
    Database,
    #[error("residency journal record is invalid or its capacity was reached")]
    InvalidRecord,
}

fn db(error: rusqlite::Error) -> ResidencyJournalError {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => ResidencyJournalError::Busy,
        Some(ErrorCode::NotADatabase) => ResidencyJournalError::WrongDatabase,
        _ => ResidencyJournalError::Database,
    }
}

pub(crate) struct Intent {
    pub provider_id: ProviderId,
    pub model_key: String,
    pub ram_bytes: u64,
    pub vram_bytes: u64,
}

pub(crate) struct Journal(Connection);

impl Journal {
    pub fn open(path: &Path) -> Result<Self, ResidencyJournalError> {
        if path.as_os_str().is_empty() {
            return Err(ResidencyJournalError::WrongDatabase);
        }
        let path = std::path::absolute(path).map_err(|_| ResidencyJournalError::Database)?;
        let mut connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(db)?;
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
        if (id, version) == (0, 0) {
            let empty: bool = tx
                .query_row(
                    "SELECT NOT EXISTS(SELECT 1 FROM sqlite_schema)",
                    [],
                    |row| row.get(0),
                )
                .map_err(db)?;
            if !empty {
                return Err(ResidencyJournalError::WrongDatabase);
            }
            tx.execute_batch(
                "CREATE TABLE residency_intent (
                    provider_id TEXT NOT NULL,
                    model_key TEXT NOT NULL CHECK(length(CAST(model_key AS BLOB)) BETWEEN 1 AND 4096),
                    ram_bytes INTEGER NOT NULL CHECK(ram_bytes >= 0),
                    vram_bytes INTEGER NOT NULL CHECK(vram_bytes >= 0),
                    PRIMARY KEY(provider_id, model_key)
                ) STRICT;",
            ).map_err(db)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(db)?;
            tx.pragma_update(None, "user_version", VERSION)
                .map_err(db)?;
        } else if (id, version) != (APPLICATION_ID, VERSION) {
            return Err(ResidencyJournalError::WrongDatabase);
        }
        tx.commit().map_err(db)?;
        Ok(Self(connection))
    }

    pub fn records(&self) -> Result<Vec<Intent>, ResidencyJournalError> {
        let mut query = self.0.prepare(
            "SELECT provider_id, model_key, ram_bytes, vram_bytes FROM residency_intent LIMIT ?1"
        ).map_err(db)?;
        let mut rows = query.query([MAX_RECORDS + 1]).map_err(db)?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().map_err(db)? {
            let provider: String = row.get(0).map_err(db)?;
            let provider_id = provider
                .parse()
                .map_err(|_| ResidencyJournalError::InvalidRecord)?;
            let model_key: String = row.get(1).map_err(db)?;
            if model_key.is_empty() || model_key.len() > 4096 || records.len() as i64 == MAX_RECORDS
            {
                return Err(ResidencyJournalError::InvalidRecord);
            }
            records.push(Intent {
                provider_id,
                model_key,
                ram_bytes: u64::try_from(row.get::<_, i64>(2).map_err(db)?)
                    .map_err(|_| ResidencyJournalError::InvalidRecord)?,
                vram_bytes: u64::try_from(row.get::<_, i64>(3).map_err(db)?)
                    .map_err(|_| ResidencyJournalError::InvalidRecord)?,
            });
        }
        Ok(records)
    }

    pub fn record(&self, intent: &Intent) -> Result<(), ResidencyJournalError> {
        let ram =
            i64::try_from(intent.ram_bytes).map_err(|_| ResidencyJournalError::InvalidRecord)?;
        let vram =
            i64::try_from(intent.vram_bytes).map_err(|_| ResidencyJournalError::InvalidRecord)?;
        let inserted = self
            .0
            .execute(
                "INSERT INTO residency_intent SELECT ?1, ?2, ?3, ?4
             WHERE (SELECT count(*) FROM residency_intent) < ?5",
                params![
                    intent.provider_id.to_string(),
                    intent.model_key,
                    ram,
                    vram,
                    MAX_RECORDS
                ],
            )
            .map_err(db)?;
        if inserted != 1 {
            return Err(ResidencyJournalError::InvalidRecord);
        }
        Ok(())
    }

    pub fn remove(
        &self,
        provider_id: ProviderId,
        model_key: &str,
    ) -> Result<(), ResidencyJournalError> {
        self.0
            .execute(
                "DELETE FROM residency_intent WHERE provider_id=?1 AND model_key=?2",
                params![provider_id.to_string(), model_key],
            )
            .map_err(db)?;
        Ok(())
    }

    pub fn clear_acknowledged(
        &mut self,
        records: Vec<Intent>,
        can_clear: impl Fn(&Intent) -> bool,
    ) -> Result<Vec<Intent>, ResidencyJournalError> {
        let tx = self.0.transaction().map_err(db)?;
        let mut retained = Vec::new();
        for intent in records {
            if can_clear(&intent) {
                tx.execute(
                    "DELETE FROM residency_intent WHERE provider_id=?1 AND model_key=?2",
                    params![intent.provider_id.to_string(), intent.model_key],
                )
                .map_err(db)?;
            } else {
                retained.push(intent);
            }
        }
        tx.commit().map_err(db)?;
        Ok(retained)
    }
}
