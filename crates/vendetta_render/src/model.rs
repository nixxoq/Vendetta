use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString, VariantArray};
use vendetta_model::{MediaRecord, MessageId, MessageKey, MessageState, PeerId, PeerType};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case", ascii_case_insensitive)]
pub enum PresentationMode {
    #[default]
    #[strum(
        serialize = "telegram-like",
        serialize = "telegram_like",
        serialize = "telegram"
    )]
    TelegramLike,
    #[strum(
        serialize = "archive-optimized",
        serialize = "archive_optimized",
        serialize = "archive",
        serialize = "dense"
    )]
    ArchiveOptimized,
}

impl PresentationMode {
    pub fn parse(s: &str) -> Option<Self> {
        s.parse().ok()
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case", ascii_case_insensitive)]
pub enum MediaMode {
    #[default]
    Copy,
    #[strum(serialize = "link", serialize = "symlink")]
    Link,
}

impl MediaMode {
    pub fn parse(s: &str) -> Option<Self> {
        s.parse().ok()
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case", ascii_case_insensitive)]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    #[strum(serialize = "system", serialize = "auto")]
    System,
}

impl ThemeMode {
    pub fn parse(s: &str) -> Option<Self> {
        s.parse().ok()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptions {
    pub output_dir: PathBuf,
    pub presentation_mode: PresentationMode,
    pub media_mode: MediaMode,
    pub theme: ThemeMode,
    pub chunk_size: usize,
    pub replace: bool,
    pub media_src_dir: Option<PathBuf>,
    pub include_service_messages: bool,
    pub include_deleted_messages: bool,
    pub include_edit_history: bool,
    pub build_search_index: bool,
    pub build_date_index: bool,
    pub target_peers: Option<Vec<PeerId>>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("export_html"),
            presentation_mode: PresentationMode::TelegramLike,
            media_mode: MediaMode::Copy,
            theme: ThemeMode::System,
            chunk_size: 250,
            replace: false,
            media_src_dir: None,
            include_service_messages: true,
            include_deleted_messages: true,
            include_edit_history: true,
            build_search_index: true,
            build_date_index: true,
            target_peers: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderTopic {
    pub topic_id: i32,
    pub title: String,
    pub icon_color: Option<i32>,
    pub icon_emoji_id: Option<i64>,
    pub icon_asset: Option<String>,
    pub total_messages: usize,
    pub last_message_date: Option<i64>,
    pub is_general: bool,
    pub is_closed: bool,
    pub is_pinned: bool,
    pub is_hidden: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPeer {
    pub peer_id: PeerId,
    pub peer_type: PeerType,
    pub name: String,
    pub username: Option<String>,
    pub phone: Option<String>,
    pub total_messages: usize,
    pub last_message_date: Option<i64>,
    pub is_forum: bool,
    pub topics: Vec<RenderTopic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderRevision {
    pub captured_at: i64,
    pub edit_date: Option<i64>,
    pub formatted_html: String,
    pub raw_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderReplyPreview {
    pub target_key: MessageKey,
    pub sender_name: Option<String>,
    pub text_snippet: Option<String>,
    pub media_indicator: Option<String>,
    pub state: MessageState,
    pub target_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RenderForwardInfo {
    pub source_peer_id: Option<PeerId>,
    pub source_peer_type: Option<PeerType>,
    pub origin_name: Option<String>,
    pub source_username: Option<String>,
    pub origin_channel_post: Option<i64>,
    pub origin_date: Option<i64>,
    pub origin_signature: Option<String>,
    pub source_avatar_markup: Option<String>,
    pub is_source_archived: bool,
    pub source_chat_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderMediaItem {
    pub record: MediaRecord,
    pub relative_url: Option<String>,
    pub is_available: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderMessage {
    pub key: MessageKey,
    pub date: i64,
    pub sender_id: Option<PeerId>,
    pub sender_name: Option<String>,
    pub is_outgoing: bool,
    pub state: MessageState,
    pub formatted_html: Option<String>,
    pub raw_text: Option<String>,
    pub reply_preview: Option<RenderReplyPreview>,
    pub forward_info: Option<RenderForwardInfo>,
    pub media_items: Vec<RenderMediaItem>,
    pub revisions: Vec<RenderRevision>,
    pub grouped_id: Option<i64>,
    pub is_service: bool,
    pub service_description: Option<String>,
    pub views: Option<i32>,
    pub forwards_count: Option<i32>,
    pub author_signature: Option<String>,
    pub reply_to_top_id: Option<MessageId>,
    pub reactions: Vec<RenderReactionGroup>,
    pub is_channel_post: bool,
    pub comments_count: Option<i32>,
    pub has_comments: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum RenderReactionKey {
    Emoji(String),
    CustomEmoji {
        document_id: i64,
        alt_text: Option<String>,
        asset_rel_path: Option<String>,
    },
    Paid,
    Unknown(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderReactor {
    pub peer_id: PeerId,
    pub name: String,
    pub username: Option<String>,
    pub avatar_markup: Option<String>,
    pub is_me: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderReactionGroup {
    pub reaction: RenderReactionKey,
    pub count: usize,
    pub is_chosen_by_me: bool,
    pub reactors: Vec<RenderReactor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderAlbum {
    pub grouped_id: i64,
    pub messages: Vec<RenderMessage>,
    pub media_items: Vec<RenderMediaItem>,
    pub continuation_prev: bool,
    pub continuation_next: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RenderItem {
    Message(Box<RenderMessage>),
    Album(RenderAlbum),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportSummary {
    pub dialogs_count: usize,
    pub messages_count: usize,
    pub chunks_count: usize,
    pub media_copied_count: usize,
    pub search_shards_count: usize,
    pub deleted_messages_count: usize,
    pub edited_messages_count: usize,
    pub manifest_path: PathBuf,
}
