use std::fmt::Write;

use vendetta_model::MediaKind;

use crate::{media::card::render_media_card, model::RenderMediaItem};

pub fn render_album_gallery(
    grouped_id: i64,
    items: &[RenderMediaItem],
    continuation_prev: Option<&str>,
    continuation_next: Option<&str>,
) -> String {
    if items.is_empty() {
        return String::new();
    }

    let (visual_items, doc_items): (Vec<_>, Vec<_>) = items.iter().partition(|item| {
        matches!(
            item.record.kind,
            MediaKind::Photo | MediaKind::Video | MediaKind::Animation
        )
    });

    let mut html = String::with_capacity(items.len() * 300 + 128);

    if !visual_items.is_empty() {
        let count = visual_items.len();
        let _ = writeln!(
            html,
            "<div class=\"media-album album-count-{count}\" data-grouped-id=\"{grouped_id}\">"
        );

        if let Some(prev_url) = continuation_prev {
            let _ = writeln!(
                html,
                "  <div class=\"album-continuation-badge badge-prev\"><a href=\"{prev_url}\" class=\"continuation-link\">▲ Continued from previous page</a></div>"
            );
        }

        html.push_str("  <div class=\"album-grid\">\n");
        for item in visual_items {
            html.push_str("    <div class=\"album-item\">\n");
            let _ = writeln!(html, "      {}", render_media_card(item).trim());
            html.push_str("    </div>\n");
        }
        html.push_str("  </div>\n");

        if let Some(next_url) = continuation_next {
            let _ = writeln!(
                html,
                "  <div class=\"album-continuation-badge badge-next\"><a href=\"{next_url}\" class=\"continuation-link\">▼ Continues on next page</a></div>"
            );
        }

        html.push_str("</div>\n");
    }

    for doc in doc_items {
        let _ = writeln!(html, "{}", render_media_card(doc).trim());
    }

    html
}
