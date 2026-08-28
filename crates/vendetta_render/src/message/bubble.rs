use std::{collections::HashSet, fmt::Write};

use vendetta_model::{MessageState, PeerId};

use crate::{
    entity::html_escape,
    media::{format_file_size, render_album_gallery, render_media_card},
    message::{
        deletes::render_state_indicator,
        edits::{chrono_like_format, render_edit_history},
        forwards::render_forward_header,
        reactions::render_message_reactions,
        service::{format_short_time, render_service_message},
    },
    model::{
        PresentationMode, RenderAlbum, RenderItem, RenderMessage, RenderReactionGroup,
        RenderReactionKey,
    },
    reply::render_reply_card,
    url_builder::{ArchiveUrlBuilder, render_avatar_markup},
};

#[derive(Debug, Clone, Copy, Default)]
pub struct GroupingContext {
    pub is_first_in_group: bool,
    pub is_last_in_group: bool,
    pub show_sender: bool,
    pub show_avatar: bool,
}

pub fn render_chat_item(
    item: &RenderItem,
    ctx: &GroupingContext,
    mode: PresentationMode,
    page_idx: usize,
    total_pages: usize,
    available_avatars: &HashSet<PeerId>,
) -> String {
    match item {
        RenderItem::Message(msg) => render_message_bubble(msg, ctx, mode, available_avatars),
        RenderItem::Album(album) => {
            render_album_bubble(album, ctx, mode, page_idx, total_pages, available_avatars)
        }
    }
}

pub fn render_message_bubble(
    msg: &RenderMessage,
    ctx: &GroupingContext,
    mode: PresentationMode,
    available_avatars: &HashSet<PeerId>,
) -> String {
    if msg.is_service {
        return render_service_message(msg);
    }

    match mode {
        PresentationMode::TelegramLike => render_telegram_like(msg, ctx, available_avatars),
        PresentationMode::ArchiveOptimized => render_archive_optimized(msg),
    }
}

pub fn render_album_bubble(
    album: &RenderAlbum,
    ctx: &GroupingContext,
    mode: PresentationMode,
    page_idx: usize,
    total_pages: usize,
    available_avatars: &HashSet<PeerId>,
) -> String {
    match mode {
        PresentationMode::TelegramLike => {
            render_album_telegram_like(album, ctx, page_idx, total_pages, available_avatars)
        }
        PresentationMode::ArchiveOptimized => {
            render_album_archive_optimized(album, page_idx, total_pages)
        }
    }
}

