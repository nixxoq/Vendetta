use std::{collections::HashSet, path::Path};

use grammers_tl_types::{self as tl, Deserializable};
use vendetta_model::{MessageKey, MessageRecord, PeerId, PeerType};
use vendetta_storage::ArchiveDb;

use crate::{
    entity::render_formatted_text,
    error::RenderResult,
    model::{
        RenderForwardInfo, RenderMediaItem, RenderMessage, RenderReactionGroup, RenderReactionKey,
        RenderReactor, RenderRevision,
    },
    reply::ReplyResolver,
    url_builder::ArchiveUrlBuilder,
};

pub struct MessageBuildContext<'a, F>
where
    F: Fn(PeerId) -> Option<String>,
{
    pub db: &'a ArchiveDb,
    pub reply_resolver: &'a ReplyResolver<'a>,
    pub available_avatars: &'a HashSet<PeerId>,
    pub exported_peer_ids: &'a HashSet<PeerId>,
    pub media_src_dir: Option<&'a Path>,
    pub include_edit_history: bool,
    pub authoritative_name_resolver: F,
}

pub fn build_render_message<F>(
    ctx: &MessageBuildContext<'_, F>,
    msg: &MessageRecord,
) -> RenderResult<RenderMessage>
where
    F: Fn(PeerId) -> Option<String>,
{
    let current_peer = ctx.db.get_peer(msg.key.peer_id).ok().flatten();
    let is_channel = current_peer
        .as_ref()
        .is_some_and(|p| p.peer_type == PeerType::Channel);

    let (sender_name, is_channel_post) = if let Some(sid) = msg.sender_id {
        let name =
            (ctx.authoritative_name_resolver)(sid).unwrap_or_else(|| format!("User {}", sid.raw()));
        (Some(name), false)
    } else if is_channel {
        let name = (ctx.authoritative_name_resolver)(msg.key.peer_id)
            .unwrap_or_else(|| format!("Channel {}", msg.key.peer_id.raw()));
        (Some(name), true)
    } else {
        (None, false)
    };

    let mut is_service = false;
    let mut service_description = None;
    let mut author_signature = None;
    let mut comments_count = None;
    let mut has_comments = false;

    if let Some(ref raw) = msg.raw_tl {
        if let Ok(tl::enums::Message::Service(s)) = tl::enums::Message::from_bytes(raw) {
            is_service = true;
            let formatted = format_service_action(&s.action);
            if formatted != "Service event" {
                service_description = Some(formatted);
            } else if let Some(t) = &msg.text
                && !t.trim().is_empty()
                && t != "Service action"
            {
                service_description = Some(t.clone());
            } else {
                service_description = Some(formatted);
            }
        } else if let Ok(tl::enums::Message::Message(m)) = tl::enums::Message::from_bytes(raw) {
            author_signature = m.post_author;
            if let Some(tl::enums::MessageReplies::Replies(r)) = m.replies {
                has_comments = r.comments;
                if r.replies > 0 {
                    comments_count = Some(r.replies);
                }
            }
        }
    }

    let formatted_html = msg
        .text
        .as_deref()
        .map(|t| render_formatted_text(t, msg.entities_json.as_deref()));

    let is_forum_topic_root = if let Some(ref raw) = msg.raw_tl
        && let Ok(tl::enums::Message::Message(m)) = tl::enums::Message::from_bytes(raw)
        && let Some(tl::enums::MessageReplyHeader::Header(h)) = &m.reply_to
    {
        h.forum_topic && h.reply_to_top_id.is_none()
    } else {
        false
    };

    let reply_preview = if !is_forum_topic_root && let Some(target_id) = msg.reply_to_msg_id {
        let target_peer = msg.reply_to_peer_id.unwrap_or(msg.key.peer_id);
        let target_key = MessageKey::new(target_peer, target_id);
        Some(
            ctx.reply_resolver
                .resolve_reply(msg.key.peer_id, target_key),
        )
    } else {
        None
    };

    let forward_info =
        resolve_forward_info(ctx.db, msg, ctx.available_avatars, ctx.exported_peer_ids);

    let mut revisions = Vec::new();
    if ctx.include_edit_history {
        let rev_records = ctx.db.list_message_revisions(msg.key)?;
        for rev in rev_records {
            let rev_html = rev
                .text
                .as_deref()
                .map(|t| render_formatted_text(t, rev.entities_json.as_deref()))
                .unwrap_or_default();

            revisions.push(RenderRevision {
                captured_at: rev.captured_at,
                edit_date: rev.edit_date,
                formatted_html: rev_html,
                raw_text: rev.text,
            });
        }
    }

    let mut media_items = Vec::new();
    let raw_media_joins = ctx
        .db
        .get_message_media_with_roles(msg.key.peer_id, msg.key.message_id)?;

    let mut seen_media_ids = HashSet::new();
    let mut has_primary_video = false;

    for (m_rec, role, position) in raw_media_joins {
        if !seen_media_ids.insert(m_rec.media_id.clone()) {
            continue;
        }

        if !role.is_primary_attachment() {
            continue;
        }

        if let Some(ref mime) = m_rec.mime_type
            && (mime.contains("mpegurl") || mime.contains("tgstoryboard"))
        {
            continue;
        }

        if m_rec.kind == vendetta_model::MediaKind::Video {
            if has_primary_video && position > 0 {
                continue;
            }
            has_primary_video = true;
        }

        let rel_url = m_rec
            .local_rel_path
            .as_deref()
            .map(|p| ArchiveUrlBuilder::media_url(2, p));

        let (is_available, unavailable_reason) = match m_rec.download_status {
            vendetta_model::MediaDownloadStatus::Completed => {
                if m_rec.verification_status
                    == vendetta_model::MediaVerificationStatus::CorruptedHash
                    || m_rec.verification_status
                        == vendetta_model::MediaVerificationStatus::CorruptedSize
                {
                    (false, Some("File corrupted on disk".to_string()))
                } else if m_rec.verification_status
                    == vendetta_model::MediaVerificationStatus::MissingFile
                {
                    (false, Some("File missing on disk".to_string()))
                } else if m_rec.local_rel_path.is_some() {
                    (true, None)
                } else {
                    (false, Some("No local path assigned".to_string()))
                }
            }
            vendetta_model::MediaDownloadStatus::Skipped => {
                let r = m_rec
                    .filter_reason
                    .map(|fr| format!("Skipped: {}", fr.as_ref()))
                    .unwrap_or_else(|| "Skipped by download filter policy".to_string());
                (false, Some(r))
            }
            vendetta_model::MediaDownloadStatus::VerificationFailed => {
                (false, Some("Verification failed".to_string()))
            }
            vendetta_model::MediaDownloadStatus::PermanentlyFailed => {
                (false, Some("Download permanently failed".to_string()))
            }
            _ => (
                false,
                Some("Download pending / not yet fetched".to_string()),
            ),
        };

        media_items.push(RenderMediaItem {
            record: m_rec,
            relative_url: rel_url,
            is_available,
            unavailable_reason,
        });
    }

    let reactions = resolve_reactions(ctx.db, msg, ctx.available_avatars, ctx.media_src_dir);

    Ok(RenderMessage {
        key: msg.key,
        date: msg.date,
        sender_id: msg.sender_id,
        sender_name,
        is_outgoing: false,
        state: msg.state,
        formatted_html,
        raw_text: msg.text.clone(),
        reply_preview,
        forward_info,
        media_items,
        revisions,
        grouped_id: msg.grouped_id,
        is_service,
        service_description,
        views: msg.views,
        forwards_count: msg.forwards_count,
        author_signature,
        reply_to_top_id: msg.reply_to_top_id,
        reactions,
        is_channel_post,
        comments_count,
        has_comments,
    })
}

