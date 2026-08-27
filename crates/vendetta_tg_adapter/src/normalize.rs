use grammers_client::peer::{Channel, Dialog, Group, Peer, User};
use grammers_tl_types::{self as tl, Serializable};
use vendetta_core::now_unix_secs;
use vendetta_model::{
    MediaDownloadStatus, MediaKind, MediaRecord, MediaRole, MediaVerificationStatus, MessageId,
    MessageKey, MessageMediaJoin, MessageRecord, MessageState, PeerId, PeerRecord, PeerType,
};

pub fn normalize_peer_enum(peer: &tl::enums::Peer) -> PeerId {
    normalize_peer_and_type(peer).0
}

pub fn normalize_peer_and_type(peer: &tl::enums::Peer) -> (PeerId, PeerType) {
    match peer {
        tl::enums::Peer::User(u) => (PeerId::new(u.user_id), PeerType::User),
        tl::enums::Peer::Chat(c) => (PeerId::new(-c.chat_id), PeerType::Group),
        tl::enums::Peer::Channel(c) => (
            PeerId::new(-1_000_000_000_000 - c.channel_id),
            PeerType::Channel,
        ),
    }
}

pub fn normalize_user(user: &User) -> PeerRecord {
    let peer_id = PeerId::new(user.id().bare_id_unchecked());
    let raw_tl = user.raw.to_bytes();

    let name = match (user.first_name(), user.last_name()) {
        (Some(first), Some(last)) => Some(format!("{first} {last}")),
        (Some(first), None) => Some(first.to_string()),
        (None, Some(last)) => Some(last.to_string()),
        (None, None) => user.username().map(|u| u.to_string()),
    };

    PeerRecord {
        peer_id,
        peer_type: PeerType::User,
        name,
        username: user.username().map(|u| u.to_string()),
        phone: user.phone().map(|p| p.to_string()),
        raw_tl: Some(raw_tl),
        updated_at: now_unix_secs(),
    }
}

pub fn normalize_group(group: &Group) -> PeerRecord {
    let peer_id = PeerId::new(group.id().bot_api_dialog_id_unchecked());
    let raw_tl = group.raw.to_bytes();

    let (title, username) = match &group.raw {
        tl::enums::Chat::Chat(chat) => (Some(chat.title.clone()), None),
        tl::enums::Chat::Channel(chan) => (Some(chan.title.clone()), chan.username.clone()),
        tl::enums::Chat::Forbidden(f) => (Some(format!("[Inaccessible] {}", f.title)), None),
        tl::enums::Chat::ChannelForbidden(f) => (Some(format!("[Inaccessible] {}", f.title)), None),
        tl::enums::Chat::Community(comm) => (Some(comm.title.clone()), None),
        tl::enums::Chat::CommunityForbidden(f) => {
            (Some(format!("[Inaccessible] {}", f.title)), None)
        }
        tl::enums::Chat::Empty(_) => (Some("[Deleted Chat]".to_string()), None),
    };

    PeerRecord {
        peer_id,
        peer_type: PeerType::Group,
        name: title,
        username,
        phone: None,
        raw_tl: Some(raw_tl),
        updated_at: now_unix_secs(),
    }
}

pub fn normalize_channel(channel: &Channel) -> PeerRecord {
    let peer_id = PeerId::new(channel.id().bot_api_dialog_id_unchecked());
    let raw_tl = channel.raw.to_bytes();

    PeerRecord {
        peer_id,
        peer_type: PeerType::Channel,
        name: Some(channel.raw.title.clone()),
        username: channel.raw.username.clone(),
        phone: None,
        raw_tl: Some(raw_tl),
        updated_at: now_unix_secs(),
    }
}

pub fn normalize_peer(peer: &Peer) -> PeerRecord {
    match peer {
        Peer::User(u) => normalize_user(u),
        Peer::Group(g) => normalize_group(g),
        Peer::Channel(c) => normalize_channel(c),
        Peer::Community(comm) => {
            let peer_id = PeerId::new(comm.id().bot_api_dialog_id_unchecked());
            PeerRecord {
                peer_id,
                peer_type: PeerType::Group,
                name: Some(comm.title().to_string()),
                username: None,
                phone: None,
                raw_tl: Some(comm.raw.to_bytes()),
                updated_at: now_unix_secs(),
            }
        }
    }
}