fn render_telegram_like(
    msg: &RenderMessage,
    ctx: &GroupingContext,
    available_avatars: &HashSet<PeerId>,
) -> String {
    let anchor = ArchiveUrlBuilder::message_anchor(msg.key.peer_id, msg.key.message_id);
    let mut classes = vec!["message-row"];
    if msg.is_outgoing {
        classes.push("msg-outgoing");
    } else {
        classes.push("msg-incoming");
    }

    if ctx.is_first_in_group {
        classes.push("group-first");
    }
    if ctx.is_last_in_group {
        classes.push("group-last");
    }
    if msg.state == MessageState::Deleted {
        classes.push("state-deleted");
    }

    let is_sticker_msg = msg.media_items.len() == 1
        && msg.media_items[0].record.kind == vendetta_model::MediaKind::Sticker
        && msg.formatted_html.is_none()
        && msg.raw_text.as_deref().unwrap_or("").is_empty();

    if is_sticker_msg {
        classes.push("msg-sticker");
    }
    if msg.is_channel_post {
        classes.push("channel-post");
    }

    let class_attr = classes.join(" ");
    let mut html = String::with_capacity(512);
    let _ = writeln!(html, "<div class=\"{class_attr}\" id=\"{anchor}\">");

    if !msg.is_channel_post {
        if ctx.show_avatar && !msg.is_outgoing {
            let sender_id = msg.sender_id.unwrap_or(msg.key.peer_id);
            let sender_name = msg.sender_name.as_deref().unwrap_or("User");
            let avatar_html = render_avatar_markup(
                Some(sender_id),
                sender_name,
                2,
                false,
                "avatar",
                available_avatars,
            );
            let _ = writeln!(html, "  {avatar_html}");
        } else if !msg.is_outgoing {
            html.push_str("  <div class=\"avatar avatar-placeholder\"></div>\n");
        }
    }

    html.push_str("  <div class=\"message-bubble\">\n");

    if ctx.show_sender
        && !msg.is_outgoing
        && !msg.is_channel_post
        && let Some(name) = &msg.sender_name
    {
        let _ = writeln!(
            html,
            "    <div class=\"message-sender\">{}</div>",
            html_escape(name)
        );
    }

    if let Some(fwd) = &msg.forward_info {
        let _ = writeln!(html, "    {}", render_forward_header(fwd).trim());
    }

    if let Some(reply) = &msg.reply_preview {
        let _ = writeln!(html, "    {}", render_reply_card(reply).trim());
    }

    if let Some(indicator) = render_state_indicator(msg.state) {
        let _ = writeln!(html, "    {indicator}");
    }

    if !msg.media_items.is_empty() {
        html.push_str("    <div class=\"message-media-container\">\n");
        if msg.media_items.len() > 1 {
            let gid = msg.grouped_id.unwrap_or(0);
            let _ = writeln!(
                html,
                "      {}",
                render_album_gallery(gid, &msg.media_items, None, None).trim()
            );
        } else {
            for media in &msg.media_items {
                let _ = writeln!(html, "      {}", render_media_card(media).trim());
            }
        }
        html.push_str("    </div>\n");
    }

    let text_cls = if !msg.media_items.is_empty() {
        "message-text message-caption"
    } else {
        "message-text"
    };

    if let Some(text_html) = &msg.formatted_html {
        if !text_html.is_empty() {
            let _ = writeln!(html, "    <div class=\"{text_cls}\">{text_html}</div>");
        }
    } else if let Some(raw) = &msg.raw_text
        && !raw.is_empty()
        && msg.state != MessageState::Deleted
    {
        let _ = writeln!(
            html,
            "    <div class=\"{text_cls}\">{}</div>",
            html_escape(raw).replace('\n', "<br>")
        );
    }

    if !msg.revisions.is_empty() {
        let _ = writeln!(html, "    {}", render_edit_history(&msg.revisions).trim());
    }

    if !msg.reactions.is_empty() {
        html.push_str(&render_message_reactions(&msg.reactions));
    }

    let full_time = chrono_like_format(msg.date);
    let short_time = format_short_time(msg.date);

    html.push_str("    <div class=\"message-meta\">\n");
    if let Some(sig) = &msg.author_signature {
        let _ = writeln!(
            html,
            "      <span class=\"meta-sig\">{}</span>",
            html_escape(sig)
        );
    }
    if let Some(views) = msg.views {
        let _ = writeln!(html, "      <span class=\"meta-views\">👁 {views}</span>");
    }
    if msg.state == MessageState::Edited {
        html.push_str("      <span class=\"meta-edited\">edited</span>\n");
    }
    let _ = writeln!(
        html,
        "      <time datetime=\"{full_time}\" title=\"{full_time}\" class=\"meta-time\">{short_time}</time>"
    );
    html.push_str("    </div>\n");

    if let Some(count) = msg.comments_count {
        let _ = writeln!(
            html,
            "    <div class=\"channel-comments-bar\"><span class=\"comments-icon\">💬</span> <span class=\"comments-label\">{} comment{}</span></div>",
            count,
            if count == 1 { "" } else { "s" }
        );
    } else if msg.has_comments {
        html.push_str("    <div class=\"channel-comments-bar\"><span class=\"comments-icon\">💬</span> <span class=\"comments-label\">Leave a comment</span></div>\n");
    }

    html.push_str("  </div>\n");
    html.push_str("</div>\n");
    html
}

