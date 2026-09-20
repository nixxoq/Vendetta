use std::collections::HashMap;

use grammers_tl_types::Deserializable;
use vendetta_model::{MediaKind, MessageKey, MessageState};
use vendetta_storage::ArchiveDb;

use crate::{entity::html_escape, model::RenderReplyPreview, url_builder::ArchiveUrlBuilder};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageLocation {
    pub page_index: usize,
    pub topic_id: Option<i32>,
    pub page_file: String,
}

#[derive(Debug, Clone, Default)]
pub struct ReplyLocationMap {
    locations: HashMap<MessageKey, MessageLocation>,
}

impl ReplyLocationMap {
    pub fn new() -> Self {
        Self {
            locations: HashMap::new(),
        }
    }

    pub fn insert(&mut self, key: MessageKey, page_index: usize, topic_id: Option<i32>) {
        let page_file = ArchiveUrlBuilder::page_file_name(page_index);
        self.insert_with_file(key, page_index, topic_id, page_file);
    }

    pub fn insert_with_file(
        &mut self,
        key: MessageKey,
        page_index: usize,
        topic_id: Option<i32>,
        page_file: String,
    ) {
        self.locations.insert(
            key,
            MessageLocation {
                page_index,
                topic_id,
                page_file,
            },
        );
    }

    pub fn get_location(&self, key: &MessageKey) -> Option<(usize, Option<i32>)> {
        self.locations
            .get(key)
            .map(|loc| (loc.page_index, loc.topic_id))
    }

    pub fn get_location_full(&self, key: &MessageKey) -> Option<&MessageLocation> {
        self.locations.get(key)
    }

    pub fn get_page(&self, key: &MessageKey) -> Option<usize> {
        self.locations.get(key).map(|loc| loc.page_index)
    }

    pub fn get_page_file(&self, key: &MessageKey) -> Option<&str> {
        self.locations.get(key).map(|loc| loc.page_file.as_str())
    }
}

pub struct ReplyResolver<'a> {
    db: &'a ArchiveDb,
    location_map: &'a ReplyLocationMap,
}

fn media_kind_indicator(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Photo | MediaKind::Thumbnail => "Photo",
        MediaKind::Video => "Video",
        MediaKind::Audio => "Audio",
        MediaKind::Voice => "Voice message",
        MediaKind::Document => "Document",
        MediaKind::Sticker => "Sticker",
        MediaKind::VideoNote => "Video message",
        MediaKind::Animation => "GIF",
        MediaKind::Other => "Attachment",
    }
}

impl<'a> ReplyResolver<'a> {
    pub fn new(db: &'a ArchiveDb, location_map: &'a ReplyLocationMap) -> Self {
        Self { db, location_map }
    }

    fn resolve_sender_name(&self, target_msg: &vendetta_model::MessageRecord) -> String {
        let peer_id = target_msg.sender_id.unwrap_or(target_msg.key.peer_id);
        if let Ok(Some(peer)) = self.db.get_peer(peer_id) {
            if let Some(name) = &peer.name {
                let t = name.trim();
                if !t.is_empty() && t != "Unknown" {
                    return t.to_string();
                }
            }
            if let Some(ref raw) = peer.raw_tl {
                if let Ok(grammers_tl_types::enums::Chat::Channel(c)) =
                    grammers_tl_types::enums::Chat::from_bytes(raw)
                {
                    let t = c.title.trim();
                    if !t.is_empty() && t != "Unknown" {
                        return t.to_string();
                    }
                } else if let Ok(grammers_tl_types::enums::Chat::Chat(c)) =
                    grammers_tl_types::enums::Chat::from_bytes(raw)
                {
                    let t = c.title.trim();
                    if !t.is_empty() && t != "Unknown" {
                        return t.to_string();
                    }
                } else if let Ok(grammers_tl_types::enums::User::User(u)) =
                    grammers_tl_types::enums::User::from_bytes(raw)
                {
                    let full = match (&u.first_name, &u.last_name) {
                        (Some(f), Some(l)) => format!("{f} {l}"),
                        (Some(f), None) => f.clone(),
                        (None, Some(l)) => l.clone(),
                        (None, None) => u.username.clone().unwrap_or_default(),
                    };
                    let trimmed = full.trim();
                    if !trimmed.is_empty() && trimmed != "Unknown" {
                        return trimmed.to_string();
                    }
                }
            }
            if let Some(uname) = &peer.username {
                let u = uname.trim();
                if !u.is_empty() {
                    return format!("@{u}");
                }
            }
        }

        if let Ok(Some(title)) = self.db.find_creation_or_title_change(peer_id) {
            let t = title.trim();
            if !t.is_empty() && t != "Unknown" {
                return t.to_string();
            }
        }

        format!("Chat {}", peer_id.raw())
    }

