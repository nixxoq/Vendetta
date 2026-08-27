use thiserror::Error;
use vendetta_model::PeerId;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("Grammers invocation error: {0}")]
    Invocation(String),

    #[error("Session persistence error: {0}")]
    Session(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Rate limited / FLOOD_WAIT: {seconds} seconds")]
    FloodWait { seconds: u32 },

    #[error("Premium rate limited / FLOOD_PREMIUM_WAIT: {seconds} seconds")]
    FloodPremiumWait { seconds: u32 },

    #[error("File reference expired")]
    FileReferenceExpired,

    #[error("File reference invalid")]
    FileReferenceInvalid,

    #[error("File migrated to DC {0}")]
    FileMigrate(i32),

    #[error("CDN redirect received for DC {dc_id} (unsupported in Milestone 5)")]
    CdnRedirectUnsupported { dc_id: i32, file_token: Vec<u8> },

    #[error("Chunk integrity mismatch at offset {offset}: expected {expected}, actual {actual}")]
    ChunkIntegrityMismatch {
        offset: i64,
        expected: String,
        actual: String,
    },

    #[error("Not authenticated")]
    NotAuthenticated,

    #[error("Sign up required with official mobile client")]
    SignUpRequired,

    #[error("Invalid authentication code")]
    InvalidCode,

    #[error("Two-factor authentication password required (hint: {hint:?})")]
    PasswordRequired { hint: Option<String> },

    #[error("Invalid two-factor authentication password")]
    InvalidPassword,

    #[error("Authentication flow in invalid state: {0}")]
    InvalidAuthState(String),

    #[error("Peer {0} not found or uncached in session")]
    PeerNotFoundOrUncached(PeerId),

    #[error("Missing access hash for peer {0}")]
    MissingAccessHash(PeerId),

    #[error("Invalid peer type for peer {peer_id} (expected {expected})")]
    InvalidPeerType {
        peer_id: PeerId,
        expected: &'static str,
    },

    #[error("Cannot determine peer type for peer {0}")]
    UnknownPeerType(PeerId),

    #[error("Peer not found or inaccessible: {0}")]
    NotFound(String),

    #[error("Client disconnected")]
    Disconnected,

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

    #[error("Channel sync failed for peer {peer_id}: {reason}")]
    ChannelSyncError { peer_id: PeerId, reason: String },
}

pub type AdapterResult<T> = std::result::Result<T, AdapterError>;

impl From<grammers_mtsender::InvocationError> for AdapterError {
    fn from(err: grammers_mtsender::InvocationError) -> Self {
        match err {
            grammers_mtsender::InvocationError::Rpc(rpc) => {
                let name = &rpc.name;
                if rpc.code == 303 || name.starts_with("FILE_MIGRATE_") {
                    let dc = rpc.value.unwrap_or(0) as i32;
                    AdapterError::FileMigrate(dc)
                } else if name == "FILE_REFERENCE_EXPIRED" || name == "FILEREF_UPGRADE_NEEDED" {
                    AdapterError::FileReferenceExpired
                } else if name == "FILE_REFERENCE_INVALID" {
                    AdapterError::FileReferenceInvalid
                } else if name.starts_with("FLOOD_PREMIUM_WAIT_") {
                    AdapterError::FloodPremiumWait {
                        seconds: rpc.value.unwrap_or(1),
                    }
                } else if rpc.code == 420 || name == "FLOOD_WAIT" || name.starts_with("FLOOD_WAIT_")
                {
                    AdapterError::FloodWait {
                        seconds: rpc.value.unwrap_or(1),
                    }
                } else if rpc.code == 401 {
                    AdapterError::NotAuthenticated
                } else {
                    AdapterError::Invocation(format!("RPC {}: {}", rpc.code, rpc.name))
                }
            }
            grammers_mtsender::InvocationError::Dropped => AdapterError::Disconnected,
            other => AdapterError::Invocation(other.to_string()),
        }
    }
}

impl From<grammers_client::SignInError> for AdapterError {
    fn from(err: grammers_client::SignInError) -> Self {
        match err {
            grammers_client::SignInError::SignUpRequired => AdapterError::SignUpRequired,
            grammers_client::SignInError::InvalidCode => AdapterError::InvalidCode,
            grammers_client::SignInError::PasswordRequired(token) => {
                AdapterError::PasswordRequired {
                    hint: token.hint().map(|s| s.to_string()),
                }
            }
            grammers_client::SignInError::InvalidPassword(_token) => AdapterError::InvalidPassword,
            grammers_client::SignInError::Other(invoc) => invoc.into(),
        }
    }
}

impl From<AdapterError> for vendetta_core::VendettaError {
    fn from(err: AdapterError) -> Self {
        vendetta_core::VendettaError::Adapter(err.to_string())
    }
}
