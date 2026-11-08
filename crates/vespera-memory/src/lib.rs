//! Independent, authorized SQLite memory storage. No inference engine is required.
//! Calls are blocking: async applications should place database work on a blocking worker.
mod schema;
mod snapshot;
mod store;
mod types;
pub use snapshot::copy_memory_database;
pub use store::MemoryStore;
pub use types::{
    MemoryAuthorizer, MemoryOrigin, MemoryPage, MemoryQuery, NamespaceUsage, StorageError,
    StorageLimits, StoredMemory,
};

pub fn sqlite_version() -> &'static str {
    rusqlite::version()
}
