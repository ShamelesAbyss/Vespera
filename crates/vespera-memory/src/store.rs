use crate::types::{db, layer_name, parse_layer};
use crate::{
    MemoryAuthorizer, MemoryOrigin, MemoryPage, MemoryQuery, NamespaceUsage, StorageError,
    StorageLimits, StoredMemory,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use vespera_core::{AgentId, MemoryId, MemoryRecord, Permission};

pub struct MemoryStore {
    connection: Mutex<Connection>,
    limits: StorageLimits,
    authorizer: Arc<dyn MemoryAuthorizer>,
    read_only: bool,
}
impl MemoryStore {
    /// Explicitly open/create a database file. Parent directories must already exist.
    /// Existing unrelated or future-version databases are rejected without migration.
    pub fn open(
        path: impl AsRef<Path>,
        limits: StorageLimits,
        authorizer: Arc<dyn MemoryAuthorizer>,
    ) -> Result<Self, StorageError> {
        Self::open_mode(path.as_ref(), limits, authorizer, true, false)
    }
    /// Open existing storage without creating or initializing a missing/empty file.
    pub fn open_existing(
        path: impl AsRef<Path>,
        limits: StorageLimits,
        authorizer: Arc<dyn MemoryAuthorizer>,
    ) -> Result<Self, StorageError> {
        Self::open_mode(path.as_ref(), limits, authorizer, false, false)
    }
    /// Open existing storage with SQLite's read-only flag. No schema creation/migration.
    pub fn open_read_only(
        path: impl AsRef<Path>,
        limits: StorageLimits,
        authorizer: Arc<dyn MemoryAuthorizer>,
    ) -> Result<Self, StorageError> {
        Self::open_mode(path.as_ref(), limits, authorizer, false, true)
    }
    fn open_mode(
        path: &Path,
        limits: StorageLimits,
        authorizer: Arc<dyn MemoryAuthorizer>,
        initialize: bool,
        read_only: bool,
    ) -> Result<Self, StorageError> {
        limits.validate()?;
        if path.as_os_str().is_empty()
            || path.to_string_lossy() == ":memory:"
            || path.to_string_lossy().starts_with("file:")
        {
            return Err(StorageError::Invalid("path"));
        }
        let mut flags = rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        flags |= if read_only {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        } else {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
        };
        if initialize {
            flags |= rusqlite::OpenFlags::SQLITE_OPEN_CREATE;
        }
        let mut connection = Connection::open_with_flags(path, flags).map_err(db)?;
        if initialize {
            crate::schema::initialize(&mut connection)?;
        } else {
            crate::schema::open_existing(&mut connection)?;
        }
        Ok(Self {
            connection: Mutex::new(connection),
            limits,
            authorizer,
            read_only,
        })
    }
    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn authorize(
        &self,
        actor: AgentId,
        owner: AgentId,
        permission: Permission,
    ) -> Result<(), StorageError> {
        if actor != owner
            || actor.validate().is_err()
            || !self.authorizer.allowed(actor, permission)
        {
            return Err(StorageError::Denied);
        }
        Ok(())
    }
    fn validate_record(&self, actor: AgentId, record: &MemoryRecord) -> Result<(), StorageError> {
        self.authorize(actor, record.agent_id, Permission::MemoryWrite)?;
        if self.read_only {
            return Err(StorageError::ReadOnly);
        }
        record
            .validate()
            .map_err(|_| StorageError::Invalid("record"))?;
        if record.content.len() > self.limits.max_record_bytes {
            return Err(StorageError::Capacity);
        }
        integer(record.created_at_unix_ms)?;
        if record
            .source_task_id
            .is_some_and(|task| !self.authorizer.owns_task(actor, task))
        {
            return Err(StorageError::SourceUnavailable);
        }
        Ok(())
    }
    pub fn insert(
        &self,
        actor: AgentId,
        record: MemoryRecord,
        origin: MemoryOrigin,
    ) -> Result<StoredMemory, StorageError> {
        self.validate_record(actor, &record)?;
        if origin == MemoryOrigin::Derived {
            return Err(StorageError::Invalid("derived_requires_sources"));
        }
        if origin == MemoryOrigin::Generated && record.source_task_id.is_none() {
            return Err(StorageError::Invalid("generated_requires_task"));
        }
        self.write(record, origin, None, &[])
    }
    /// Create a derived record and lineage atomically, preserving all originals.
    /// Caller-supplied method/content describe provenance, not factual verification.
    pub fn consolidate(
        &self,
        actor: AgentId,
        summary: MemoryRecord,
        sources: &[MemoryId],
        method: &str,
    ) -> Result<StoredMemory, StorageError> {
        self.validate_record(actor, &summary)?;
        self.authorize(actor, summary.agent_id, Permission::MemoryRead)?;
        if sources.is_empty()
            || sources.len() > 32
            || sources.iter().collect::<BTreeSet<_>>().len() != sources.len()
        {
            return Err(StorageError::Invalid("sources"));
        }
        if method.trim().is_empty() || method.len() > 128 {
            return Err(StorageError::Invalid("method"));
        }
        self.write(summary, MemoryOrigin::Derived, Some(method), sources)
    }
    fn write(
        &self,
        record: MemoryRecord,
        origin: MemoryOrigin,
        method: Option<&str>,
        sources: &[MemoryId],
    ) -> Result<StoredMemory, StorageError> {
        let mut connection = self.connection();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        self.validate_record(record.agent_id, &record)?;
        if origin == MemoryOrigin::Derived {
            self.authorize(record.agent_id, record.agent_id, Permission::MemoryRead)?;
        }
        // Database-derived counts inside the write transaction also coordinate other connections.
        let total: NamespaceUsage = usage(&tx, None)?;
        let own: NamespaceUsage = usage(&tx, Some(record.agent_id))?;
        let bytes = record.content.len() as u64;
        if total.records >= self.limits.max_total_records
            || own.records >= self.limits.max_namespace_records
            || total
                .content_bytes
                .checked_add(bytes)
                .is_none_or(|v| v > self.limits.max_total_bytes)
            || own
                .content_bytes
                .checked_add(bytes)
                .is_none_or(|v| v > self.limits.max_namespace_bytes)
        {
            return Err(StorageError::Capacity);
        }
        for source in sources {
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM memories WHERE id=?1 AND owner=?2)",
                    params![source.to_string(), record.agent_id.to_string()],
                    |row| row.get(0),
                )
                .map_err(db)?;
            if !exists {
                return Err(StorageError::SourceUnavailable);
            }
        }
        let namespace_exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM namespace_counters WHERE owner=?1)",
                [record.agent_id.to_string()],
                |row| row.get(0),
            )
            .map_err(db)?;
        if !namespace_exists {
            let count: i64 = tx
                .query_row("SELECT count(*) FROM namespace_counters", [], |row| {
                    row.get(0)
                })
                .map_err(db)?;
            if unsigned(count)? >= self.limits.max_namespaces {
                return Err(StorageError::Capacity);
            }
        }
        let owner_sequence:i64=tx.query_row("INSERT INTO namespace_counters(owner,next_sequence) VALUES(?1,1) ON CONFLICT(owner) DO UPDATE SET next_sequence=next_sequence+1 RETURNING next_sequence",[record.agent_id.to_string()],|row|row.get(0)).map_err(db)?;
        tx.execute("INSERT INTO memories(id,owner,layer,content,content_bytes,created_at,source_task_id,origin,method,owner_sequence) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", params![record.id.to_string(),record.agent_id.to_string(),layer_name(record.layer),record.content,integer(bytes)?,integer(record.created_at_unix_ms)?,record.source_task_id.map(|id| id.to_string()),origin.name(),method,owner_sequence]).map_err(db)?;
        let sequence = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO memory_fts(rowid,content) VALUES(?1,?2)",
            params![sequence, record.content],
        )
        .map_err(db)?;
        for source in sources {
            tx.execute(
                "INSERT INTO memory_sources(summary_id,source_id) VALUES(?1,?2)",
                params![record.id.to_string(), source.to_string()],
            )
            .map_err(db)?;
        }
        tx.commit().map_err(db)?;
        Ok(StoredMemory {
            record,
            sequence: unsigned(owner_sequence)?,
            origin,
            derivation_method: method.map(str::to_owned),
            source_memories: {
                let mut ids = sources.to_vec();
                ids.sort();
                ids
            },
        })
    }
    pub fn get(
        &self,
        actor: AgentId,
        owner: AgentId,
        id: MemoryId,
    ) -> Result<Option<StoredMemory>, StorageError> {
        self.authorize(actor, owner, Permission::MemoryRead)?;
        let mut connection = self.connection();
        let tx = connection.transaction().map_err(db)?;
        self.authorize(actor, owner, Permission::MemoryRead)?;
        let raw = tx.query_row("SELECT owner_sequence,id,owner,layer,content,created_at,source_task_id,origin,method FROM memories WHERE owner=?1 AND id=?2", params![owner.to_string(),id.to_string()], raw_memory).optional().map_err(db)?;
        let record = raw.map(|raw| self.materialize(&tx, raw)).transpose()?;
        tx.commit().map_err(db)?;
        Ok(record)
    }
    pub fn namespace_usage(
        &self,
        actor: AgentId,
        owner: AgentId,
    ) -> Result<NamespaceUsage, StorageError> {
        self.authorize(actor, owner, Permission::MemoryRead)?;
        let connection = self.connection();
        self.authorize(actor, owner, Permission::MemoryRead)?;
        usage(&connection, Some(owner))
    }
    pub fn list(
        &self,
        actor: AgentId,
        owner: AgentId,
        query: &MemoryQuery,
    ) -> Result<MemoryPage, StorageError> {
        self.query(actor, owner, query, None)
    }
    /// Literal phrase search. FTS operators and SQL cannot be injected through query text.
    /// Results use insertion order, not cross-namespace corpus scores.
    pub fn search(
        &self,
        actor: AgentId,
        owner: AgentId,
        phrase: &str,
        query: &MemoryQuery,
    ) -> Result<MemoryPage, StorageError> {
        self.authorize(actor, owner, Permission::MemoryRead)?;
        if phrase.trim().is_empty()
            || phrase.len() > 256
            || phrase.contains('\0')
            || phrase.split_whitespace().count() > 32
        {
            return Err(StorageError::Invalid("phrase"));
        }
        self.query(
            actor,
            owner,
            query,
            Some(format!("\"{}\"", phrase.replace('"', "\"\""))),
        )
    }
    fn query(
        &self,
        actor: AgentId,
        owner: AgentId,
        query: &MemoryQuery,
        phrase: Option<String>,
    ) -> Result<MemoryPage, StorageError> {
        self.authorize(actor, owner, Permission::MemoryRead)?;
        if !(1..=100).contains(&query.limit)
            || query.max_content_bytes < self.limits.max_record_bytes
            || query.max_content_bytes > 4 * 1024 * 1024
            || query
                .created_since_unix_ms
                .zip(query.created_before_unix_ms)
                .is_some_and(|(a, b)| a > b)
        {
            return Err(StorageError::Invalid("query"));
        }
        let after = integer(query.after_sequence)?;
        let since = query.created_since_unix_ms.map(integer).transpose()?;
        let before = query.created_before_unix_ms.map(integer).transpose()?;
        let filter = if phrase.is_some() {
            "AND sequence IN (SELECT rowid FROM memory_fts WHERE memory_fts MATCH ?6)"
        } else {
            "AND ?6 IS NULL"
        };
        let sql = format!(
            "SELECT owner_sequence,id,owner,layer,content,created_at,source_task_id,origin,method FROM memories WHERE owner=?1 AND (?2 IS NULL OR layer=?2) AND owner_sequence>?3 AND (?4 IS NULL OR created_at>=?4) AND (?5 IS NULL OR created_at<?5) {filter} ORDER BY owner_sequence LIMIT ?7"
        );
        let connection = self.connection();
        self.authorize(actor, owner, Permission::MemoryRead)?;
        let mut statement = connection.prepare(&sql).map_err(db)?;
        let mut rows = statement
            .query_map(
                params![
                    owner.to_string(),
                    query.layer.map(layer_name),
                    after,
                    since,
                    before,
                    phrase,
                    i64::from(query.limit) + 1
                ],
                raw_memory,
            )
            .map_err(db)?;
        let mut page = MemoryPage {
            records: Vec::new(),
            next_after_sequence: None,
            content_bytes: 0,
        };
        for raw in &mut rows {
            let record = self.materialize(&connection, raw.map_err(db)?)?;
            if page.records.len() >= query.limit as usize
                || page.content_bytes + record.record.content.len() > query.max_content_bytes
            {
                page.next_after_sequence = page.records.last().map(|r| r.sequence);
                break;
            }
            page.content_bytes += record.record.content.len();
            page.records.push(record);
        }
        Ok(page)
    }
    /// Explicit deletion requires both read and write. Referenced originals are protected.
    /// FTS removal and authoritative deletion commit or roll back together.
    pub fn delete(&self, actor: AgentId, owner: AgentId, id: MemoryId) -> Result<(), StorageError> {
        self.authorize(actor, owner, Permission::MemoryWrite)?;
        self.authorize(actor, owner, Permission::MemoryRead)?;
        if self.read_only {
            return Err(StorageError::ReadOnly);
        }
        let mut connection = self.connection();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db)?;
        self.authorize(actor, owner, Permission::MemoryWrite)?;
        self.authorize(actor, owner, Permission::MemoryRead)?;
        let raw: Option<(i64, String)> = tx
            .query_row(
                "SELECT sequence,content FROM memories WHERE id=?1 AND owner=?2",
                params![id.to_string(), owner.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(db)?;
        if let Some((sequence, content)) = raw {
            let referenced: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM memory_sources WHERE source_id=?1)",
                    [id.to_string()],
                    |row| row.get(0),
                )
                .map_err(db)?;
            if referenced {
                return Err(StorageError::Referenced);
            }
            tx.execute(
                "INSERT INTO memory_fts(memory_fts,rowid,content) VALUES('delete',?1,?2)",
                params![sequence, content],
            )
            .map_err(db)?;
            tx.execute(
                "DELETE FROM memories WHERE id=?1 AND owner=?2",
                params![id.to_string(), owner.to_string()],
            )
            .map_err(db)?;
        }
        tx.commit().map_err(db)
    }
    fn materialize(
        &self,
        connection: &Connection,
        raw: RawMemory,
    ) -> Result<StoredMemory, StorageError> {
        if raw.content.len() > self.limits.max_record_bytes {
            return Err(StorageError::Capacity);
        }
        let record = MemoryRecord {
            id: raw.id.parse().map_err(|_| StorageError::Database)?,
            agent_id: raw.owner.parse().map_err(|_| StorageError::Database)?,
            layer: parse_layer(&raw.layer)?,
            content: raw.content,
            created_at_unix_ms: unsigned(raw.created_at)?,
            source_task_id: raw
                .source_task
                .map(|id| id.parse().map_err(|_| StorageError::Database))
                .transpose()?,
        };
        record.validate().map_err(|_| StorageError::Database)?;
        let mut statement=connection.prepare("SELECT source_id FROM memory_sources WHERE summary_id=?1 ORDER BY source_id LIMIT 33").map_err(db)?;
        let sources = statement
            .query_map([record.id.to_string()], |row| row.get::<_, String>(0))
            .map_err(db)?
            .map(|value| {
                value
                    .map_err(db)?
                    .parse()
                    .map_err(|_| StorageError::Database)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if sources.len() > 32 {
            return Err(StorageError::Database);
        }
        Ok(StoredMemory {
            record,
            sequence: unsigned(raw.sequence)?,
            origin: MemoryOrigin::parse(&raw.origin)?,
            derivation_method: raw.method,
            source_memories: sources,
        })
    }
}

fn integer(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::Invalid("integer_range"))
}
fn unsigned(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::Database)
}
fn usage(connection: &Connection, owner: Option<AgentId>) -> Result<NamespaceUsage, StorageError> {
    let (records,bytes):(i64,i64)=connection.query_row("SELECT count(*),coalesce(sum(content_bytes),0) FROM memories WHERE (?1 IS NULL OR owner=?1)",[owner.map(|id|id.to_string())],|row|Ok((row.get(0)?,row.get(1)?))).map_err(db)?;
    Ok(NamespaceUsage {
        records: unsigned(records)?,
        content_bytes: unsigned(bytes)?,
    })
}
struct RawMemory {
    sequence: i64,
    id: String,
    owner: String,
    layer: String,
    content: String,
    created_at: i64,
    source_task: Option<String>,
    origin: String,
    method: Option<String>,
}
fn raw_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawMemory> {
    Ok(RawMemory {
        sequence: row.get(0)?,
        id: row.get(1)?,
        owner: row.get(2)?,
        layer: row.get(3)?,
        content: row.get(4)?,
        created_at: row.get(5)?,
        source_task: row.get(6)?,
        origin: row.get(7)?,
        method: row.get(8)?,
    })
}
