use thiserror::Error;

#[derive(Debug, Error)]
pub enum VendettaError {
    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Migration error: {0}")]
    Migration(String),

    #[error("Adapter error: {0}")]
    Adapter(String),

    #[error("Session error: {0}")]
    Session(String),

    #[error("Sync error: {0}")]
    Sync(String),

    #[error("Model validation error: {0}")]
    ModelValidation(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Other error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, VendettaError>;
