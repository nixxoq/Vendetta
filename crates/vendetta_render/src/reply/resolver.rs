use std::collections::HashMap;

use vendetta_model::{MediaKind, MessageKey, MessageState, PeerId};
use vendetta_storage::ArchiveDb;

use crate::{entity::html_escape, model::RenderReplyPreview, url_builder::ArchiveUrlBuilder};

#[derive(Debug, Clone, Default)]
pub struct ReplyLocationMap {
    locations: HashMap<MessageKey, usize>,
}

impl ReplyLocationMap {
    pub fn new() -> Self {
        Self {
            locations: HashMap::new(),
        }
    }

    pub fn insert(&mut self, key: MessageKey, page_index: usize) {
        self.locations.insert(key, page_index);
    }

    pub fn get_page(&self, key: &MessageKey) -> Option<usize> {
        self.locations.get(key).copied()
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

    pub fn resolve_reply(&self, source_peer: PeerId, target_key: MessageKey) -> RenderReplyPreview {
        let target_msg_opt = self.db.get_message(target_key).ok().flatten();

        let (sender_name, text_snippet, media_indicator, state) =
            if let Some(target) = &target_msg_opt {
                let sender = if let Some(sid) = target.sender_id {
                    self.db
                        .get_peer(sid)
                        .ok()
                        .flatten()
                        .and_then(|p| p.name)
                        .unwrap_or_else(|| format!("User {}", sid.raw()))
                } else {
                    "Unknown".to_string()
                };

                let media_items = self
                    .db
                    .get_media_for_message(target_key.peer_id, target_key.message_id)
                    .ok()
                    .unwrap_or_default();

                let media_ind = media_items
                    .first()
                    .map(|m| media_kind_indicator(m.kind).to_string());

                let snippet = if target.state == MessageState::Deleted {
                    Some("[Deleted message]".to_string())
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

        let target_url = self.location_map.get_page(&target_key).map(|page_idx| {
            ArchiveUrlBuilder::message_full_link(
                Some(source_peer),
                2,
                target_key.peer_id,
                page_idx,
                target_key.message_id,
            )
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