    pub fn resolve_reply(
        &self,
        source_key: MessageKey,
        target_key: MessageKey,
    ) -> RenderReplyPreview {
        let target_msg_opt = self.db.get_message(target_key).ok().flatten();

        let (sender_name, text_snippet, media_indicator, state) =
            if let Some(target) = &target_msg_opt {
                let sender = self.resolve_sender_name(target);

                let media_items = self
                    .db
                    .get_media_for_message(target_key.peer_id, target_key.message_id)
                    .ok()
                    .unwrap_or_default();

                let media_ind = media_items
                    .first()
                    .map(|m| media_kind_indicator(m.kind).to_string());

                let service_desc = if let Some(ref raw) = target.raw_tl {
                    if let Ok(grammers_tl_types::enums::Message::Service(s)) =
                        grammers_tl_types::enums::Message::from_bytes(raw)
                    {
                        match &s.action {
                            grammers_tl_types::enums::MessageAction::TopicCreate(t) => {
                                Some(format!("Created topic \"{}\"", t.title))
                            }
                            grammers_tl_types::enums::MessageAction::TopicEdit(t) => {
                                if let Some(title) = &t.title {
                                    Some(format!("Renamed topic to \"{title}\""))
                                } else if t.closed == Some(true) {
                                    Some("Closed topic".to_string())
                                } else if t.closed == Some(false) {
                                    Some("Reopened topic".to_string())
                                } else {
                                    Some("Edited topic".to_string())
                                }
                            }
                            grammers_tl_types::enums::MessageAction::SuggestProfilePhoto(_) => {
                                Some("Suggested profile photo".to_string())
                            }
                            grammers_tl_types::enums::MessageAction::ChatEditPhoto(_) => {
                                Some("Changed group photo".to_string())
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                let snippet = if target.state == MessageState::Deleted {
                    Some("[Deleted message]".to_string())
                } else if let Some(desc) = service_desc {
                    Some(desc)
                } else if let Some(t) = &target.text
                    && let Some(first_line) = t.lines().map(str::trim).find(|l| !l.is_empty())
                {
                    let truncated = if first_line.chars().count() > 100 {
                        first_line.chars().take(97).chain("...".chars()).collect()
                    } else {
                        first_line.to_string()
                    };
                    Some(truncated)
                } else if let Some(ind) = &media_ind {
                    Some(ind.clone())
                } else {
                    Some("[Attachment]".to_string())
                };

                (Some(sender), snippet, media_ind, target.state)
            } else {
                (
                    None,
                    Some("[Original message unavailable]".to_string()),
                    None,
                    MessageState::Empty,
                )
            };

        let target_url = self
            .location_map
            .get_location_full(&target_key)
            .map(|target_loc| {
                let anchor =
                    ArchiveUrlBuilder::message_anchor(target_key.peer_id, target_key.message_id);
                let source_loc = self.location_map.get_location_full(&source_key);
                let source_path = if let Some(sl) = source_loc {
                    if let Some(tid) = sl.topic_id {
                        format!("topics/{tid}/{}", sl.page_file)
                    } else {
                        sl.page_file.clone()
                    }
                } else {
                    "page_00001.html".to_string()
                };

                let target_path = if let Some(tid) = target_loc.topic_id {
                    format!("topics/{tid}/{}", target_loc.page_file)
                } else {
                    target_loc.page_file.clone()
                };

                let source_full_rel = format!(
                    "chats/{}/{}",
                    ArchiveUrlBuilder::peer_token(source_key.peer_id),
                    source_path
                );
                let target_full_rel = format!(
                    "chats/{}/{}",
                    ArchiveUrlBuilder::peer_token(target_key.peer_id),
                    target_path
                );

                let rel =
                    ArchiveUrlBuilder::relative_day_to_day(&source_full_rel, &target_full_rel);
                format!("{rel}#{anchor}")
            });

        RenderReplyPreview {
            target_key,
            sender_name,
            text_snippet,
            media_indicator,
            state,
            target_url,
        }
    }
}

pub fn render_reply_card(reply: &RenderReplyPreview) -> String {
    let sender = reply.sender_name.as_deref().unwrap_or("Replied message");
    let snippet = reply.text_snippet.as_deref().unwrap_or("[No text]");

    let state_badge = match reply.state {
        MessageState::Deleted => " <span class=\"reply-badge-deleted\">[Deleted]</span>",
        MessageState::Empty => " <span class=\"reply-badge-missing\">[Unavailable]</span>",
        MessageState::Inaccessible => {
            " <span class=\"reply-badge-inaccessible\">[Inaccessible]</span>"
        }
        _ => "",
    };

    let inner = format!(
        r#"  <div class="reply-accent-bar"></div>
  <div class="reply-content">
    <div class="reply-header"><span class="reply-sender">{}</span>{state_badge}</div>
    <div class="reply-body"><span class="reply-snippet">{}</span></div>
  </div>"#,
        html_escape(sender),
        html_escape(snippet)
    );

    if let Some(url) = &reply.target_url {
        format!(
            "<a href=\"{}\" class=\"msg-reply-preview reply-card\">\n{inner}\n</a>\n",
            html_escape(url)
        )
    } else {
        format!("<div class=\"msg-reply-preview reply-card reply-unlinked\">\n{inner}\n</div>\n")
    }
}
