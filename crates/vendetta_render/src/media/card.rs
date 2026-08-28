use vendetta_model::MediaKind;

use crate::{
    entity::html_escape,
    media::placeholder::{format_file_size, render_unavailable_media},
    model::RenderMediaItem,
};

pub fn render_media_card(item: &RenderMediaItem) -> String {
    if !item.is_available {
        return render_unavailable_media(item);
    }

    let Some(rel_url) = &item.relative_url else {
        return render_unavailable_media(item);
    };

    let safe_url = html_escape(rel_url);

    match item.record.kind {
        MediaKind::Photo => format!(
            r##"<div class="media-card media-photo">
  <a href="{safe_url}" class="media-lightbox-trigger" data-media-type="photo">
    <img src="{safe_url}" alt="Photo" loading="lazy" class="photo-img">
  </a>
</div>
"##
        ),
        MediaKind::Video | MediaKind::VideoNote | MediaKind::Animation => format!(
            r##"<div class="media-card media-video">
  <video controls preload="metadata" class="video-player">
    <source src="{safe_url}">
    <a href="{safe_url}" download>Download video</a>
  </video>
</div>
"##
        ),
        MediaKind::Voice | MediaKind::Audio => {
            let title = item.record.file_name.as_deref().unwrap_or(
                if item.record.kind == MediaKind::Voice {
                    "Voice Message"
                } else {
                    "Audio Track"
                },
            );
            let size = item
                .record
                .size_bytes
                .map(format_file_size)
                .unwrap_or_default();

            format!(
                r##"<div class="media-card media-audio">
  <div class="audio-meta">
    <strong class="audio-title">{}</strong> <span class="audio-size">{}</span>
  </div>
  <audio controls preload="metadata" class="audio-player">
    <source src="{safe_url}">
    <a href="{safe_url}" download>Download audio</a>
  </audio>
</div>
"##,
                html_escape(title),
                html_escape(&size)
            )
        }
        MediaKind::Sticker => {
            let mime = item.record.mime_type.as_deref().unwrap_or("");
            let file_name = item.record.file_name.as_deref().unwrap_or("");

            if mime == "video/webm" || file_name.ends_with(".webm") {
                format!(
                    r##"<div class="media-card media-sticker">
  <video class="sticker-video" autoplay loop muted playsinline>
    <source src="{safe_url}" type="video/webm">
  </video>
</div>
"##
                )
            } else if mime == "application/x-tgsticker" || file_name.ends_with(".tgs") {
                format!(
                    r##"<div class="media-card media-sticker">
  <canvas class="sticker-canvas" data-tgs-url="{safe_url}" width="192" height="192" aria-label="Animated Sticker">
    <div class="sticker-fallback"><svg class="icon"><use href="#icon-sticker"></use></svg><span>Sticker</span></div>
  </canvas>
</div>
"##
                )
            } else {
                format!(
                    r##"<div class="media-card media-sticker">
  <img src="{safe_url}" alt="Sticker" loading="lazy" class="sticker-img">
</div>
"##
                )
            }
        }
        _ => {
            let filename = item.record.file_name.as_deref().unwrap_or("Attachment");
            let mime = item
                .record
                .mime_type
                .as_deref()
                .unwrap_or("application/octet-stream");
            let size = item
                .record
                .size_bytes
                .map(format_file_size)
                .unwrap_or_default();

            format!(
                r##"<div class="media-card media-document">
  <div class="doc-icon"><svg class="icon"><use href="#icon-document"></use></svg></div>
  <div class="doc-info">
    <a href="{safe_url}" download class="doc-filename">{}</a>
    <div class="doc-meta"><span class="doc-size">{}</span> • <span class="doc-mime">{}</span></div>
  </div>
</div>
"##,
                html_escape(filename),
                html_escape(&size),
                html_escape(mime)
            )
        }
    }
}