pub fn normalize_dialog(dialog: &Dialog) -> PeerRecord {
    normalize_peer(&dialog.peer)
}

pub fn normalize_raw_user(user: &tl::enums::User) -> Option<PeerRecord> {
    match user {
        tl::enums::User::User(u) => {
            let peer_id = PeerId::new(u.id);
            let name = match (&u.first_name, &u.last_name) {
                (Some(first), Some(last)) => Some(format!("{first} {last}")),
                (Some(first), None) => Some(first.clone()),
                (None, Some(last)) => Some(last.clone()),
                (None, None) => u.username.clone(),
            };
            Some(PeerRecord {
                peer_id,
                peer_type: PeerType::User,
                name,
                username: u.username.clone(),
                phone: u.phone.clone(),
                raw_tl: Some(user.to_bytes()),
                updated_at: now_unix_secs(),
            })
        }
        tl::enums::User::Empty(e) => Some(PeerRecord {
            peer_id: PeerId::new(e.id),
            peer_type: PeerType::User,
            name: Some("[Deleted Account]".to_string()),
            username: None,
            phone: None,
            raw_tl: Some(user.to_bytes()),
            updated_at: now_unix_secs(),
        }),
    }
}

pub fn normalize_raw_chat(chat: &tl::enums::Chat) -> Option<PeerRecord> {
    match chat {
        tl::enums::Chat::Chat(c) => Some(PeerRecord {
            peer_id: PeerId::new(-c.id),
            peer_type: PeerType::Group,
            name: Some(c.title.clone()),
            username: None,
            phone: None,
            raw_tl: Some(chat.to_bytes()),
            updated_at: now_unix_secs(),
        }),
        tl::enums::Chat::Channel(c) => {
            let peer_type = if c.broadcast {
                PeerType::Channel
            } else {
                PeerType::Group
            };
            let peer_id = PeerId::new(-1_000_000_000_000 - c.id);
            Some(PeerRecord {
                peer_id,
                peer_type,
                name: Some(c.title.clone()),
                username: c.username.clone(),
                phone: None,
                raw_tl: Some(chat.to_bytes()),
                updated_at: now_unix_secs(),
            })
        }
        tl::enums::Chat::Forbidden(f) => Some(PeerRecord {
            peer_id: PeerId::new(-f.id),
            peer_type: PeerType::Group,
            name: Some(format!("[Inaccessible] {}", f.title)),
            username: None,
            phone: None,
            raw_tl: Some(chat.to_bytes()),
            updated_at: now_unix_secs(),
        }),
        tl::enums::Chat::ChannelForbidden(f) => {
            let peer_type = if f.broadcast {
                PeerType::Channel
            } else {
                PeerType::Group
            };
            let peer_id = PeerId::new(-1_000_000_000_000 - f.id);
            Some(PeerRecord {
                peer_id,
                peer_type,
                name: Some(format!("[Inaccessible] {}", f.title)),
                username: None,
                phone: None,
                raw_tl: Some(chat.to_bytes()),
                updated_at: now_unix_secs(),
            })
        }
        tl::enums::Chat::Community(c) => {
            let peer_id = PeerId::new(-1_000_000_000_000 - c.id);
            Some(PeerRecord {
                peer_id,
                peer_type: PeerType::Group,
                name: Some(c.title.clone()),
                username: None,
                phone: None,
                raw_tl: Some(chat.to_bytes()),
                updated_at: now_unix_secs(),
            })
        }
        tl::enums::Chat::CommunityForbidden(f) => {
            let peer_id = PeerId::new(-1_000_000_000_000 - f.id);
            Some(PeerRecord {
                peer_id,
                peer_type: PeerType::Group,
                name: Some(format!("[Inaccessible] {}", f.title)),
                username: None,
                phone: None,
                raw_tl: Some(chat.to_bytes()),
                updated_at: now_unix_secs(),
            })
        }
        tl::enums::Chat::Empty(e) => Some(PeerRecord {
            peer_id: PeerId::new(-e.id),
            peer_type: PeerType::Group,
            name: Some("[Deleted Chat]".to_string()),
            username: None,
            phone: None,
            raw_tl: Some(chat.to_bytes()),
            updated_at: now_unix_secs(),
        }),
    }
}

