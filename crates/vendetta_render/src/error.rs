use std::path::PathBuf;
use thiserror::Error;
use vendetta_storage::StorageError;

pub type RenderResult<T> = Result<T, RenderError>;

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Target export directory already exists: {0}. Use --replace to overwrite.")]
    TargetAlreadyExists(PathBuf),

    #[error("HTML archive verification failed: {0}")]
    VerificationFailed(String),

    #[error("Invalid export configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Missing referenced media file: {0}")]
    MissingMedia(String),

    #[error("Unsafe path traversal detected: {0}")]
    UnsafePath(String),

    #[error("Export operation aborted: {0}")]
    ExportAborted(String),
}
