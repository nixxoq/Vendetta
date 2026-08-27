use thiserror::Error;
use vendetta_tg_adapter::AdapterError;

#[derive(Debug, Error)]
pub enum MediaEngineError {
    #[error("Storage error: {0}")]
    Storage(#[from] vendetta_storage::StorageError),

    #[error("Telegram adapter error: {0}")]
    Adapter(#[from] AdapterError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Rate limited / FLOOD_WAIT: {seconds} seconds")]
    FloodWait { seconds: u32 },

    #[error("Premium rate limited / FLOOD_PREMIUM_WAIT: {seconds} seconds")]
    FloodPremiumWait { seconds: u32 },

    #[error("File reference expired for media {0}")]
    FileReferenceExpired(String),

    #[error("File reference invalid for media {0}")]
    FileReferenceInvalid(String),

    #[error("File migrated to DC {0}")]
    FileMigrate(i32),

    #[error("CDN redirect received (deferred/unsupported in Milestone 5): DC {dc_id}")]
    CdnRedirectUnsupported { dc_id: i32 },

    #[error(
        "Chunk integrity verification mismatch at offset {offset}: expected {expected}, actual {actual}"
    )]
    ChunkHashMismatch {
        offset: i64,
        expected: String,
        actual: String,
    },

    #[error("Final size mismatch for {media_id}: expected {expected} bytes, got {actual} bytes")]
    FinalSizeMismatch {
        media_id: String,
        expected: i64,
        actual: i64,
    },

    #[error("Final SHA-256 mismatch for {media_id}: expected {expected}, got {actual}")]
    FinalHashMismatch {
        media_id: String,
        expected: String,
        actual: String,
    },

    #[error("Unsupported media location / no downloadable location: {0}")]
    UnsupportedLocation(String),

    #[error("Media download retry limit exceeded: {0}")]
    RetryLimitExceeded(String),

    #[error("File verification failed: {0}")]
    VerificationFailed(String),

    #[error("Chunk planner error: {0}")]
    ChunkPlanner(#[from] crate::downloader::ChunkPlannerError),

    #[error("Corrupted progress for {media_id} (downloaded {downloaded_bytes} bytes): {reason}")]
    CorruptedProgress {
        media_id: String,
        downloaded_bytes: i64,
        reason: String,
    },

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Internal media error: {0}")]
    Internal(String),
}

pub type MediaEngineResult<T> = std::result::Result<T, MediaEngineError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryAction {
    RetryImmediately,
    RetryAfterDelay { seconds: u32 },
    RefreshAndRetry,
    MigrateAndRetry { new_dc: i32 },
    PauseForAuth,
    PermanentFailure,
}

impl MediaEngineError {
    pub fn classify_retry_action(&self) -> RetryAction {
        match self {
            Self::FloodWait { seconds } | Self::FloodPremiumWait { seconds } => {
                RetryAction::RetryAfterDelay { seconds: *seconds }
            }
            Self::FileReferenceExpired(_) | Self::FileReferenceInvalid(_) => {
                RetryAction::RefreshAndRetry
            }
            Self::FileMigrate(dc) => RetryAction::MigrateAndRetry { new_dc: *dc },
            Self::ChunkHashMismatch { .. } => RetryAction::RetryImmediately,
            Self::Adapter(AdapterError::FloodWait { seconds })
            | Self::Adapter(AdapterError::FloodPremiumWait { seconds }) => {
                RetryAction::RetryAfterDelay { seconds: *seconds }
            }
            Self::Adapter(AdapterError::FileReferenceExpired)
            | Self::Adapter(AdapterError::FileReferenceInvalid) => RetryAction::RefreshAndRetry,
            Self::Adapter(AdapterError::FileMigrate(dc)) => {
                RetryAction::MigrateAndRetry { new_dc: *dc }
            }
            Self::Adapter(AdapterError::NotAuthenticated) => RetryAction::PauseForAuth,
            Self::Adapter(AdapterError::Disconnected) => {
                RetryAction::RetryAfterDelay { seconds: 2 }
            }
            Self::Io(_) => RetryAction::RetryAfterDelay { seconds: 1 },
            Self::FinalSizeMismatch { .. }
            | Self::FinalHashMismatch { .. }
            | Self::CorruptedProgress { .. } => RetryAction::RetryImmediately,
            Self::UnsupportedLocation(_)
            | Self::RetryLimitExceeded(_)
            | Self::CdnRedirectUnsupported { .. } => RetryAction::PermanentFailure,
            _ => RetryAction::PermanentFailure,
        }
    }
}
