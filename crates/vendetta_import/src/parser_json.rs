use std::{fs, path::Path};

use serde_json::Value;
use tracing::debug;
use vendetta_model::{MediaKind, MediaRole, MessageId, PeerId, PeerType};

use crate::{
    discovery::TDesktopChatSource,
    error::{ImportError, ImportResult},
    model::{
        ImportChat, ImportEntityKind, ImportForwardInfo, ImportMediaItem, ImportMessage,
        ImportReaction, ImportServiceEvent, ImportTextEntity,
    },
};

pub fn parse_tdesktop_json(source: &TDesktopChatSource) -> ImportResult<Vec<ImportChat>> {
    let entry_file = source.files.first().ok_or_else(|| {
        ImportError::ParsingFailed(format!(
            "No JSON entry file in {}",
            source.basedir.display()
        ))
    })?;

    let content = fs::read_to_string(entry_file)?;
    let root: Value = serde_json::from_str(&content)?;

    if let Some(chats_list) = root
        .get("chats")
        .and_then(|c| c.get("list"))
        .and_then(Value::as_array)
    {
        chats_list
            .iter()
            .map(|chat_val| parse_single_chat_json(chat_val, &source.basedir))
            .collect()
    } else {
        parse_single_chat_json(&root, &source.basedir).map(|chat| vec![chat])
    }
}

fn parse_single_chat_json(val: &Value, base_dir: &Path) -> ImportResult<ImportChat> {
    let raw_id = val.get("id").and_then(Value::as_i64).unwrap_or(1);
    let raw_type = val.get("type").and_then(Value::as_str).unwrap_or_default();
    let name = val
        .get("name")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    let (peer_id, peer_type) = map_json_peer_id(raw_id, raw_type);

    let messages: Vec<ImportMessage> = val
        .get("messages")
        .and_then(Value::as_array)
        .map(|msg_arr| {
            msg_arr
                .iter()
                .filter_map(|m_val| parse_single_message_json(m_val, base_dir))
                .collect()
        })
        .unwrap_or_default();

    debug!(
        "Parsed JSON chat '{}' ({}) with {} messages",
        name.as_deref().unwrap_or("unnamed"),
        peer_id,
        messages.len()
    );

    Ok(ImportChat {
        peer_id,
        peer_type,
        name,
        username: None,
        messages,
    })
}

fn parse_single_message_json(m_val: &Value, base_dir: &Path) -> Option<ImportMessage> {
    let id = m_val.get("id").and_then(Value::as_i64).unwrap_or(0);
    if id <= 0 {
        return None;
    }
    let message_id = MessageId::new(id);

    let date = parse_json_unix_timestamp(m_val, "date_unixtime")
        .unwrap_or_else(vendetta_core::now_unix_secs);
    let edit_date = parse_json_unix_timestamp(m_val, "edited_unixtime");

    let sender_name = m_val
        .get("from")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    let sender_id = m_val
        .get("from_id")
        .and_then(Value::as_str)
        .and_then(parse_from_id);

    let reply_to_message_id = m_val
        .get("reply_to_message_id")
        .and_then(Value::as_i64)
        .map(MessageId::new);

    let forward_info = m_val
        .get("forwarded_from")
        .and_then(Value::as_str)
        .map(|f| ImportForwardInfo {
            from_name: Some(f.to_string()),
            date: None,
        });

    let (text, entities) = parse_json_text(m_val);
    let reactions = parse_json_reactions(m_val);
    let media = parse_json_media(m_val, base_dir);
    let service_event = parse_json_service_event(m_val, text.as_deref());

    Some(ImportMessage {
        message_id,
        date,
        sender_id,
        sender_name,
        text,
        entities,
        edit_date,
        reply_to_message_id,
        forward_info,
        reactions,
        media,
        service_event,
        is_joined_continuation: false,
        is_outgoing: false,
    })
}

fn parse_json_service_event(m_val: &Value, text: Option<&str>) -> Option<ImportServiceEvent> {
    let msg_type = m_val
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message");
    if msg_type != "service" {
        return None;
    }

    let action = m_val
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some(match action {
        "pin_message" => ImportServiceEvent::PinMessage,
        "edit_chat_title" => {
            let title = m_val
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default();
            ImportServiceEvent::EditTitle(title.to_string())
        }
        "edit_chat_photo" => ImportServiceEvent::EditPhoto,
        "delete_chat_photo" => ImportServiceEvent::DeletePhoto,
        other => {
            let txt = text.unwrap_or(other);
            ImportServiceEvent::ActionText(txt.to_string())
        }
    })
}

fn map_json_peer_id(id: i64, peer_type_str: &str) -> (PeerId, PeerType) {
    match peer_type_str {
        "saved_messages" | "personal_chat" => (PeerId::new(id), PeerType::User),
        "private_group" | "public_group" => (PeerId::new(-id.abs()), PeerType::Group),
        "private_channel" | "public_channel" | "public_supergroup" => (
            PeerId::new(-1_000_000_000_000 - id.abs()),
            PeerType::Channel,
        ),
        _ if id > 0 => (PeerId::new(id), PeerType::User),
        _ => (PeerId::new(id), PeerType::Group),
    }
}

