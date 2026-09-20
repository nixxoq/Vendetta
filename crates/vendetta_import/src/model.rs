use std::path::PathBuf;

use grammers_tl_types::{self as tl, Serializable};
use serde::{Deserialize, Serialize};
use vendetta_model::{
    FilterReason, MediaKind, MediaRole, MessageId, MessageKey, MessageRecord, MessageState, PeerId,
    PeerRecord, PeerType,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportChat {
    pub peer_id: PeerId,
    pub peer_type: PeerType,
    pub name: Option<String>,
    pub username: Option<String>,
    pub messages: Vec<ImportMessage>,
}

impl ImportChat {
    pub fn to_peer_record(&self) -> PeerRecord {
        let latest_date = self
            .messages
            .iter()
            .map(|m| m.date)
            .max()
            .unwrap_or_else(vendetta_core::now_unix_secs);

        PeerRecord {
            peer_id: self.peer_id,
            peer_type: self.peer_type,
            name: self.name.clone(),
            username: self.username.clone(),
            phone: None,
            raw_tl: None,
            updated_at: latest_date,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportMessage {
    pub message_id: MessageId,
    pub date: i64,
    pub sender_id: Option<PeerId>,
    pub sender_name: Option<String>,
    pub text: Option<String>,
    pub entities: Vec<ImportTextEntity>,
    pub edit_date: Option<i64>,
    pub reply_to_message_id: Option<MessageId>,
    pub forward_info: Option<ImportForwardInfo>,
    pub reactions: Vec<ImportReaction>,
    pub media: Vec<ImportMediaItem>,
    pub service_event: Option<ImportServiceEvent>,
    pub is_joined_continuation: bool,
    pub is_outgoing: bool,
}

impl ImportMessage {
    pub fn to_message_record(&self, chat_id: PeerId) -> MessageRecord {
        let entities_json = (!self.entities.is_empty())
            .then(|| {
                self.entities
                    .iter()
                    .map(ImportTextEntity::to_tl)
                    .collect::<Vec<_>>()
            })
            .and_then(|tl_entities| serde_json::to_string(&tl_entities).ok());

        let forward_json = self.forward_info.as_ref().map(|f| {
            serde_json::json!({
                "from_name": f.from_name,
                "date": f.date,
            })
            .to_string()
        });

        let reactions_json = (!self.reactions.is_empty()).then(|| {
            let results: Vec<_> = self
                .reactions
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "reaction": { "Emoji": { "emoticon": r.emoji } },
                        "count": r.count
                    })
                })
                .collect();

            serde_json::json!({
                "can_see_list": true,
                "results": results,
                "recent_reactions": []
            })
            .to_string()
        });

        let state = self
            .edit_date
            .map_or(MessageState::Active, |_| MessageState::Edited);

        MessageRecord {
            key: MessageKey::new(chat_id, self.message_id),
            date: self.date,
            sender_id: self.sender_id,
            text: self.text.clone(),
            entities_json,
            edit_date: self.edit_date,
            state,
            reply_to_msg_id: self.reply_to_message_id,
            reply_to_top_id: None,
            reply_to_peer_id: self.reply_to_message_id.map(|_| chat_id),
            grouped_id: None,
            forward_json,
            reactions_json,
            views: None,
            forwards_count: None,
            raw_tl: Some(self.to_tl_bytes(chat_id)),
        }
    }

    fn to_tl_bytes(&self, chat_id: PeerId) -> Vec<u8> {
        let peer = peer_id_to_tl_user(chat_id);

        match &self.service_event {
            Some(event) => tl::enums::Message::Service(tl::types::MessageService {
                out: self.is_outgoing,
                mentioned: false,
                media_unread: false,
                reactions_are_possible: false,
                reactions: None,
                silent: false,
                post: false,
                legacy: false,
                id: self.message_id.raw() as i32,
                from_id: None,
                peer_id: peer,
                saved_peer_id: None,
                reply_to: None,
                date: self.date as i32,
                action: event.to_tl_action(),
                ttl_period: None,
            })
            .to_bytes(),
            None => tl::enums::Message::Message(tl::types::Message {
                out: self.is_outgoing,
                mentioned: false,
                media_unread: false,
                silent: false,
                post: false,
                from_scheduled: false,
                legacy: false,
                edit_hide: false,
                pinned: false,
                noforwards: false,
                invert_media: false,
                offline: false,
                video_processing_pending: false,
                paid_suggested_post_stars: false,
                paid_suggested_post_ton: false,
                id: self.message_id.raw() as i32,
                from_id: self.sender_id.map(peer_id_to_tl_user),
                from_boosts_applied: None,
                from_rank: None,
                peer_id: peer,
                saved_peer_id: None,
                fwd_from: None,
                via_bot_id: None,
                via_business_bot_id: None,
                guestchat_via_from: None,
                reply_to: None,
                date: self.date as i32,
                message: self.text.clone().unwrap_or_default(),
                media: None,
                reply_markup: None,
                entities: None,
                views: None,
                forwards: None,
                replies: None,
                edit_date: self.edit_date.map(|d| d as i32),
                post_author: None,
                grouped_id: None,
                reactions: None,
                restriction_reason: None,
                ttl_period: None,
                quick_reply_shortcut_id: None,
                effect: None,
                factcheck: None,
                report_delivery_until_date: None,
                paid_message_stars: None,
                suggested_post: None,
                schedule_repeat_period: None,
                summary_from_language: None,
                rich_message: None,
            })
            .to_bytes(),
        }
    }
}

