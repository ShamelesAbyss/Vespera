use serde::{Deserialize, Serialize};
use thiserror::Error;
use vespera_core::{AgentId, MemoryId, MemoryLayer, MemoryRecord, Permission, TaskId};

/// Trusted control-plane boundary, evaluated on every operation. IDs are not authentication.
/// Implementations must consult current grants and accepted task assignments, not model text.
pub trait MemoryAuthorizer: Send + Sync + 'static {
    fn allowed(&self, actor: AgentId, permission: Permission) -> bool;
    fn owns_task(&self, actor: AgentId, task_id: TaskId) -> bool;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StorageError {
    #[error("memory access denied")]
    Denied,
    #[error("invalid memory field: {0}")]
    Invalid(&'static str),
    #[error("memory storage capacity reached")]
    Capacity,
    #[error("memory write conflicts with existing state")]
    Conflict,
    #[error("memory source is unavailable in the authorized namespace")]
    SourceUnavailable,
    #[error("memory is referenced by a derived record; remove that record first")]
    Referenced,
    #[error("file is not a Vespera memory database")]
    WrongDatabase,
    #[error("memory schema version is unsupported")]
    UnsupportedSchema,
    #[error("memory database is busy")]
    Busy,
    #[error("memory store was opened read-only")]
    ReadOnly,
    #[error("memory copy destination or one of its SQLite sidecars already exists")]
    SnapshotTargetExists,
    #[error("memory copy failed its SQLite, index or lineage integrity check")]
    SnapshotInvalid,
    #[error("memory copy could not be staged or published on this filesystem")]
    SnapshotIo,
    #[error("memory database operation failed; raw SQL and private values suppressed")]
    Database,
}
pub(crate) fn db(error: rusqlite::Error) -> StorageError {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            StorageError::Busy
        }
        Some(rusqlite::ErrorCode::ConstraintViolation) => StorageError::Conflict,
        _ => StorageError::Database,
    }
}

/// Logical authoritative content quotas. Index/file/allocator overhead is separate.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageLimits {
    pub max_namespaces: u64,
    pub max_record_bytes: usize,
    pub max_namespace_records: u64,
    pub max_namespace_bytes: u64,
    pub max_total_records: u64,
    pub max_total_bytes: u64,
}
impl Default for StorageLimits {
    fn default() -> Self {
        Self {
            max_namespaces: 64,
            max_record_bytes: 64 * 1024,
            max_namespace_records: 1024,
            max_namespace_bytes: 8 * 1024 * 1024,
            max_total_records: 8192,
            max_total_bytes: 64 * 1024 * 1024,
        }
    }
}
impl StorageLimits {
    pub fn validate(&self) -> Result<(), StorageError> {
        if !(1..=1024).contains(&self.max_namespaces)
            || !(1..=64 * 1024).contains(&self.max_record_bytes)
            || !(1..=1_000_000).contains(&self.max_namespace_records)
            || self.max_total_records < self.max_namespace_records
            || self.max_total_records > 1_000_000
            || self.max_namespace_bytes < self.max_record_bytes as u64
            || self.max_total_bytes < self.max_namespace_bytes
            || self.max_total_bytes > 1024 * 1024 * 1024
        {
            return Err(StorageError::Invalid("limits"));
        }
        Ok(())
    }
}

/// Origin is a caller-supplied classification, not independently established truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOrigin {
    Observation,
    Import,
    Generated,
    Derived,
}
impl MemoryOrigin {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::Import => "import",
            Self::Generated => "generated",
            Self::Derived => "derived",
        }
    }
    pub(crate) fn parse(value: &str) -> Result<Self, StorageError> {
        match value {
            "observation" => Ok(Self::Observation),
            "import" => Ok(Self::Import),
            "generated" => Ok(Self::Generated),
            "derived" => Ok(Self::Derived),
            _ => Err(StorageError::Database),
        }
    }
}
pub(crate) fn layer_name(layer: MemoryLayer) -> &'static str {
    match layer {
        MemoryLayer::LongTerm => "long_term",
        MemoryLayer::Episodic => "episodic",
        MemoryLayer::Working => "working",
        MemoryLayer::Relationship => "relationship",
        MemoryLayer::Goal => "goal",
        MemoryLayer::Observation => "observation",
    }
}
pub(crate) fn parse_layer(value: &str) -> Result<MemoryLayer, StorageError> {
    match value {
        "long_term" => Ok(MemoryLayer::LongTerm),
        "episodic" => Ok(MemoryLayer::Episodic),
        "working" => Ok(MemoryLayer::Working),
        "relationship" => Ok(MemoryLayer::Relationship),
        "goal" => Ok(MemoryLayer::Goal),
        "observation" => Ok(MemoryLayer::Observation),
        _ => Err(StorageError::Database),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoredMemory {
    pub record: MemoryRecord,
    pub sequence: u64,
    pub origin: MemoryOrigin,
    pub derivation_method: Option<String>,
    pub source_memories: Vec<MemoryId>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NamespaceUsage {
    pub records: u64,
    pub content_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct MemoryQuery {
    pub layer: Option<MemoryLayer>,
    pub after_sequence: u64,
    pub created_since_unix_ms: Option<u64>,
    pub created_before_unix_ms: Option<u64>,
    pub limit: u32,
    /// Content bytes only; metadata is separately bounded by record/result counts.
    pub max_content_bytes: usize,
}
impl Default for MemoryQuery {
    fn default() -> Self {
        Self {
            layer: None,
            after_sequence: 0,
            created_since_unix_ms: None,
            created_before_unix_ms: None,
            limit: 100,
            max_content_bytes: 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct MemoryPage {
    pub records: Vec<StoredMemory>,
    pub next_after_sequence: Option<u64>,
    pub content_bytes: usize,
}
