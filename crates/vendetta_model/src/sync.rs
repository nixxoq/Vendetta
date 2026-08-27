use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString, VariantArray};

use crate::{
    message::{MessageId, MessageRecord},
    peer::{PeerId, PeerRecord, PeerType},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStateRecord {
    pub peer_id: PeerId,
    pub pts: Option<i32>,
    pub qts: Option<i32>,
    pub date: Option<i32>,
    pub seq: Option<i32>,
    pub min_message_id: Option<i64>,
    pub max_message_id: Option<i64>,
    pub last_synced_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSyncState {
    pub account_id: String,
    pub pts: i32,
    pub qts: i32,
    pub date: i32,
    pub seq: i32,
    #[serde(default)]
    pub sync_uncertain: bool,
    pub last_synced_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerSyncState {
    pub peer_id: PeerId,
    pub pts: Option<i32>,
    pub min_message_id: Option<MessageId>,
    pub max_message_id: Option<MessageId>,
    pub has_gap: bool,
    #[serde(default)]
    pub sync_uncertain: bool,
    pub poll_timeout_secs: Option<i32>,
    pub last_synced_at: i64,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SyncBaselineStatus {
    InProgress,
    Completed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncBaseline {
    pub baseline_id: String,
    pub common_pts: i32,
    pub common_qts: i32,
    pub common_date: i32,
    pub common_seq: i32,
    pub status: SyncBaselineStatus,
    pub captured_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ChannelQueueStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelQueueItem {
    pub peer_id: PeerId,
    pub discovered_pts: i32,
    pub current_pts: Option<i32>,
    pub status: ChannelQueueStatus,
    pub attempts: i32,
    pub poll_timeout: Option<i32>,
    pub last_error: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommonDeletionTombstone {
    pub message_id: MessageId,
    pub pts: Option<i32>,
    pub pts_count: i32,
    pub observed_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncIntegrityReport {
    pub scope: String,
    pub peer_id: Option<PeerId>,
    pub fully_lossless_contiguous_sync: bool,
    pub current_history_repaired: bool,
    pub new_messages_recovered: bool,
    pub current_content_reconciled: bool,
    pub historical_edits_complete: bool,
    pub historical_deletions_complete: bool,
    pub event_window_lost: bool,
    pub channel_discovery_complete: bool,
    pub gap_summary: Option<String>,
    pub created_at: i64,
    #[serde(default = "default_provenance_version")]
    pub provenance_version: u32,
    #[serde(default)]
    pub deletion_reconciliation_performed: bool,
    #[serde(default)]
    pub deletion_reconciliation_complete: bool,
    #[serde(default)]
    pub deletion_event_gap_count: u32,
    #[serde(default)]
    pub deletion_tombstones_reconciled: usize,
    #[serde(default)]
    pub historical_message_reconciliation_performed: bool,
    #[serde(default)]
    pub historical_message_reconciliation_complete: bool,
    #[serde(default)]
    pub historical_message_gap_count: u32,
}

const fn default_provenance_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizedUpdate {
    NewMessage {
        message: MessageRecord,
        pts: Option<i32>,
        pts_count: i32,
    },
    EditedMessage {
        message: MessageRecord,
        pts: Option<i32>,
        pts_count: i32,
    },
    CommonDeletedMessages {
        message_ids: Vec<MessageId>,
        pts: Option<i32>,
        pts_count: i32,
    },
    ChannelDeletedMessages {
        channel_id: PeerId,
        message_ids: Vec<MessageId>,
        pts: Option<i32>,
        pts_count: i32,
    },
    ChannelCatchupRequired {
        channel_id: PeerId,
        pts: Option<i32>,
    },
    PeerDiscovered {
        peer: PeerRecord,
    },
    ServiceAction {
        peer_id: PeerId,
        message_id: MessageId,
        actor_id: Option<PeerId>,
        date: i64,
        action_text: String,
        raw_tl: Vec<u8>,
        pts: Option<i32>,
        pts_count: i32,
    },
    Unsupported {
        constructor_name: String,
        constructor_id: u32,
        affects_sync_state: bool,
        pts: Option<i32>,
        pts_count: i32,
        qts: Option<i32>,
        qts_count: i32,
        diagnostic_info: Option<String>,
        raw_tl: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedEventRecord {
    pub event_id: i64,
    pub peer_id: Option<PeerId>,
    pub constructor_id: u32,
    pub pts: Option<i32>,
    pub pts_count: Option<i32>,
    pub qts: Option<i32>,
    pub qts_count: Option<i32>,
    pub affects_sync_state: bool,
    pub diagnostic_info: Option<String>,
    pub raw_tl: Vec<u8>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogInfo {
    pub peer_id: PeerId,
    pub peer_type: Option<PeerType>,
    pub pts: Option<i32>,
    pub top_message: Option<MessageId>,
    pub unread_count: i32,
    pub is_pinned: bool,
    pub folder_id: Option<i32>,
    #[serde(default)]
    pub is_unresolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogFilterRecord {
    pub id: i32,
    pub title: String,
    pub pinned_peers: Vec<PeerId>,
    pub include_peers: Vec<PeerId>,
    pub exclude_peers: Vec<PeerId>,
}