fn peer_id_to_tl_user(id: PeerId) -> tl::enums::Peer {
    tl::enums::Peer::User(tl::types::PeerUser {
        user_id: id.raw().unsigned_abs() as i64,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportTextEntity {
    pub kind: ImportEntityKind,
    pub offset_utf16: usize,
    pub length_utf16: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportEntityKind {
    Bold,
    Italic,
    Underline,
    Strike,
    Code,
    Pre(Option<String>),
    TextUrl(String),
    Spoiler,
    Blockquote,
    Mention,
    Hashtag,
    CustomEmoji(i64),
}

impl ImportTextEntity {
    pub fn to_tl(&self) -> tl::enums::MessageEntity {
        let offset = self.offset_utf16 as i32;
        let length = self.length_utf16 as i32;

        match &self.kind {
            ImportEntityKind::Bold => {
                tl::enums::MessageEntity::Bold(tl::types::MessageEntityBold { offset, length })
            }
            ImportEntityKind::Italic => {
                tl::enums::MessageEntity::Italic(tl::types::MessageEntityItalic { offset, length })
            }
            ImportEntityKind::Underline => {
                tl::enums::MessageEntity::Underline(tl::types::MessageEntityUnderline {
                    offset,
                    length,
                })
            }
            ImportEntityKind::Strike => {
                tl::enums::MessageEntity::Strike(tl::types::MessageEntityStrike { offset, length })
            }
            ImportEntityKind::Code => {
                tl::enums::MessageEntity::Code(tl::types::MessageEntityCode { offset, length })
            }
            ImportEntityKind::Pre(lang) => {
                tl::enums::MessageEntity::Pre(tl::types::MessageEntityPre {
                    offset,
                    length,
                    language: lang.clone().unwrap_or_default(),
                })
            }
            ImportEntityKind::TextUrl(url) => {
                tl::enums::MessageEntity::TextUrl(tl::types::MessageEntityTextUrl {
                    offset,
                    length,
                    url: url.clone(),
                })
            }
            ImportEntityKind::Spoiler => {
                tl::enums::MessageEntity::Spoiler(tl::types::MessageEntitySpoiler {
                    offset,
                    length,
                })
            }
            ImportEntityKind::Blockquote => {
                tl::enums::MessageEntity::Blockquote(tl::types::MessageEntityBlockquote {
                    offset,
                    length,
                    collapsed: false,
                })
            }
            ImportEntityKind::Mention => {
                tl::enums::MessageEntity::Mention(tl::types::MessageEntityMention {
                    offset,
                    length,
                })
            }
            ImportEntityKind::Hashtag => {
                tl::enums::MessageEntity::Hashtag(tl::types::MessageEntityHashtag {
                    offset,
                    length,
                })
            }
            ImportEntityKind::CustomEmoji(doc_id) => {
                tl::enums::MessageEntity::CustomEmoji(tl::types::MessageEntityCustomEmoji {
                    offset,
                    length,
                    document_id: *doc_id,
                })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportForwardInfo {
    pub from_name: Option<String>,
    pub date: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReaction {
    pub emoji: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportMediaItem {
    pub source_path: Option<PathBuf>,
    pub file_name: Option<String>,
    pub kind: MediaKind,
    pub mime_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub is_skipped: bool,
    pub skip_reason: Option<FilterReason>,
    pub role: MediaRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportServiceEvent {
    ActionText(String),
    PinMessage,
    EditTitle(String),
    EditPhoto,
    DeletePhoto,
}

impl ImportServiceEvent {
    pub fn to_tl_action(&self) -> tl::enums::MessageAction {
        match self {
            Self::ActionText(text) => {
                tl::enums::MessageAction::CustomAction(tl::types::MessageActionCustomAction {
                    message: text.clone(),
                })
            }
            Self::PinMessage => tl::enums::MessageAction::PinMessage,
            Self::EditTitle(title) => {
                tl::enums::MessageAction::ChatEditTitle(tl::types::MessageActionChatEditTitle {
                    title: title.clone(),
                })
            }
            Self::EditPhoto => {
                tl::enums::MessageAction::ChatEditPhoto(tl::types::MessageActionChatEditPhoto {
                    photo: tl::enums::Photo::Empty(tl::types::PhotoEmpty { id: 0 }),
                })
            }
            Self::DeletePhoto => tl::enums::MessageAction::ChatDeletePhoto,
        }
    }
}