fn format_action_text(action: &tl::enums::MessageAction) -> String {
    match action {
        tl::enums::MessageAction::ChatCreate(c) => format!("Created group: \"{}\"", c.title),
        tl::enums::MessageAction::ChatEditTitle(c) => {
            format!("Changed group title to: \"{}\"", c.title)
        }
        tl::enums::MessageAction::ChatEditPhoto(_) => "Changed group photo".to_string(),
        tl::enums::MessageAction::ChatDeletePhoto => "Removed group photo".to_string(),
        tl::enums::MessageAction::ChatAddUser(c) => {
            format!("Added {} user(s)", c.users.len())
        }
        tl::enums::MessageAction::ChatDeleteUser(c) => format!("Removed user {}", c.user_id),
        tl::enums::MessageAction::ChatJoinedByLink(c) => {
            format!("User joined via invite link (inviter: {})", c.inviter_id)
        }
        tl::enums::MessageAction::ChannelCreate(c) => format!("Created channel: \"{}\"", c.title),
        tl::enums::MessageAction::PinMessage => "Pinned a message".to_string(),
        tl::enums::MessageAction::HistoryClear => "Cleared message history".to_string(),
        tl::enums::MessageAction::ContactSignUp => "Joined Telegram".to_string(),
        tl::enums::MessageAction::ChatMigrateTo(c) => {
            format!("Group upgraded to supergroup {}", c.channel_id)
        }
        tl::enums::MessageAction::ChannelMigrateFrom(c) => {
            format!("Supergroup migrated from basic chat {}", c.chat_id)
        }
        tl::enums::MessageAction::Empty => "Service notification".to_string(),
        _ => "Service action".to_string(),
    }
}

fn extract_reply_metadata(
    reply_header: Option<&tl::enums::MessageReplyHeader>,
) -> (Option<MessageId>, Option<MessageId>, Option<PeerId>) {
    match reply_header {
        Some(tl::enums::MessageReplyHeader::Header(h)) => (
            h.reply_to_msg_id.map(|id| MessageId::new(id as i64)),
            h.reply_to_top_id.map(|id| MessageId::new(id as i64)),
            h.reply_to_peer_id.as_ref().map(normalize_peer_enum),
        ),
        _ => (None, None, None),
    }
}

pub fn normalize_message(msg: &tl::enums::Message, fallback_peer: Option<PeerId>) -> MessageRecord {
    let raw_tl = Some(msg.to_bytes());

    match msg {
        tl::enums::Message::Message(m) => {
            let peer_id = normalize_peer_enum(&m.peer_id);
            let message_id = MessageId::new(m.id as i64);
            let sender_id = m.from_id.as_ref().map(normalize_peer_enum);

            let (reply_to_msg_id, reply_to_top_id, reply_to_peer_id) =
                extract_reply_metadata(m.reply_to.as_ref());

            let entities_json = m
                .entities
                .as_ref()
                .and_then(|e| serde_json::to_string(e).ok());
            let forward_json = m
                .fwd_from
                .as_ref()
                .and_then(|f| serde_json::to_string(f).ok());
            let reactions_json = m
                .reactions
                .as_ref()
                .and_then(|r| serde_json::to_string(r).ok());

            let state = if m.edit_date.is_some() {
                MessageState::Edited
            } else {
                MessageState::Active
            };

            MessageRecord {
                key: MessageKey::new(peer_id, message_id),
                date: m.date as i64,
                sender_id,
                text: if m.message.is_empty() {
                    None
                } else {
                    Some(m.message.clone())
                },
                entities_json,
                edit_date: m.edit_date.map(|d| d as i64),
                state,
                reply_to_msg_id,
                reply_to_top_id,
                reply_to_peer_id,
                grouped_id: m.grouped_id,
                forward_json,
                reactions_json,
                views: m.views,
                forwards_count: m.forwards,
                raw_tl,
            }
        }
        tl::enums::Message::Service(s) => {
            let peer_id = normalize_peer_enum(&s.peer_id);
            let message_id = MessageId::new(s.id as i64);
            let sender_id = s.from_id.as_ref().map(normalize_peer_enum);

            let (reply_to_msg_id, reply_to_top_id, reply_to_peer_id) =
                extract_reply_metadata(s.reply_to.as_ref());

            let reactions_json = s
                .reactions
                .as_ref()
                .and_then(|r| serde_json::to_string(r).ok());

            let action_text = format_action_text(&s.action);

            MessageRecord {
                key: MessageKey::new(peer_id, message_id),
                date: s.date as i64,
                sender_id,
                text: Some(action_text),
                entities_json: None,
                edit_date: None,
                state: MessageState::Active,
                reply_to_msg_id,
                reply_to_top_id,
                reply_to_peer_id,
                grouped_id: None,
                forward_json: None,
                reactions_json,
                views: None,
                forwards_count: None,
                raw_tl,
            }
        }
        tl::enums::Message::Empty(e) => {
            let peer_id = e
                .peer_id
                .as_ref()
                .map(normalize_peer_enum)
                .or(fallback_peer)
                .unwrap_or(PeerId::new(0));
            let message_id = MessageId::new(e.id as i64);

            MessageRecord {
                key: MessageKey::new(peer_id, message_id),
                date: 0,
                sender_id: None,
                text: Some("[Empty / Unavailable Message]".to_string()),
                entities_json: None,
                edit_date: None,
                state: MessageState::Empty,
                reply_to_msg_id: None,
                reply_to_top_id: None,
                reply_to_peer_id: None,
                grouped_id: None,
                forward_json: None,
                reactions_json: None,
                views: None,
                forwards_count: None,
                raw_tl,
            }
        }
    }
}

