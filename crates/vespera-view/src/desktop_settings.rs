use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use vespera_config::ProviderConfig;
use vespera_core::ProviderKind;

use crate::desktop_agent;

pub(super) const ENDPOINT_LIMIT: usize = 160;
const FILE: &str = "desktop-settings.sqlite3";
const APPLICATION_ID: i32 = 0x5644_5345;
const STORAGE: &str = "Could not access saved discovery settings.";
const FORMAT: &str = "Saved discovery settings have an unrecognized format.";
const INVALID: &str =
    "Discovery settings require a valid loopback endpoint and credential reference.";

pub(super) fn load(dir: &Path) -> Result<Option<ProviderConfig>, &'static str> {
    let path = dir.join(FILE);
    match path.symlink_metadata() {
        Ok(info) if info.is_file() && info.len() == 0 => return Ok(None),
        Ok(info) if info.is_file() => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        _ => return Err(FORMAT),
    }
    let db = open(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    if identity(&db)? != (APPLICATION_ID, 1) {
        return Err(FORMAT);
    }
    read(&db).map(Some)
}

pub(super) fn save(dir: &Path, settings: &ProviderConfig) -> Result<(), &'static str> {
    let kind = validate(settings).ok_or(INVALID)?;
    fs::create_dir_all(dir).map_err(|_| STORAGE)?;
    let path = dir.join(FILE);
    let can_initialize = match path.symlink_metadata() {
        Ok(info) if info.is_file() => info.len() == 0,
        Ok(_) => return Err(FORMAT),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => return Err(STORAGE),
    };
    let mut db = open(
        &path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| STORAGE)?;
    match identity(&tx)? {
        (APPLICATION_ID, 1) => {
            read(&tx)?;
        }
        (0, 0) if can_initialize => {
            let objects: i64 = tx
                .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
                .map_err(|_| FORMAT)?;
            if objects != 0 {
                return Err(FORMAT);
            }
            tx.execute_batch(
                "CREATE TABLE discovery (
                    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                    kind INTEGER NOT NULL CHECK(kind IN (0,1)),
                    endpoint TEXT NOT NULL CHECK(length(CAST(endpoint AS BLOB)) <= 160),
                    credential_env TEXT CHECK(length(CAST(credential_env AS BLOB)) <= 128),
                    credential_windows INTEGER NOT NULL CHECK(credential_windows IN (0,1)),
                    CHECK(credential_windows = 0 OR credential_env IS NULL)
                ) STRICT;",
            )
            .map_err(|_| STORAGE)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(|_| STORAGE)?;
            tx.pragma_update(None, "user_version", 1)
                .map_err(|_| STORAGE)?;
        }
        _ => return Err(FORMAT),
    }
    tx.execute(
        "INSERT INTO discovery VALUES (1, ?1, ?2, ?3, ?4)
         ON CONFLICT(singleton) DO UPDATE SET kind = excluded.kind, endpoint = excluded.endpoint,
         credential_env = excluded.credential_env, credential_windows = excluded.credential_windows",
        params![kind, settings.endpoint, settings.credential_env, settings.credential_windows],
    ).map_err(|_| STORAGE)?;
    tx.commit().map_err(|_| STORAGE)
}

fn open(path: &Path, flags: OpenFlags) -> Result<Connection, &'static str> {
    let db = Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NOFOLLOW)
        .map_err(|_| STORAGE)?;
    db.busy_timeout(Duration::ZERO).map_err(|_| STORAGE)?;
    Ok(db)
}

fn identity(db: &Connection) -> Result<(i32, u32), &'static str> {
    Ok((
        db.pragma_query_value(None, "application_id", |row| row.get(0))
            .map_err(|_| FORMAT)?,
        db.pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|_| FORMAT)?,
    ))
}

