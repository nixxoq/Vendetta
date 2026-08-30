use std::{collections::BTreeMap, path::Path};

use grammers_tl_types::{self as tl, Deserializable};
use vendetta_model::{MessageRecord, TopicInfo};

use crate::{model::RenderTopic, url_builder::ArchiveUrlBuilder};

pub fn discover_topics(messages: &[MessageRecord]) -> BTreeMap<i32, TopicInfo> {
    let mut discovered = BTreeMap::from([(
        1,
        TopicInfo {
            topic_id: 1,
            title: "General".to_string(),
            icon_color: None,
            icon_emoji_id: None,
            is_general: true,
            is_closed: false,
            is_pinned: false,
            is_hidden: false,
        },
    )]);

    for msg in messages {
        if let Some(ref raw) = msg.raw_tl
            && let Ok(tl::enums::Message::Service(s)) = tl::enums::Message::from_bytes(raw)
        {
            match &s.action {
                tl::enums::MessageAction::TopicCreate(tc) => {
                    discovered.insert(
                        msg.key.message_id.raw() as i32,
                        TopicInfo {
                            topic_id: msg.key.message_id.raw() as i32,
                            title: tc.title.clone(),
                            icon_color: Some(tc.icon_color),
                            icon_emoji_id: tc.icon_emoji_id,
                            is_general: false,
                            is_closed: false,
                            is_pinned: false,
                            is_hidden: false,
                        },
                    );
                }
                tl::enums::MessageAction::TopicEdit(te) => {
                    let target_tid =
                        msg.reply_to_top_id
                            .map(|t| t.raw() as i32)
                            .unwrap_or_else(|| {
                                if let Some(tl::enums::MessageReplyHeader::Header(h)) = &s.reply_to
                                {
                                    h.reply_to_top_id.or(h.reply_to_msg_id).unwrap_or(1)
                                } else {
                                    1
                                }
                            });

                    if let Some(entry) = discovered.get_mut(&target_tid) {
                        if let Some(ref title) = te.title {
                            entry.title = title.clone();
                        }
                        if let Some(closed) = te.closed {
                            entry.is_closed = closed;
                        }
                        if let Some(hidden) = te.hidden {
                            entry.is_hidden = hidden;
                        }
                        if let Some(icon) = te.icon_emoji_id {
                            entry.icon_emoji_id = Some(icon);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    discovered
}

pub fn resolve_message_topic_id(
    msg: &MessageRecord,
    discovered_topics: &BTreeMap<i32, TopicInfo>,
) -> i32 {
    let Some(ref raw) = msg.raw_tl else {
        return 1;
    };
    let Ok(tl_msg) = tl::enums::Message::from_bytes(raw) else {
        return 1;
    };

    let reply_header = match tl_msg {
        tl::enums::Message::Service(ref s) => {
            if matches!(s.action, tl::enums::MessageAction::TopicCreate(_)) {
                return msg.key.message_id.raw() as i32;
            }
            s.reply_to.as_ref()
        }
        tl::enums::Message::Message(ref m) => m.reply_to.as_ref(),
        tl::enums::Message::Empty(_) => return 1,
    };

    if let Some(top_id) = msg.reply_to_top_id {
        return top_id.raw() as i32;
    }

    if let Some(tl::enums::MessageReplyHeader::Header(h)) = reply_header {
        if let Some(top_id) = h.reply_to_top_id {
            return top_id;
        }
        if h.forum_topic
            && let Some(root_id) = h.reply_to_msg_id
            && discovered_topics.contains_key(&root_id)
        {
            return root_id;
        }
    }

    1
}

pub fn resolve_topic_icon_asset(doc_id: i64, media_src_dir: Option<&Path>) -> Option<String> {
    let src_base = media_src_dir?;

    for sub in ["reactions", "icons"] {
        for ext in ["webp", "png"] {
            let file_name = format!("{doc_id}.{ext}");
            let paths = [
                src_base.join(sub).join(&file_name),
                src_base.join("media").join(sub).join(&file_name),
            ];
            if paths.into_iter().any(|p| p.is_file()) {
                let export_rel = format!("media/{sub}/{file_name}");
                return Some(ArchiveUrlBuilder::media_url(2, &export_rel));
            }
        }
    }

    None
}

pub fn build_render_topics(
    discovered_topics: &BTreeMap<i32, TopicInfo>,
    topic_messages: &BTreeMap<i32, Vec<MessageRecord>>,
    media_src_dir: Option<&Path>,
) -> Vec<RenderTopic> {
    let mut peer_topics: Vec<_> = discovered_topics
        .iter()
        .map(|(&tid, meta)| {
            let msgs_slice = topic_messages
                .get(&tid)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let icon_asset = meta
                .icon_emoji_id
                .and_then(|doc_id| resolve_topic_icon_asset(doc_id, media_src_dir));

            RenderTopic {
                topic_id: tid,
                title: meta.title.clone(),
                icon_color: meta.icon_color,
                icon_emoji_id: meta.icon_emoji_id,
                icon_asset,
                total_messages: msgs_slice.len(),
                last_message_date: msgs_slice.last().map(|m| m.date),
                is_general: tid == 1,
                is_closed: meta.is_closed,
                is_pinned: meta.is_pinned,
                is_hidden: meta.is_hidden,
            }
        })
        .collect();

    peer_topics.sort_by(|a, b| {
        b.is_pinned
            .cmp(&a.is_pinned)
            .then_with(|| (b.topic_id == 1).cmp(&(a.topic_id == 1)))
            .then_with(|| {
                b.last_message_date
                    .unwrap_or(0)
                    .cmp(&a.last_message_date.unwrap_or(0))
            })
            .then_with(|| a.topic_id.cmp(&b.topic_id))
    });

    peer_topics
}