pub fn resolve_reactions(
    db: &ArchiveDb,
    msg: &MessageRecord,
    available_avatars: &HashSet<PeerId>,
    media_src_dir: Option<&Path>,
) -> Vec<RenderReactionGroup> {
    let Some(ref raw_json) = msg.reactions_json else {
        return Vec::new();
    };

    let Some(data) = vendetta_model::parse_reactions_json(raw_json) else {
        return Vec::new();
    };

    let mut groups = Vec::new();

    for res in data.results {
        let is_chosen = res.chosen_order.is_some()
            || data
                .recent_reactors
                .iter()
                .any(|r| r.is_me && r.reaction == res.reaction);

        let matching_reactors: Vec<RenderReactor> = data
            .recent_reactors
            .iter()
            .filter(|r| r.reaction == res.reaction)
            .map(|r| {
                let (name, username) = if let Ok(Some(p)) = db.get_peer(r.peer_id) {
                    (
                        p.name
                            .unwrap_or_else(|| format!("User {}", r.peer_id.raw())),
                        p.username,
                    )
                } else {
                    (format!("User {}", r.peer_id.raw()), None)
                };

                let avatar_markup = crate::url_builder::render_avatar_markup(
                    Some(r.peer_id),
                    &name,
                    2,
                    false,
                    "avatar-img",
                    available_avatars,
                );

                RenderReactor {
                    peer_id: r.peer_id,
                    name,
                    username,
                    avatar_markup: Some(avatar_markup),
                    is_me: r.is_me,
                }
            })
            .collect();

        let render_key = match res.reaction {
            vendetta_model::ReactionKey::Emoji(s) => RenderReactionKey::Emoji(s),
            vendetta_model::ReactionKey::CustomEmoji { document_id } => {
                let asset_path = format!("media/reactions/{document_id}.webp");
                let exists = media_src_dir.as_ref().is_some_and(|src| {
                    src.join(&asset_path).is_file()
                        || src.join(format!("reactions/{document_id}.webp")).is_file()
                });

                if exists {
                    let rel_url = ArchiveUrlBuilder::media_url(2, &asset_path);
                    RenderReactionKey::CustomEmoji {
                        document_id,
                        alt_text: None,
                        asset_rel_path: Some(rel_url),
                    }
                } else {
                    RenderReactionKey::CustomEmoji {
                        document_id,
                        alt_text: None,
                        asset_rel_path: None,
                    }
                }
            }
            vendetta_model::ReactionKey::Paid => RenderReactionKey::Paid,
            vendetta_model::ReactionKey::Unknown(s) => RenderReactionKey::Unknown(s),
        };

        groups.push(RenderReactionGroup {
            reaction: render_key,
            count: res.count,
            is_chosen_by_me: is_chosen,
            reactors: matching_reactors,
        });
    }

    groups
}

