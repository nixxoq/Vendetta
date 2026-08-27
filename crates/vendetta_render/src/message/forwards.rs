use std::fmt::Write;

use crate::{entity::html_escape, message::edits::chrono_like_format, model::RenderForwardInfo};

pub fn render_forward_header(fwd: &RenderForwardInfo) -> String {
    let mut html = String::with_capacity(256);
    html.push_str("<div class=\"forward-header\">\n  <div class=\"fwd-top\">\n");

    if let Some(ref av) = fwd.source_avatar_markup {
        let _ = writeln!(html, "    {av}");
    }

    html.push_str("    <span class=\"fwd-label\">Forwarded from</span> ");

    if let Some(origin) = &fwd.origin_name {
        let escaped_name = html_escape(origin);
        if fwd.is_source_archived
            && let Some(ref chat_url) = fwd.source_chat_url
        {
            let _ = write!(
                html,
                "<a href=\"{chat_url}\" class=\"fwd-origin\">{escaped_name}</a>"
            );
        } else {
            let _ = write!(html, "<strong class=\"fwd-origin\">{escaped_name}</strong>");
        }
    } else if let Some(pid) = fwd.source_peer_id {
        let _ = write!(
            html,
            "<strong class=\"fwd-origin\">peer {}</strong>",
            pid.raw()
        );
    } else {
        html.push_str("<em class=\"fwd-origin-unavailable\">unavailable source</em>");
    }

    if let Some(username) = &fwd.source_username {
        let clean_user = username.trim_start_matches('@');
        let _ = write!(
            html,
            " <span class=\"fwd-username\">(@{})</span>",
            html_escape(clean_user)
        );
    }

    if let Some(sig) = &fwd.origin_signature {
        let _ = write!(
            html,
            " <span class=\"fwd-sig\">({})</span>",
            html_escape(sig)
        );
    }

    html.push_str("\n  </div>\n");

    let has_meta = fwd.source_peer_id.is_some()
        || fwd.origin_channel_post.is_some()
        || fwd.origin_date.is_some();

    if has_meta {
        html.push_str("  <div class=\"fwd-meta\">\n");

        if let Some(pid) = fwd.source_peer_id {
            let _ = writeln!(html, "    <span class=\"fwd-id\">ID: {}</span>", pid.raw());
        }

        if let Some(msg_id) = fwd.origin_channel_post {
            let _ = writeln!(html, "    <span class=\"fwd-msg-id\">Msg: #{msg_id}</span>");
        }

        if let Some(date) = fwd.origin_date {
            let date_str = chrono_like_format(date);
            let _ = writeln!(html, "    <time class=\"fwd-date\">{date_str}</time>");
        }

        html.push_str("  </div>\n");
    }

    html.push_str("</div>\n");
    html
}