fn read(db: &Connection) -> Result<ProviderConfig, &'static str> {
    let (kind, endpoint, credential_env, credential_windows): (i64, String, Option<String>, bool) =
        db.query_row(
            "SELECT kind, endpoint, credential_env, credential_windows FROM discovery
             WHERE singleton = 1 AND length(CAST(endpoint AS BLOB)) <= ?1
                 AND (credential_env IS NULL OR length(CAST(credential_env AS BLOB)) <= 128)
                 AND credential_windows IN (0,1)",
            [ENDPOINT_LIMIT as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| FORMAT)?;
    let kind = match kind {
        0 => ProviderKind::Ollama,
        1 => ProviderKind::OpenAiCompatible,
        _ => return Err(FORMAT),
    };
    let mut settings = desktop_agent::provider(kind, endpoint);
    settings.credential_env = credential_env;
    settings.credential_windows = credential_windows;
    validate(&settings).ok_or(FORMAT)?;
    Ok(settings)
}

fn validate(settings: &ProviderConfig) -> Option<i64> {
    if settings.endpoint.len() > ENDPOINT_LIMIT
        || settings.endpoint.chars().any(char::is_control)
        || settings
            .credential_env
            .as_deref()
            .is_some_and(|name| !desktop_agent::credential_name(name))
        || desktop_agent::config(settings.clone()).validate().is_err()
    {
        return None;
    }
    match settings.kind {
        ProviderKind::Ollama => Some(0),
        ProviderKind::OpenAiCompatible => Some(1),
        ProviderKind::Custom => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use vespera_core::TaskId;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("vespera-settings-{}", TaskId::new())))
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(self.0.join(FILE));
            let _ = fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn discovery_settings_round_trip_and_reject_an_invalid_update() {
        let dir = Fixture::new();
        assert!(load(&dir.0).unwrap().is_none());
        let mut settings = desktop_agent::provider(
            ProviderKind::OpenAiCompatible,
            "http://127.0.0.1:9000/v1".into(),
        );
        settings.credential_env = Some("VESPERA_TEST_REFERENCE".into());
        save(&dir.0, &settings).unwrap();
        let restored = load(&dir.0).unwrap().unwrap();
        assert_eq!(restored.kind, settings.kind);
        assert_eq!(restored.endpoint, settings.endpoint);
        assert_eq!(restored.credential_env, settings.credential_env);
        assert!(!dir.0.join("desktop.sqlite3").exists());

        settings.endpoint = "http://user:secret@127.0.0.1:9000/v1".into();
        assert_eq!(save(&dir.0, &settings), Err(INVALID));
        let retained = load(&dir.0).unwrap().unwrap();
        assert_eq!(retained.endpoint, restored.endpoint);
        assert_eq!(retained.credential_env, restored.credential_env);

        settings.endpoint = restored.endpoint;
        settings.credential_env = None;
        settings.credential_windows = true;
        save(&dir.0, &settings).unwrap();
        let updated = load(&dir.0).unwrap().unwrap();
        assert!(updated.credential_windows);
        assert!(updated.credential_env.is_none());
    }

    #[test]
    fn discovery_settings_preserve_an_unrecognized_file() {
        let dir = Fixture::new();
        fs::create_dir(&dir.0).unwrap();
        let path = dir.0.join(FILE);
        fs::write(&path, b"foreign fixture").unwrap();
        let settings =
            desktop_agent::provider(ProviderKind::Ollama, "http://127.0.0.1:11434".into());
        assert!(load(&dir.0).is_err());
        assert!(save(&dir.0, &settings).is_err());
        assert_eq!(fs::read(path).unwrap(), b"foreign fixture");
    }

    #[test]
    fn discovery_settings_retry_after_interrupted_file_creation() {
        let dir = Fixture::new();
        fs::create_dir(&dir.0).unwrap();
        fs::File::create(dir.0.join(FILE)).unwrap();
        assert!(load(&dir.0).unwrap().is_none());
        let settings =
            desktop_agent::provider(ProviderKind::Ollama, "http://127.0.0.1:11434".into());
        save(&dir.0, &settings).unwrap();
        assert_eq!(load(&dir.0).unwrap().unwrap().endpoint, settings.endpoint);
    }
}
