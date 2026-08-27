use crate::{
    entity::html_escape, message::edits::chrono_like_format, model::RenderMessage,
    url_builder::ArchiveUrlBuilder,
};

pub fn render_service_message(msg: &RenderMessage) -> String {
    let anchor = ArchiveUrlBuilder::message_anchor(msg.key.peer_id, msg.key.message_id);
    let desc = msg
        .service_description
        .as_deref()
        .or(msg.raw_text.as_deref())
        .unwrap_or("System action");

    let date_str = chrono_like_format(msg.date);
    let short_time = format_short_time(msg.date);

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