pub fn resolve_forward_info(
    db: &ArchiveDb,
    msg: &MessageRecord,
    available_avatars: &HashSet<PeerId>,
    exported_peer_ids: &HashSet<PeerId>,
) -> Option<RenderForwardInfo> {
    let mut from_peer: Option<(PeerId, PeerType)> = None;
    let mut saved_from_peer: Option<(PeerId, PeerType)> = None;
    let mut from_name: Option<String> = None;
    let mut saved_from_name: Option<String> = None;
    let mut channel_post: Option<i64> = None;
    let mut saved_from_msg_id: Option<i64> = None;
    let mut post_author: Option<String> = None;
    let mut date: Option<i64> = None;
    let mut saved_date: Option<i64> = None;

    let mut found_in_tl = false;
    if let Some(ref bytes) = msg.raw_tl
        && let Ok(tl::enums::Message::Message(m)) = tl::enums::Message::from_bytes(bytes)
        && let Some(tl::enums::MessageFwdHeader::Header(h)) = &m.fwd_from
    {
        found_in_tl = true;
        from_peer = h.from_id.as_ref().map(normalize_peer_and_type);
        saved_from_peer = h.saved_from_peer.as_ref().map(normalize_peer_and_type);
        from_name = h.from_name.clone();
        saved_from_name = h.saved_from_name.clone();
        channel_post = h.channel_post.map(|p| p as i64);
        saved_from_msg_id = h.saved_from_msg_id.map(|p| p as i64);
        post_author = h.post_author.clone();
        date = Some(h.date as i64);
        saved_date = h.saved_date.map(|d| d as i64);
    }

    if !found_in_tl
        && let Some(fj) = &msg.forward_json
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(fj)
    {
        let v = val.get("Header").unwrap_or(&val);
        if from_peer.is_none()
            && let Some(fid) = v.get("from_id")
        {
            from_peer = parse_json_peer(fid);
        }
        if saved_from_peer.is_none()
            && let Some(sid) = v.get("saved_from_peer").or_else(|| v.get("saved_from_id"))
        {
            saved_from_peer = parse_json_peer(sid);
        }
        if from_name.is_none() {
            from_name = v
                .get("from_name")
                .and_then(|x| x.as_str())
                .map(ToString::to_string);
        }
        if saved_from_name.is_none() {
            saved_from_name = v
                .get("saved_from_name")
                .and_then(|x| x.as_str())
                .map(ToString::to_string);
        }
        if channel_post.is_none() {
            channel_post = v.get("channel_post").and_then(|x| x.as_i64());
        }
        if saved_from_msg_id.is_none() {
            saved_from_msg_id = v.get("saved_from_msg_id").and_then(|x| x.as_i64());
        }
        if post_author.is_none() {
            post_author = v
                .get("post_author")
                .and_then(|x| x.as_str())
                .map(ToString::to_string);
        }
        if date.is_none() {
            date = v.get("date").and_then(|x| x.as_i64());
        }
        if saved_date.is_none() {
            saved_date = v.get("saved_date").and_then(|x| x.as_i64());
        }
    }

    if !found_in_tl && msg.forward_json.is_none() {
        return None;
    }

    let (source_peer_id, source_peer_type) = match from_peer.or(saved_from_peer) {
        Some((p, t)) => (Some(p), Some(t)),
        None => (None, None),
    };
    let origin_channel_post = channel_post.or(saved_from_msg_id);
    let origin_date = date.or(saved_date);
    let origin_signature = post_author;

    let mut origin_name: Option<String> = None;
    let mut source_username: Option<String> = None;
    let mut is_source_archived = false;
    let mut source_chat_url: Option<String> = None;
    let mut source_avatar_markup: Option<String> = None;

    if let Some(pid) = source_peer_id {
        if let Ok(Some(peer)) = db.get_peer(pid) {
            origin_name = peer.name;
            source_username = peer.username;
        }

        if exported_peer_ids.contains(&pid) {
            is_source_archived = true;
            source_chat_url = Some(ArchiveUrlBuilder::chat_root_url(2, pid));
        }

        let display_name_for_avatar = origin_name.as_deref().unwrap_or("Peer");
        source_avatar_markup = Some(crate::url_builder::render_avatar_markup(
            Some(pid),
            display_name_for_avatar,
            2,
            false,
            "fwd-avatar",
            available_avatars,
        ));
    }

    if origin_name.is_none() {
        origin_name = from_name.or(saved_from_name);
        if source_avatar_markup.is_none()
            && let Some(ref name) = origin_name
        {
            source_avatar_markup = Some(crate::url_builder::render_avatar_markup(
                source_peer_id,
                name,
                2,
                false,
                "fwd-avatar",
                available_avatars,
            ));
        }
    }

    if origin_name.is_none()
        && let Some(pid) = source_peer_id
    {
        let label = match source_peer_type {
            Some(PeerType::Channel) => format!("channel {}", pid.raw()),
            Some(PeerType::Group) => format!("group {}", pid.raw()),
            Some(PeerType::User) => format!("user {}", pid.raw()),
            _ => format!("peer {}", pid.raw()),
        };
        origin_name = Some(label);
        if source_avatar_markup.is_none() {
            source_avatar_markup = Some(crate::url_builder::render_avatar_markup(
                Some(pid),
                &format!("{}", pid.raw()),
                2,
                false,
                "fwd-avatar",
                available_avatars,
            ));
        }
    }

    Some(RenderForwardInfo {
        source_peer_id,
        source_peer_type,
        origin_name,
        source_username,
        origin_channel_post,
        origin_date,
        origin_signature,
        source_avatar_markup,
        is_source_archived,
        source_chat_url,
    })
}