pub fn normalize_update(update: &tl::enums::Update) -> vendetta_model::NormalizedUpdate {
    let raw_tl = update.to_bytes();
    match update {
        tl::enums::Update::NewMessage(u) => {
            let message = normalize_message(&u.message, None);
            vendetta_model::NormalizedUpdate::NewMessage {
                message,
                pts: Some(u.pts),
                pts_count: u.pts_count,
            }
        }
        tl::enums::Update::EditMessage(u) => {
            let message = normalize_message(&u.message, None);
            vendetta_model::NormalizedUpdate::EditedMessage {
                message,
                pts: Some(u.pts),
                pts_count: u.pts_count,
            }
        }
        tl::enums::Update::DeleteMessages(u) => {
            let message_ids = u
                .messages
                .iter()
                .map(|&id| MessageId::new(id as i64))
                .collect();
            vendetta_model::NormalizedUpdate::CommonDeletedMessages {
                message_ids,
                pts: Some(u.pts),
                pts_count: u.pts_count,
            }
        }
        tl::enums::Update::NewChannelMessage(u) => {
            let message = normalize_message(&u.message, None);
            vendetta_model::NormalizedUpdate::NewMessage {
                message,
                pts: Some(u.pts),
                pts_count: u.pts_count,
            }
        }
        tl::enums::Update::EditChannelMessage(u) => {
            let message = normalize_message(&u.message, None);
            vendetta_model::NormalizedUpdate::EditedMessage {
                message,
                pts: Some(u.pts),
                pts_count: u.pts_count,
            }
        }
        tl::enums::Update::DeleteChannelMessages(u) => {
            let channel_id = PeerId::new(-1_000_000_000_000 - u.channel_id);
            let message_ids = u
                .messages
                .iter()
                .map(|&id| MessageId::new(id as i64))
                .collect();
            vendetta_model::NormalizedUpdate::ChannelDeletedMessages {
                channel_id,
                message_ids,
                pts: Some(u.pts),
                pts_count: u.pts_count,
            }
        }
        tl::enums::Update::ChannelTooLong(u) => {
            let channel_id = PeerId::new(-1_000_000_000_000 - u.channel_id);
            vendetta_model::NormalizedUpdate::ChannelCatchupRequired {
                channel_id,
                pts: u.pts,
            }
        }
        tl::enums::Update::Channel(u) => {
            let channel_id = PeerId::new(-1_000_000_000_000 - u.channel_id);
            vendetta_model::NormalizedUpdate::ChannelCatchupRequired {
                channel_id,
                pts: None,
            }
        }
        tl::enums::Update::ReadChannelInbox(u) => {
            let channel_id = PeerId::new(-1_000_000_000_000 - u.channel_id);
            vendetta_model::NormalizedUpdate::ChannelCatchupRequired {
                channel_id,
                pts: Some(u.pts),
            }
        }
        tl::enums::Update::ReadChannelOutbox(u) => {
            let channel_id = PeerId::new(-1_000_000_000_000 - u.channel_id);
            vendetta_model::NormalizedUpdate::ChannelCatchupRequired {
                channel_id,
                pts: None,
            }
        }
        other => {
            let constructor_id = match other {
                tl::enums::Update::NewEncryptedMessage(_) => 0x12bcbd9a,
                tl::enums::Update::ReadHistoryInbox(_) => 0x9961fd5c,
                tl::enums::Update::ReadHistoryOutbox(_) => 0x2f2f21bf,
                _ => 0x0,
            };

            let affects_sync_state = match other {
                tl::enums::Update::NewEncryptedMessage(e) => e.qts > 0,
                _ => false,
            };

            let pts = match other {
                tl::enums::Update::ReadHistoryInbox(r) => Some(r.pts),
                tl::enums::Update::ReadHistoryOutbox(r) => Some(r.pts),
                _ => None,
            };

            let pts_count = match other {
                tl::enums::Update::ReadHistoryInbox(r) => r.pts_count,
                tl::enums::Update::ReadHistoryOutbox(r) => r.pts_count,
                _ => 0,
            };

            let (qts, qts_count) = match other {
                tl::enums::Update::NewEncryptedMessage(e) => (Some(e.qts), 1),
                _ => (None, 0),
            };

            vendetta_model::NormalizedUpdate::Unsupported {
                constructor_name: format!("{other:?}"),
                constructor_id,
                affects_sync_state,
                pts,
                pts_count,
                qts,
                qts_count,
                diagnostic_info: Some(format!("Unsupported TL update {other:?}")),
                raw_tl,
            }
        }
    }
}

