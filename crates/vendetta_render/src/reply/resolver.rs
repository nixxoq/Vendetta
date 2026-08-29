use std::collections::HashMap;

use grammers_tl_types::Deserializable;
use vendetta_model::{MediaKind, MessageKey, MessageState, PeerId};
use vendetta_storage::ArchiveDb;

use crate::{entity::html_escape, model::RenderReplyPreview, url_builder::ArchiveUrlBuilder};

#[derive(Debug, Clone, Default)]
pub struct ReplyLocationMap {
    locations: HashMap<MessageKey, (usize, Option<i32>)>,
}

impl ReplyLocationMap {
    pub fn new() -> Self {
        Self {
            locations: HashMap::new(),
        }
    }

    pub fn insert(&mut self, key: MessageKey, page_index: usize, topic_id: Option<i32>) {
        self.locations.insert(key, (page_index, topic_id));
    }

    pub fn get_location(&self, key: &MessageKey) -> Option<(usize, Option<i32>)> {
        self.locations.get(key).copied()
    }

    pub fn get_page(&self, key: &MessageKey) -> Option<usize> {
        self.locations.get(key).map(|(p, _)| *p)
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

    pub fn resolve_reply(&self, source_peer: PeerId, target_key: MessageKey) -> RenderReplyPreview {
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
                    && !t.trim().is_empty()
                {
                    let first_line = t.lines().next().unwrap_or("").trim();
                    let truncated = if first_line.chars().count() > 100 {
                        let mut s: String = first_line.chars().take(97).collect();
                        s.push_str("...");
                        s
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

        let target_url = self.location_map.get_location(&target_key).map(|(page_idx, target_topic_id)| {
            let anchor = ArchiveUrlBuilder::message_anchor(target_key.peer_id, target_key.message_id);
            if target_key.peer_id == source_peer {
                let chunk_file = if let Some(tid) = target_topic_id {
                    ArchiveUrlBuilder::topic_page_file_name(tid, page_idx)
                } else {
                    ArchiveUrlBuilder::page_file_name(page_idx)
                };
                format!("{chunk_file}#{anchor}")
            } else {
                let target_chunk = if let Some(tid) = target_topic_id {
                    ArchiveUrlBuilder::topic_chunk_file_rel(target_key.peer_id, tid, page_idx)
                } else {
                    ArchiveUrlBuilder::chunk_file_rel(target_key.peer_id, page_idx)
                };
                let rel = ArchiveUrlBuilder::relative_url(2, &target_chunk);
                format!("{rel}#{anchor}")
            }
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