pub fn format_service_action(action: &tl::enums::MessageAction) -> String {
    match action {
        tl::enums::MessageAction::ChatCreate(c) => {
            format!("Created group \"{}\"", c.title)
        }
        tl::enums::MessageAction::ChatEditTitle(c) => {
            format!("Changed group name to \"{}\"", c.title)
        }
        tl::enums::MessageAction::ChatEditPhoto(_) => "Changed group photo".to_string(),
        tl::enums::MessageAction::ChatDeletePhoto => "Removed group photo".to_string(),
        tl::enums::MessageAction::ChatAddUser(u) => {
            format!("Added {} users", u.users.len())
        }
        tl::enums::MessageAction::ChatDeleteUser(u) => {
            format!("Removed user {}", u.user_id)
        }
        tl::enums::MessageAction::ChatJoinedByLink(_) => "Joined chat via invite link".to_string(),
        tl::enums::MessageAction::ChannelCreate(c) => {
            format!("Created channel \"{}\"", c.title)
        }
        tl::enums::MessageAction::PinMessage => "Pinned a message".to_string(),
        tl::enums::MessageAction::ScreenshotTaken => "Took a screenshot".to_string(),
        tl::enums::MessageAction::CustomAction(c) => c.message.clone(),
        tl::enums::MessageAction::BotAllowed(b) => {
            format!("Allowed bot {}", b.domain.as_deref().unwrap_or(""))
        }
        tl::enums::MessageAction::SetChatTheme(_) => "Changed chat theme".to_string(),
        tl::enums::MessageAction::ChatJoinedByRequest => "Joined chat via join request".to_string(),
        tl::enums::MessageAction::WebViewDataSent(w) => {
            format!("Sent web app data: {}", w.text)
        }
        tl::enums::MessageAction::GiftCode(_) => "Telegram Premium gift code".to_string(),
        tl::enums::MessageAction::TopicCreate(t) => format!("Created topic \"{}\"", t.title),
        tl::enums::MessageAction::TopicEdit(t) => {
            if let Some(title) = &t.title {
                format!("Renamed topic to \"{title}\"")
            } else if t.closed == Some(true) {
                "Closed topic".to_string()
            } else if t.closed == Some(false) {
                "Reopened topic".to_string()
            } else if t.hidden == Some(true) {
                "Hidden topic".to_string()
            } else if t.hidden == Some(false) {
                "Unhidden topic".to_string()
            } else {
                "Edited topic".to_string()
            }
        }
        tl::enums::MessageAction::SuggestProfilePhoto(_) => "Suggested profile photo".to_string(),
        tl::enums::MessageAction::RequestedPeer(_) => "Shared requested peer".to_string(),
        tl::enums::MessageAction::SetChatWallPaper(_) => "Changed chat wallpaper".to_string(),
        tl::enums::MessageAction::StarGift(_) => "Telegram Star gift".to_string(),
        tl::enums::MessageAction::StarGiftUnique(_) => "Unique Telegram Star gift".to_string(),
        tl::enums::MessageAction::ChatMigrateTo(_) => "Migrated group to supergroup".to_string(),
        tl::enums::MessageAction::ChannelMigrateFrom(c) => {
            format!("Supergroup migrated from basic chat {}", c.chat_id)
        }
        tl::enums::MessageAction::HistoryClear => "Cleared chat history".to_string(),
        tl::enums::MessageAction::GiftPremium(_) => "Gifted Telegram Premium".to_string(),
        tl::enums::MessageAction::GiveawayLaunch(_) => "Launched giveaway".to_string(),
        tl::enums::MessageAction::GiveawayResults(_) => "Giveaway results".to_string(),
        tl::enums::MessageAction::PrizeStars(_) => "Won Telegram Stars prize".to_string(),
        _ => "Service event".to_string(),
    }
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

pub fn parse_json_peer(val: &serde_json::Value) -> Option<(PeerId, PeerType)> {
    if let Some(uid) = val
        .get("user_id")
        .or_else(|| val.get("User").and_then(|x| x.get("user_id")))
        .and_then(|x| x.as_i64())
    {
        return Some((PeerId::new(uid), PeerType::User));
    }
    if let Some(cid) = val
        .get("channel_id")
        .or_else(|| val.get("Channel").and_then(|x| x.get("channel_id")))
        .and_then(|x| x.as_i64())
    {
        let pid = if cid > 0 {
            -1_000_000_000_000 - cid
        } else {
            cid
        };
        return Some((PeerId::new(pid), PeerType::Channel));
    }
    if let Some(chid) = val
        .get("chat_id")
        .or_else(|| val.get("Chat").and_then(|x| x.get("chat_id")))
        .and_then(|x| x.as_i64())
    {
        let pid = if chid > 0 { -chid } else { chid };
        return Some((PeerId::new(pid), PeerType::Group));
    }
    if let Some(raw) = val.as_i64() {
        let pt = if raw < -1_000_000_000_000 {
            PeerType::Channel
        } else if raw < 0 {
            PeerType::Group
        } else {
            PeerType::User
        };
        return Some((PeerId::new(raw), pt));
    }
    None
}
