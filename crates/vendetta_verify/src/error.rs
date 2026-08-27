use thiserror::Error;

#[derive(Debug, Error)]
pub enum VerificationError {
    #[error("Database error: {0}")]
    Database(#[from] vendetta_storage::StorageError),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Render verifier error: {0}")]
    Render(#[from] vendetta_render::RenderError),

    #[error("Fatal verification failure: {0}")]
    Fatal(String),
}

pub type VerificationResult<T> = Result<T, VerificationError>;