fn parse_from_id(from_id_str: &str) -> Option<PeerId> {
    if let Some(user_id) = from_id_str.strip_prefix("user") {
        user_id.parse::<i64>().ok().map(PeerId::new)
    } else if let Some(channel_id) = from_id_str.strip_prefix("channel") {
        channel_id
            .parse::<i64>()
            .ok()
            .map(|cid| PeerId::new(-1_000_000_000_000 - cid))
    } else if let Some(chat_id) = from_id_str.strip_prefix("chat") {
        chat_id.parse::<i64>().ok().map(|cid| PeerId::new(-cid))
    } else {
        from_id_str.parse::<i64>().ok().map(PeerId::new)
    }
}
fn parse_json_text(val: &Value) -> (Option<String>, Vec<ImportTextEntity>) {
    if let Some(entities_arr) = val.get("text_entities").and_then(Value::as_array) {
        let (emitted_text, entities, _) = entities_arr.iter().fold(
            (String::new(), Vec::new(), 0usize),
            |(mut text, mut entities, mut offset), ent_val| {
                let chunk_text = ent_val
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let u16_len = chunk_text.encode_utf16().count();
                let start_u16 = offset;

                text.push_str(chunk_text);
                offset += u16_len;

                if u16_len > 0
                    && let Some(kind) = parse_entity_kind(ent_val)
                {
                    entities.push(ImportTextEntity {
                        kind,
                        offset_utf16: start_u16,
                        length_utf16: u16_len,
                    });
                }

                (text, entities, offset)
            },
        );

        let text_opt = (!emitted_text.is_empty()).then_some(emitted_text);
        (text_opt, entities)
    } else {
        val.get("text")
            .and_then(Value::as_str)
            .map(|s| (Some(s.to_string()), Vec::new()))
            .unwrap_or_default()
    }
}

fn parse_entity_kind(ent_val: &Value) -> Option<ImportEntityKind> {
    let ent_type = ent_val
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("plain");
    match ent_type {
        "bold" => Some(ImportEntityKind::Bold),
        "italic" => Some(ImportEntityKind::Italic),
        "underline" => Some(ImportEntityKind::Underline),
        "strikethrough" => Some(ImportEntityKind::Strike),
        "code" => Some(ImportEntityKind::Code),
        "pre" => {
            let lang = ent_val
                .get("language")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            Some(ImportEntityKind::Pre(lang))
        }
        "link" | "text_link" => {
            let href = ent_val
                .get("href")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(ImportEntityKind::TextUrl(href))
        }
        "spoiler" => Some(ImportEntityKind::Spoiler),
        "blockquote" => Some(ImportEntityKind::Blockquote),
        "mention" => Some(ImportEntityKind::Mention),
        "hashtag" => Some(ImportEntityKind::Hashtag),
        "custom_emoji" => {
            let doc_id = ent_val
                .get("document_id")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            Some(ImportEntityKind::CustomEmoji(doc_id))
        }
        _ => None,
    }
}

fn parse_json_unix_timestamp(val: &Value, field: &str) -> Option<i64> {
    val.get(field).and_then(|v| {
        v.as_str()
            .and_then(|s| s.parse::<i64>().ok())
            .or_else(|| v.as_i64())
    })
}

fn parse_json_reactions(val: &Value) -> Vec<ImportReaction> {
    val.get("reactions")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|r_val| {
                    let emoji = r_val
                        .get("emoji")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())?;
                    let count = r_val.get("count").and_then(Value::as_u64).unwrap_or(1) as usize;
                    Some(ImportReaction {
                        emoji: emoji.to_string(),
                        count,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_json_media(val: &Value, base_dir: &Path) -> Vec<ImportMediaItem> {
    let photo = val.get("photo").and_then(Value::as_str).map(|photo_path| {
        let path = base_dir.join(photo_path);
        let name = path
            .file_name()
            .and_then(|f| f.to_str())
            .map(ToString::to_string);

        ImportMediaItem {
            source_path: Some(path),
            file_name: name,
            kind: MediaKind::Photo,
            mime_type: Some("image/jpeg".to_string()),
            size_bytes: None,
            is_skipped: false,
            skip_reason: None,
            role: MediaRole::Attachment,
        }
    });

    let file = val.get("file").and_then(Value::as_str).map(|file_path| {
        let path = base_dir.join(file_path);
        let name = path
            .file_name()
            .and_then(|f| f.to_str())
            .map(ToString::to_string);
        let mime = val
            .get("mime_type")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let media_type = val
            .get("media_type")
            .and_then(Value::as_str)
            .unwrap_or("document");

        let (kind, role) = match media_type {
            "video_file" => (MediaKind::Video, MediaRole::Attachment),
            "voice_message" => (MediaKind::Voice, MediaRole::Voice),
            "video_message" => (MediaKind::VideoNote, MediaRole::VideoNote),
            "audio_file" => (MediaKind::Audio, MediaRole::Attachment),
            _ => (MediaKind::Document, MediaRole::Attachment),
        };

        ImportMediaItem {
            source_path: Some(path),
            file_name: name,
            kind,
            mime_type: mime,
            size_bytes: None,
            is_skipped: false,
            skip_reason: None,
            role,
        }
    });

    photo.into_iter().chain(file).collect()
}
