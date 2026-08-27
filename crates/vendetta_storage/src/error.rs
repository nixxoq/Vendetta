use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("Migration error: {0}")]
    Migration(String),

    #[error("Transaction rolled back: {0}")]
    Transaction(String),

    #[error("Entity not found: {0}")]
    NotFound(String),

    #[error("Data serialization error: {0}")]
    Serialization(String),
}

pub type StorageResult<T> = std::result::Result<T, StorageError>;

impl From<StorageError> for vendetta_core::VendettaError {
    fn from(err: StorageError) -> Self {
        vendetta_core::VendettaError::Storage(err.to_string())
    }
}
