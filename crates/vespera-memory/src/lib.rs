//! Independent, authorized SQLite memory storage. No inference engine is required.
//! Calls are blocking: async applications should place database work on a blocking worker.
mod schema;
mod store;
mod types;
pub use store::MemoryStore;
pub use types::{
    MemoryAuthorizer, MemoryOrigin, MemoryPage, MemoryQuery, NamespaceUsage, StorageError,
    StorageLimits, StoredMemory,
};

pub fn sqlite_version() -> &'static str {
    rusqlite::version()
}