fn extract_photo_record(
    photo_enum: &tl::enums::Photo,
    msg_key: MessageKey,
    role: MediaRole,
    position: i32,
    now: i64,
) -> Option<(MediaRecord, MessageMediaJoin)> {
    let tl::enums::Photo::Photo(photo) = photo_enum else {
        return None;
    };
    if photo.id == 0 {
        return None;
    }

    struct PhotoCandidate<'a> {
        size_type: &'a str,
        score: i64,
        w: Option<i32>,
        h: Option<i32>,
        size_bytes: i64,
    }

    let mut best_size: Option<PhotoCandidate> = None;

    for sz in &photo.sizes {
        match sz {
            tl::enums::PhotoSize::Size(s) => {
                let score = (s.w as i64) * (s.h as i64);
                if best_size.as_ref().is_none_or(|b| {
                    score > b.score || (score == b.score && s.size as i64 > b.size_bytes)
                }) {
                    best_size = Some(PhotoCandidate {
                        size_type: &s.r#type,
                        score,
                        w: Some(s.w),
                        h: Some(s.h),
                        size_bytes: s.size as i64,
                    });
                }
            }
            tl::enums::PhotoSize::Progressive(p) => {
                let score = (p.w as i64) * (p.h as i64);
                let size = p.sizes.last().copied().unwrap_or(0) as i64;
                if best_size
                    .as_ref()
                    .is_none_or(|b| score > b.score || (score == b.score && size > b.size_bytes))
                {
                    best_size = Some(PhotoCandidate {
                        size_type: &p.r#type,
                        score,
                        w: Some(p.w),
                        h: Some(p.h),
                        size_bytes: size,
                    });
                }
            }
            _ => {}
        }
    }

    let candidate = best_size?;
    let size_type = candidate.size_type;
    let width = candidate.w;
    let height = candidate.h;
    let size_bytes = candidate.size_bytes;

    let media_id = format!("photo_{}_{}", photo.id, size_type);
    let location =
        tl::enums::InputFileLocation::InputPhotoFileLocation(tl::types::InputPhotoFileLocation {
            id: photo.id,
            access_hash: photo.access_hash,
            file_reference: photo.file_reference.clone(),
            thumb_size: size_type.to_string(),
        });

    let record = MediaRecord {
        media_id: media_id.clone(),
        kind: MediaKind::Photo,
        mime_type: Some("image/jpeg".to_string()),
        size_bytes: if size_bytes > 0 {
            Some(size_bytes)
        } else {
            None
        },
        file_name: Some(format!("{media_id}.jpg")),
        size_type: Some(size_type.to_string()),
        width,
        height,
        dc_id: photo.dc_id,
        source_location_tl: Some(location.to_bytes()),
        file_reference: Some(photo.file_reference.clone()),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: None,
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: now,
        updated_at: now,
    };

    let join = MessageMediaJoin {
        key: msg_key,
        media_id,
        role,
        position,
    };

    Some((record, join))
}

