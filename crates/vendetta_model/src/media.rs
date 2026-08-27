use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString, VariantArray};

use crate::{message::MessageKey, peer::PeerId};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MediaKind {
    Photo,
    Video,
    Document,
    Audio,
    Voice,
    Sticker,
    Animation,
    Thumbnail,
    VideoNote,
    Other,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MediaDownloadStatus {
    Pending,
    Resolving,
    Downloading,
    Paused,
    RetryWait,
    Completed,
    VerificationFailed,
    NeedsReauth,
    NeedsFileReferenceRefresh,
    NeedsDcMigration,
    PermanentlyFailed,
    Skipped,
}

impl MediaDownloadStatus {
    pub const ALL: &'static [Self] = <Self as VariantArray>::VARIANTS;
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MediaVerificationStatus {
    Unverified,
    Verified,
    CorruptedHash,
    CorruptedSize,
    MissingFile,
}

impl MediaVerificationStatus {
    pub const ALL: &'static [Self] = <Self as VariantArray>::VARIANTS;
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MediaRole {
    Attachment,
    Thumbnail,
    Sticker,
    Voice,
    VideoNote,
    AlternativeQuality,
    StreamingManifest,
    Storyboard,
}

impl MediaRole {
    pub fn is_primary_attachment(&self) -> bool {
        matches!(
            self,
            Self::Attachment | Self::Voice | Self::VideoNote | Self::Sticker
        )
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FilterDecision {
    Allow,
    Skip,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FilterReason {
    TypeExcluded,
    SizeBelowMin,
    SizeAboveMax,
    MimeExcluded,
    ExtExcluded,
    UnsupportedLocation,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRangeHash {
    pub offset: i64,
    pub limit: i32,
    pub hash: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRecord {
    pub media_id: String,
    pub kind: MediaKind,
    pub mime_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub file_name: Option<String>,
    pub size_type: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub dc_id: i32,
    pub source_location_tl: Option<Vec<u8>>,
    pub file_reference: Option<Vec<u8>>,
    pub local_rel_path: Option<String>,
    pub sha256: Option<String>,
    pub download_status: MediaDownloadStatus,
    pub downloaded_bytes: i64,
    pub chunk_size: i32,
    pub retry_count: i32,
    pub max_retries: i32,
    pub next_retry_at: Option<i64>,
    pub claimed_at: Option<i64>,
    pub worker_id: Option<String>,
    pub last_error: Option<String>,
    pub filter_decision: Option<FilterDecision>,
    pub filter_reason: Option<FilterReason>,
    pub policy_version: i32,
    pub verification_status: MediaVerificationStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageMediaJoin {
    pub key: MessageKey,
    pub media_id: String,
    pub role: MediaRole,
    pub position: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaFilterPolicy {
    pub allow_photos: bool,
    pub allow_videos: bool,
    pub allow_documents: bool,
    pub allow_audio: bool,
    pub allow_voice: bool,
    pub allow_stickers: bool,
    pub allow_animations: bool,
    pub allow_video_notes: bool,
    pub min_size_bytes: Option<i64>,
    pub max_size_bytes: Option<i64>,
    pub allowed_mime_types: Option<Vec<String>>,
    pub blocked_mime_types: Option<Vec<String>>,
    pub allowed_extensions: Option<Vec<String>>,
    pub blocked_extensions: Option<Vec<String>>,
    pub target_peers: Option<Vec<PeerId>>,
    pub policy_version: i32,
}

impl Default for MediaFilterPolicy {
    fn default() -> Self {
        Self {
            allow_photos: true,
            allow_videos: true,
            allow_documents: true,
            allow_audio: true,
            allow_voice: true,
            allow_stickers: true,
            allow_animations: true,
            allow_video_notes: true,
            min_size_bytes: None,
            max_size_bytes: None,
            allowed_mime_types: None,
            blocked_mime_types: None,
            allowed_extensions: None,
            blocked_extensions: None,
            target_peers: None,
            policy_version: 1,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaStats {
    pub total_count: i64,
    pub pending_count: i64,
    pub resolving_count: i64,
    pub downloading_count: i64,
    pub paused_count: i64,
    pub retry_wait_count: i64,
    pub completed_count: i64,
    pub verification_failed_count: i64,
    pub needs_reauth_count: i64,
    pub needs_file_ref_refresh_count: i64,
    pub needs_dc_migration_count: i64,
    pub permanently_failed_count: i64,
    pub skipped_count: i64,
    pub failed_count: i64,

    pub unverified_count: i64,
    pub verified_count: i64,
    pub corrupted_hash_count: i64,
    pub corrupted_size_count: i64,
    pub missing_file_count: i64,
    pub corrupted_count: i64,

    pub total_size_bytes: i64,
    pub downloaded_size_bytes: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaQueueStats {
    pub eligible_count: usize,
    pub expected_bytes: u64,
    pub all_sizes_known: bool,
}
