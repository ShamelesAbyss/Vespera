//! Whole-database operations for a trusted filesystem operator.
use std::ffi::OsString;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use vespera_core::MemoryId;

use crate::{StorageError, schema, types::db};

const SIDECARS: [&str; 3] = ["-journal", "-wal", "-shm"];

/// Copy a consistent, verified SQLite snapshot to a new file.
///
/// The destination's parent must exist and support hard links. This operation
/// includes every agent namespace and is for trusted operators with direct
/// filesystem access. Application-level namespace grants do not apply.
pub fn copy_memory_database(source: &Path, destination: &Path) -> Result<(), StorageError> {
    let source = std::path::absolute(source).map_err(|_| StorageError::Invalid("source"))?;
    let destination =
        std::path::absolute(destination).map_err(|_| StorageError::Invalid("destination"))?;
    let parent = destination
        .parent()
        .ok_or(StorageError::Invalid("destination"))?;
    if !fs::metadata(&source).is_ok_and(|metadata| metadata.is_file()) {
        return Err(StorageError::WrongDatabase);
    }
    require_unused(&destination)?;

    let mut input = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db)?;
    schema::open_existing(&mut input)?;

    let stage = StagingDirectory::create(parent)?;
    let copy = stage.database();
    let path = copy.to_str().ok_or(StorageError::Invalid("destination"))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&copy).map_err(|_| StorageError::SnapshotIo)?;
    input.execute("VACUUM INTO ?1", [path]).map_err(db)?;
    drop(input);

    verify(&copy)?;
    OpenOptions::new()
        .write(true)
        .open(&copy)
        .and_then(|file| file.sync_all())
        .map_err(|_| StorageError::SnapshotIo)?;
    require_unused(&destination)?;
    // Linking publishes a complete file and fails atomically if another caller won.
    fs::hard_link(copy, destination).map_err(|error| match error.kind() {
        ErrorKind::AlreadyExists => StorageError::SnapshotTargetExists,
        _ => StorageError::SnapshotIo,
    })
}

fn require_unused(path: &Path) -> Result<(), StorageError> {
    for candidate in
        std::iter::once(path.to_path_buf()).chain(SIDECARS.map(|suffix| sidecar(path, suffix)))
    {
        match fs::symlink_metadata(candidate) {
            Ok(_) => return Err(StorageError::SnapshotTargetExists),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(_) => return Err(StorageError::SnapshotIo),
        }
    }
    Ok(())
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

fn verify(path: &Path) -> Result<(), StorageError> {
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(db)?;
    schema::open_existing(&mut connection)?;
    let integrity: String = connection
        .query_row("PRAGMA integrity_check(1)", [], |row| row.get(0))
        .map_err(db)?;
    if integrity != "ok"
        || connection
            .prepare("PRAGMA foreign_key_check")
            .map_err(db)?
            .exists([])
            .map_err(db)?
    {
        return Err(StorageError::SnapshotInvalid);
    }
    let invalid_lineage: bool = connection
        .query_row(
            "SELECT EXISTS(
            SELECT 1 FROM memory_sources AS links
            JOIN memories AS summary ON summary.id=links.summary_id
            JOIN memories AS source ON source.id=links.source_id
            WHERE summary.owner != source.owner OR summary.origin != 'derived'
        ) OR EXISTS(
            SELECT 1 FROM memories AS memory
            LEFT JOIN namespace_counters AS counter ON counter.owner=memory.owner
            WHERE counter.owner IS NULL OR memory.owner_sequence < 1
                OR counter.next_sequence < memory.owner_sequence
        )",
            [],
            |row| row.get(0),
        )
        .map_err(db)?;
    if invalid_lineage {
        return Err(StorageError::SnapshotInvalid);
    }
    let check = connection.transaction().map_err(db)?;
    check
        .execute(
            "INSERT INTO memory_fts(memory_fts, rank) VALUES('integrity-check', 1)",
            [],
        )
        .map_err(|_| StorageError::SnapshotInvalid)?;
    check.rollback().map_err(db)
}

struct StagingDirectory(PathBuf);

impl StagingDirectory {
    fn create(parent: &Path) -> Result<Self, StorageError> {
        let path = parent.join(format!(".vespera-memory-copy-{}", MemoryId::new()));
        let mut builder = DirBuilder::new();
        builder.recursive(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&path)
            .map_err(|_| StorageError::SnapshotIo)?;
        Ok(Self(path))
    }

    fn database(&self) -> PathBuf {
        self.0.join("memory.sqlite3")
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        let copy = self.database();
        for suffix in SIDECARS {
            let _ = fs::remove_file(sidecar(&copy, suffix));
        }
        let _ = fs::remove_file(copy);
        let _ = fs::remove_dir(&self.0);
    }
}