fn extract_document_record(
    doc_enum: &tl::enums::Document,
    msg_key: MessageKey,
    default_role: Option<MediaRole>,
    position: i32,
    now: i64,
) -> Option<(MediaRecord, MessageMediaJoin)> {
    let tl::enums::Document::Document(doc) = doc_enum else {
        return None;
    };
    if doc.id == 0 {
        return None;
    }

    let mut kind = MediaKind::Document;
    let mut file_name = None;
    let mut width = None;
    let mut height = None;

    for attr in &doc.attributes {
        match attr {
            tl::enums::DocumentAttribute::Filename(f) => {
                file_name = Some(f.file_name.clone());
            }
            tl::enums::DocumentAttribute::Video(v) => {
                if v.round_message {
                    kind = MediaKind::VideoNote;
                } else {
                    kind = MediaKind::Video;
                }
                width = Some(v.w);
                height = Some(v.h);
            }
            tl::enums::DocumentAttribute::Audio(a) => {
                if a.voice {
                    kind = MediaKind::Voice;
                } else {
                    kind = MediaKind::Audio;
                }
            }
            tl::enums::DocumentAttribute::Sticker(_) => {
                kind = MediaKind::Sticker;
            }
            tl::enums::DocumentAttribute::Animated => {
                if kind == MediaKind::Document {
                    kind = MediaKind::Animation;
                }
            }
            tl::enums::DocumentAttribute::ImageSize(s) => {
                width = Some(s.w);
                height = Some(s.h);
            }
            _ => {}
        }
    }

    let media_id = format!("doc_{}", doc.id);
    let location = tl::enums::InputFileLocation::InputDocumentFileLocation(
        tl::types::InputDocumentFileLocation {
            id: doc.id,
            access_hash: doc.access_hash,
            file_reference: doc.file_reference.clone(),
            thumb_size: String::new(),
        },
    );

    let record = MediaRecord {
        media_id: media_id.clone(),
        kind,
        mime_type: Some(doc.mime_type.clone()),
        size_bytes: Some(doc.size),
        file_name,
        size_type: None,
        width,
        height,
        dc_id: doc.dc_id,
        source_location_tl: Some(location.to_bytes()),
        file_reference: Some(doc.file_reference.clone()),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: None,
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: now,
        updated_at: now,
    };

    let role = default_role.unwrap_or(match kind {
        MediaKind::Voice => MediaRole::Voice,
        MediaKind::VideoNote => MediaRole::VideoNote,
        MediaKind::Sticker => MediaRole::Sticker,
        _ => MediaRole::Attachment,
    });

    let join = MessageMediaJoin {
        key: msg_key,
        media_id,
        role,
        position,
    };

    Some((record, join))
}

