use std::path::PathBuf;
use thiserror::Error;

pub type ImportResult<T> = Result<T, ImportError>;

#[derive(Debug, Error)]
pub enum ImportError {
    #[error(
        "Destination archive database already exists at {0}. Import requires a fresh destination database."
    )]
    DestinationArchiveExists(PathBuf),

    #[error("Destination output directory already exists at {0}. Use --replace to overwrite.")]
    DestinationOutputExists(PathBuf),

    #[error("Export source path does not exist: {0}")]
    SourceNotFound(PathBuf),

    #[error("ZIP archive extraction failed: {0}")]
    ZipExtractionFailed(String),

    #[error("Unrecognized or unsupported Telegram Desktop export format in: {0}")]
    UnrecognizedExportFormat(PathBuf),

    #[error("Failed to parse export data: {0}")]
    ParsingFailed(String),

    #[error("I/O error during import: {0}")]
    Io(#[from] std::io::Error),

    #[error("Storage error: {0}")]
    Storage(#[from] vendetta_storage::StorageError),

    #[error("Render error: {0}")]
    Render(#[from] vendetta_render::RenderError),

    #[error("JSON serialization/deserialization error: {0}")]
    Json(#[from] serde_json::Error),
}