fn render_album_telegram_like(
    album: &RenderAlbum,
    ctx: &GroupingContext,
    page_idx: usize,
    total_pages: usize,
    available_avatars: &HashSet<PeerId>,
) -> String {
    let Some(primary_msg) = album.messages.first() else {
        return String::new();
    };

    let primary_anchor =
        ArchiveUrlBuilder::message_anchor(primary_msg.key.peer_id, primary_msg.key.message_id);
    let mut classes = vec!["message-row", "album-row"];
    if primary_msg.is_outgoing {
        classes.push("msg-outgoing");
    } else {
        classes.push("msg-incoming");
    }

    if ctx.is_first_in_group {
        classes.push("group-first");
    }
    if ctx.is_last_in_group {
        classes.push("group-last");
    }
    if primary_msg.is_channel_post {
        classes.push("channel-post");
    }

    let class_attr = classes.join(" ");
    let mut html = String::with_capacity(1024);
    let _ = writeln!(html, "<div class=\"{class_attr}\" id=\"{primary_anchor}\">");

    for msg in album.messages.iter().skip(1) {
        let sub_anchor = ArchiveUrlBuilder::message_anchor(msg.key.peer_id, msg.key.message_id);
        let _ = writeln!(
            html,
            "  <span id=\"{sub_anchor}\" class=\"album-sub-anchor\"></span>"
        );
    }

    if !primary_msg.is_channel_post {
        if ctx.show_avatar && !primary_msg.is_outgoing {
            let sender_id = primary_msg.sender_id.unwrap_or(primary_msg.key.peer_id);
            let sender_name = primary_msg.sender_name.as_deref().unwrap_or("User");
            let avatar_html = render_avatar_markup(
                Some(sender_id),
                sender_name,
                2,
                false,
                "avatar",
                available_avatars,
            );
            let _ = writeln!(html, "  {avatar_html}");
        } else if !primary_msg.is_outgoing {
            html.push_str("  <div class=\"avatar avatar-placeholder\"></div>\n");
        }
    }

    html.push_str("  <div class=\"message-bubble album-bubble-container\">\n");

    if ctx.show_sender
        && !primary_msg.is_outgoing
        && !primary_msg.is_channel_post
        && let Some(name) = &primary_msg.sender_name
    {
        let _ = writeln!(
            html,
            "    <div class=\"message-sender\">{}</div>",
            html_escape(name)
        );
    }

    for msg in &album.messages {
        if let Some(fwd) = &msg.forward_info {
            let _ = writeln!(html, "    {}", render_forward_header(fwd).trim());
            break;
        }
    }

    for msg in &album.messages {
        if let Some(reply) = &msg.reply_preview {
            let _ = writeln!(html, "    {}", render_reply_card(reply).trim());
            break;
        }
    }

    let prev_url = (album.continuation_prev && page_idx > 0)
        .then(|| ArchiveUrlBuilder::page_file_name(page_idx - 1));
    let next_url = (album.continuation_next && page_idx + 1 < total_pages)
        .then(|| ArchiveUrlBuilder::page_file_name(page_idx + 1));

    html.push_str("    <div class=\"message-media-container album-gallery-wrapper\">\n");
    let _ = writeln!(
        html,
        "      {}",
        render_album_gallery(
            album.grouped_id,
            &album.media_items,
            prev_url.as_deref(),
            next_url.as_deref(),
        )
        .trim()
    );
    html.push_str("    </div>\n");

    for msg in &album.messages {
        if let Some(text_html) = &msg.formatted_html {
            if !text_html.is_empty() {
                let _ = writeln!(
                    html,
                    "    <div class=\"message-text message-caption\">{text_html}</div>"
                );
            }
        } else if let Some(raw) = &msg.raw_text
            && !raw.is_empty()
            && msg.state != MessageState::Deleted
        {
            let _ = writeln!(
                html,
                "    <div class=\"message-text message-caption\">{}</div>",
                html_escape(raw).replace('\n', "<br>")
            );
        }
    }

    for msg in &album.messages {
        if !msg.revisions.is_empty() {
            let _ = writeln!(html, "    {}", render_edit_history(&msg.revisions).trim());
        }
    }

    let album_reactions = aggregate_album_reactions(&album.messages);
    if !album_reactions.is_empty() {
        html.push_str(&render_message_reactions(&album_reactions));
    }

    let full_time = chrono_like_format(primary_msg.date);
    let short_time = format_short_time(primary_msg.date);

    html.push_str("    <div class=\"message-meta\">\n");
    if let Some(sig) = &primary_msg.author_signature {
        let _ = writeln!(
            html,
            "      <span class=\"meta-sig\">{}</span>",
            html_escape(sig)
        );
    }
    if let Some(views) = primary_msg.views {
        let _ = writeln!(html, "      <span class=\"meta-views\">👁 {views}</span>");
    }
    if album
        .messages
        .iter()
        .any(|m| m.state == MessageState::Edited)
    {
        html.push_str("      <span class=\"meta-edited\">edited</span>\n");
    }
    let _ = writeln!(
        html,
        "      <time datetime=\"{full_time}\" title=\"{full_time}\" class=\"meta-time\">{short_time}</time>"
    );
    html.push_str("    </div>\n");

    if let Some(count) = primary_msg.comments_count {
        let _ = writeln!(
            html,
            "    <div class=\"channel-comments-bar\"><span class=\"comments-icon\">💬</span> <span class=\"comments-label\">{} comment{}</span></div>",
            count,
            if count == 1 { "" } else { "s" }
        );
    } else if primary_msg.has_comments {
        html.push_str("    <div class=\"channel-comments-bar\"><span class=\"comments-icon\">💬</span> <span class=\"comments-label\">Leave a comment</span></div>\n");
    }

    html.push_str("  </div>\n");
    html.push_str("</div>\n");
    html
}