pub fn extract_media_records(
    msg: &tl::enums::Message,
    fallback_peer_id: Option<PeerId>,
) -> Vec<(MediaRecord, MessageMediaJoin)> {
    let mut results = Vec::new();

    let m = match msg {
        tl::enums::Message::Message(m) => m,
        _ => return results,
    };

    let peer_id = fallback_peer_id.unwrap_or_else(|| normalize_peer_enum(&m.peer_id));
    let msg_key = MessageKey::new(peer_id, MessageId::new(m.id as i64));

    let media = match &m.media {
        Some(med) => med,
        None => return results,
    };

    let now = now_unix_secs();
    let mut position = 0;

    match media {
        tl::enums::MessageMedia::Photo(photo_media) => {
            if let Some(photo) = &photo_media.photo
                && let Some(item) =
                    extract_photo_record(photo, msg_key, MediaRole::Attachment, position, now)
            {
                results.push(item);
                position += 1;
            }
            if let Some(video) = &photo_media.video
                && let Some(item) = extract_document_record(
                    video,
                    msg_key,
                    Some(MediaRole::AlternativeQuality),
                    position,
                    now,
                )
            {
                results.push(item);
            }
        }
        tl::enums::MessageMedia::Document(doc_media) => {
            if let Some(doc) = &doc_media.document
                && let Some(item) = extract_document_record(doc, msg_key, None, position, now)
            {
                results.push(item);
                position += 1;
            }
            if let Some(alt_docs) = &doc_media.alt_documents {
                for alt_doc in alt_docs {
                    let role = if let tl::enums::Document::Document(d) = alt_doc {
                        if d.mime_type.contains("mpegurl") {
                            MediaRole::StreamingManifest
                        } else if d.mime_type.contains("tgstoryboard") {
                            MediaRole::Storyboard
                        } else {
                            MediaRole::AlternativeQuality
                        }
                    } else {
                        MediaRole::AlternativeQuality
                    };

                    if let Some(item) =
                        extract_document_record(alt_doc, msg_key, Some(role), position, now)
                    {
                        results.push(item);
                        position += 1;
                    }
                }
            }
            if let Some(video_cover) = &doc_media.video_cover
                && let Some(item) =
                    extract_photo_record(video_cover, msg_key, MediaRole::Thumbnail, position, now)
            {
                results.push(item);
            }
        }
        tl::enums::MessageMedia::WebPage(wp) => {
            if let tl::enums::WebPage::Page(page) = &wp.webpage {
                if let Some(photo) = &page.photo
                    && let Some(item) =
                        extract_photo_record(photo, msg_key, MediaRole::Attachment, position, now)
                {
                    results.push(item);
                    position += 1;
                }
                if let Some(doc) = &page.document
                    && let Some(item) = extract_document_record(
                        doc,
                        msg_key,
                        Some(MediaRole::Attachment),
                        position,
                        now,
                    )
                {
                    results.push(item);
                }
            }
        }
        tl::enums::MessageMedia::Game(game_media) => {
            let tl::enums::Game::Game(game) = &game_media.game;
            if let Some(item) =
                extract_photo_record(&game.photo, msg_key, MediaRole::Thumbnail, position, now)
            {
                results.push(item);
                position += 1;
            }
            if let Some(doc) = &game.document
                && let Some(item) = extract_document_record(
                    doc,
                    msg_key,
                    Some(MediaRole::Attachment),
                    position,
                    now,
                )
            {
                results.push(item);
            }
        }
        tl::enums::MessageMedia::PaidMedia(paid_media) => {
            for ext in &paid_media.extended_media {
                if let tl::enums::MessageExtendedMedia::Media(em) = ext {
                    match &em.media {
                        tl::enums::MessageMedia::Photo(p) => {
                            if let Some(photo) = &p.photo
                                && let Some(item) = extract_photo_record(
                                    photo,
                                    msg_key,
                                    MediaRole::Attachment,
                                    position,
                                    now,
                                )
                            {
                                results.push(item);
                                position += 1;
                            }
                        }
                        tl::enums::MessageMedia::Document(d) => {
                            if let Some(doc) = &d.document
                                && let Some(item) =
                                    extract_document_record(doc, msg_key, None, position, now)
                            {
                                results.push(item);
                                position += 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        tl::enums::MessageMedia::Story(story_media) => {
            if let Some(tl::enums::StoryItem::Item(story_item)) = &story_media.story {
                match &story_item.media {
                    tl::enums::MessageMedia::Photo(p) => {
                        if let Some(photo) = &p.photo
                            && let Some(item) = extract_photo_record(
                                photo,
                                msg_key,
                                MediaRole::Attachment,
                                position,
                                now,
                            )
                        {
                            results.push(item);
                        }
                    }
                    tl::enums::MessageMedia::Document(d) => {
                        if let Some(doc) = &d.document
                            && let Some(item) =
                                extract_document_record(doc, msg_key, None, position, now)
                        {
                            results.push(item);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    results
}
