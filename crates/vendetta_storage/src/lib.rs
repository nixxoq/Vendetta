pub mod db;
pub mod error;
pub mod media;
pub mod messages;
pub mod migration;
pub mod peers;
pub mod replies;
pub mod search;
pub mod sync;

pub use db::ArchiveDb;
pub use error::{StorageError, StorageResult};
pub use migration::run_migrations;
pub use search::{FtsSearchParams, FtsSearchResult};

/// The latest canonical schema version for Vendetta SQLite archive databases.
pub const CURRENT_SCHEMA_VERSION: i64 = 5;