fn reaction_icon(key: &RenderReactionKey) -> &str {
    match key {
        RenderReactionKey::Emoji(s) | RenderReactionKey::Unknown(s) => s.as_str(),
        RenderReactionKey::CustomEmoji { .. } => "✨",
        RenderReactionKey::Paid => "⭐",
    }
}

fn render_dense_message_content(html: &mut String, msg: &RenderMessage) {
    if let Some(reply) = &msg.reply_preview {
        let _ = write!(
            html,
            "    <span class=\"dense-reply-pill\">↩ {}</span> ",
            html_escape(reply.sender_name.as_deref().unwrap_or("Reply"))
        );
    }

    if let Some(indicator) = render_state_indicator(msg.state) {
        let _ = write!(html, "    {indicator} ");
    }

    if let Some(text_html) = &msg.formatted_html {
        html.push_str(text_html);
    } else if let Some(raw) = &msg.raw_text {
        html.push_str(&html_escape(raw).replace('\n', " "));
    }

    for item in &msg.media_items {
        if item.is_available
            && let Some(url) = item.relative_url.as_deref()
        {
            let name = item
                .record
                .file_name
                .as_deref()
                .unwrap_or(item.record.kind.as_ref());
            let size_str = item
                .record
                .size_bytes
                .map(format_file_size)
                .unwrap_or_default();
            let _ = write!(
                html,
                " <a href=\"{}\" class=\"dense-media-link\">[{}: {} {}]</a>",
                html_escape(url),
                item.record.kind.as_ref(),
                html_escape(name),
                html_escape(&size_str),
            );
        } else {
            let reason = item.unavailable_reason.as_deref().unwrap_or("unavailable");
            let _ = write!(
                html,
                " <span class=\"dense-media-unavailable\">[{}: {}]</span>",
                item.record.kind.as_ref(),
                html_escape(reason)
            );
        }
    }

    if !msg.revisions.is_empty() {
        let _ = write!(
            html,
            " <span class=\"dense-edited-badge\">(Edited v{})</span>",
            msg.revisions.len() + 1
        );
    }

    if !msg.reactions.is_empty() {
        for group in &msg.reactions {
            let count = group.count;
            let icon = reaction_icon(&group.reaction);
            let _ = write!(
                html,
                " <span class=\"dense-reaction-pill\">[{icon} {count}]</span>"
            );
        }
    }
}

