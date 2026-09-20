use vendetta_model::MediaKind;

use crate::{
    entity::html_escape, message::edits::chrono_like_format, model::RenderMessage,
    url_builder::ArchiveUrlBuilder,
};

pub fn render_service_message(msg: &RenderMessage, is_unified: bool) -> String {
    let anchor = if is_unified {
        ArchiveUrlBuilder::unified_message_anchor(msg.key.peer_id, msg.key.message_id)
    } else {
        ArchiveUrlBuilder::message_anchor(msg.key.peer_id, msg.key.message_id)
    };
    let desc = msg
        .service_description
        .as_deref()
        .or(msg.raw_text.as_deref())
        .unwrap_or("System action");

    let date_str = chrono_like_format(msg.date);
    let short_time = format_short_time(msg.date);

    let photo_url = msg.media_items.iter().find_map(|m| {
        if m.is_available && matches!(m.record.kind, MediaKind::Photo) {
            m.relative_url.as_deref()
        } else {
            None
        }
    });

    let is_suggest_profile_photo = [
        "suggested this photo",
        "Suggested this photo",
        "Suggested profile photo",
    ]
    .iter()
    .any(|needle| desc.contains(needle));

    if is_suggest_profile_photo && let Some(rel_url) = photo_url {
        let safe_url = html_escape(rel_url);
        let safe_desc = html_escape(desc);

        return format!(
            r#"<div class="system-event system-event-card" id="{anchor}">
  <div class="service-card">
    <a href="{safe_url}" data-full-src="{safe_url}" class="service-card-avatar-wrap media-lightbox-trigger" data-media-type="photo">
      <img src="{safe_url}" alt="Profile Photo" class="service-card-avatar" loading="lazy">
    </a>
    <div class="service-card-text">{safe_desc}</div>
    <div class="service-card-actions">
      <a href="{safe_url}" data-full-src="{safe_url}" class="service-card-btn media-lightbox-trigger" data-media-type="photo">View Photo</a>
    </div>
    <div class="service-card-meta">
      <time title="{date_str}">{short_time}</time>
    </div>
  </div>
</div>
"#
        );
    }

    format!(
        r#"<div class="system-event" id="{anchor}">
  <span class="system-event-bubble">{} <time title="{date_str}">{short_time}</time></span>
</div>
"#,
        html_escape(desc)
    )
}

pub fn format_short_time(ts: i64) -> String {
    let rem = (ts % 86400).unsigned_abs();
    let hours = rem / 3600;
    let minutes = (rem % 3600) / 60;
    format!("{hours:02}:{minutes:02}")
}
