use crate::{entity::html_escape, model::RenderMediaItem};

pub fn render_unavailable_media(item: &RenderMediaItem) -> String {
    let kind_label = item.record.kind.as_ref();
    let file_name = item.record.file_name.as_deref().unwrap_or("Attachment");
    let size_label = item
        .record
        .size_bytes
        .map(format_file_size)
        .unwrap_or_default();
    let reason = item
        .unavailable_reason
        .as_deref()
        .unwrap_or("Media binary not available in this archive");

    format!(
        r##"<div class="media-card media-card-unavailable">
  <div class="media-icon"><svg class="icon"><use href="#icon-file-warning"></use></svg></div>
  <div class="media-info">
    <div class="media-title">{}</div>
    <div class="media-meta"><span class="media-kind">{}</span> {} • <span class="media-status">{}</span></div>
  </div>
</div>
"##,
        html_escape(file_name),
        html_escape(kind_label),
        html_escape(&size_label),
        html_escape(reason)
    )
}

pub fn format_file_size(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
