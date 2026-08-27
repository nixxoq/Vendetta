use thiserror::Error;
use vendetta_model::{MessageId, PeerId};
use vendetta_storage::StorageError;
use vendetta_tg_adapter::AdapterError;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("Telegram adapter error: {0}")]
    Adapter(#[from] AdapterError),

    #[error("Synchronization cancelled")]
    Cancelled,

    #[error(
        "Non-progressing pagination detected for peer {peer_id}: offset {offset_id:?}, returned min ID {returned_min_id:?}"
    )]
    NonProgressingPagination {
        peer_id: PeerId,
        offset_id: Option<MessageId>,
        returned_min_id: Option<MessageId>,
    },

    #[error("Corrupted sync state: {0}")]
    CorruptedState(String),

    #[error(
        "Unsupported state-affecting update (constructor 0x{constructor_id:08x}, pts: {pts:?}, pts_count: {pts_count})"
    )]
    UnsupportedStateAffectingUpdate {
        constructor_id: u32,
        pts: Option<i32>,
        pts_count: i32,
    },

    #[error("Channel discovery incomplete: {0}")]
    ChannelDiscoveryIncomplete(String),

    #[error("Difference buffer overflow: server PTS {pts}")]
    DifferenceBufferOverflow { pts: i32 },

    #[error("PTS sequence discontinuity: expected {expected}, received {received}")]
    PtsSequenceDiscontinuity { expected: i32, received: i32 },

    #[error("Missing authoritative channel synchronization baseline for peer {peer_id}")]
    MissingChannelBaseline { peer_id: PeerId },
}

pub type SyncResult<T> = Result<T, SyncError>;

impl From<SyncError> for vendetta_core::VendettaError {
    fn from(err: SyncError) -> Self {
        vendetta_core::VendettaError::Sync(err.to_string())
    }
}