fn render_album_archive_optimized(
    album: &RenderAlbum,
    page_idx: usize,
    total_pages: usize,
) -> String {
    let mut html = String::with_capacity(512);
    for (idx, msg) in album.messages.iter().enumerate() {
        let anchor = ArchiveUrlBuilder::message_anchor(msg.key.peer_id, msg.key.message_id);
        let full_time = chrono_like_format(msg.date);
        let sender =
            msg.sender_name
                .as_deref()
                .unwrap_or(if msg.is_outgoing { "You" } else { "Unknown" });

        let class_str = if msg.state == MessageState::Deleted {
            "dense-row dense-deleted"
        } else {
            "dense-row"
        };

        let _ = writeln!(html, "<div class=\"{class_str}\" id=\"{anchor}\">");
        let _ = writeln!(
            html,
            "  <div class=\"dense-time\"><time title=\"{full_time}\">{}</time></div>",
            format_short_time(msg.date)
        );
        let _ = writeln!(
            html,
            "  <div class=\"dense-sender\"><strong>{}</strong>:</div>",
            html_escape(sender)
        );
        html.push_str("  <div class=\"dense-body\">\n");

        if idx == 0 {
            let _ = write!(
                html,
                "    <span class=\"dense-album-badge\">[Album #{}: {} items]</span> ",
                album.grouped_id,
                album.media_items.len()
            );
            if album.continuation_prev && page_idx > 0 {
                let prev_file = ArchiveUrlBuilder::page_file_name(page_idx - 1);
                let _ = write!(
                    html,
                    "<a href=\"{prev_file}\" class=\"dense-continuation-link\">[▲ Continued from prev page]</a> "
                );
            }
            if album.continuation_next && page_idx + 1 < total_pages {
                let next_file = ArchiveUrlBuilder::page_file_name(page_idx + 1);
                let _ = write!(
                    html,
                    "<a href=\"{next_file}\" class=\"dense-continuation-link\">[▼ Continues on next page]</a> "
                );
            }
        }

        render_dense_message_content(&mut html, msg);
        html.push_str("\n  </div>\n");
        html.push_str("</div>\n");
    }
    html
}

fn render_archive_optimized(msg: &RenderMessage) -> String {
    let anchor = ArchiveUrlBuilder::message_anchor(msg.key.peer_id, msg.key.message_id);
    let full_time = chrono_like_format(msg.date);
    let sender =
        msg.sender_name
            .as_deref()
            .unwrap_or(if msg.is_outgoing { "You" } else { "Unknown" });

    let class_str = if msg.state == MessageState::Deleted {
        "dense-row dense-deleted"
    } else {
        "dense-row"
    };

    let mut html = String::with_capacity(384);
    let _ = writeln!(html, "<div class=\"{class_str}\" id=\"{anchor}\">");
    let _ = writeln!(
        html,
        "  <div class=\"dense-time\"><time title=\"{full_time}\">{}</time></div>",
        format_short_time(msg.date)
    );
    let _ = writeln!(
        html,
        "  <div class=\"dense-sender\"><strong>{}</strong>:</div>",
        html_escape(sender)
    );
    html.push_str("  <div class=\"dense-body\">\n");

    render_dense_message_content(&mut html, msg);

    html.push_str("\n  </div>\n");
    html.push_str("</div>\n");
    html
}

pub fn aggregate_album_reactions(messages: &[RenderMessage]) -> Vec<RenderReactionGroup> {
    let mut groups: Vec<RenderReactionGroup> = Vec::new();
    for msg in messages {
        for rx in &msg.reactions {
            if let Some(existing) = groups.iter_mut().find(|g| g.reaction == rx.reaction) {
                existing.count += rx.count;
                existing.is_chosen_by_me |= rx.is_chosen_by_me;
                for reactor in &rx.reactors {
                    if !existing
                        .reactors
                        .iter()
                        .any(|r| r.peer_id == reactor.peer_id)
                    {
                        existing.reactors.push(reactor.clone());
                    }
                }
            } else {
                groups.push(rx.clone());
            }
        }
    }
    groups
}
