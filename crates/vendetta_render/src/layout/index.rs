use std::{collections::HashSet, fmt::Write};

use vendetta_model::PeerId;

use crate::{
    assets::SYMBOLS_SVG,
    entity::html_escape,
    message::edits::chrono_like_format,
    model::{ExportSummary, PresentationMode, RenderPeer, ThemeMode},
    url_builder::{ArchiveUrlBuilder, render_avatar_markup},
};

pub fn render_global_index(
    peers: &[RenderPeer],
    presentation_mode: PresentationMode,
    theme: ThemeMode,
    summary: &ExportSummary,
    available_avatars: &HashSet<PeerId>,
) -> String {
    let mode_css = match presentation_mode {
        PresentationMode::TelegramLike => "telegram_like.css",
        PresentationMode::ArchiveOptimized => "archive_dense.css",
    };

    let theme_attr = match theme {
        ThemeMode::Light => "data-theme=\"light\"",
        ThemeMode::Dark => "data-theme=\"dark\"",
        ThemeMode::System => "",
    };

    let mut dialogs_html = String::with_capacity(peers.len() * 256);
    for peer in peers {
        let chat_url = ArchiveUrlBuilder::chat_root_url(0, peer.peer_id);
        let avatar_html = render_avatar_markup(
            Some(peer.peer_id),
            &peer.name,
            0,
            false,
            "dialog-avatar",
            available_avatars,
        );
        let type_badge = peer.peer_type.as_ref();

        let time_str = peer
            .last_message_date
            .map(chrono_like_format)
            .unwrap_or_default();

        let _ = write!(
            dialogs_html,
            r#"<li class="dialog-item">
  <a href="{chat_url}" style="display: flex; gap: 0.75rem; width: 100%; color: inherit; text-decoration: none;">
    {avatar_html}
    <div class="dialog-info">
      <div class="dialog-top-row">
        <span class="dialog-name">{}</span>
        <span class="dialog-time">{}</span>
      </div>
      <div class="dialog-snippet">{} messages &bull; <span class="badge-type">{}</span></div>
    </div>
  </a>
</li>
"#,
            html_escape(&peer.name),
            html_escape(&time_str),
            peer.total_messages,
            html_escape(type_badge)
        );
    }

    format!(
        r##"<!DOCTYPE html>
<html lang="en" {theme_attr}>
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Telegram Archive</title>
  <script>
    (function() {{
      try {{
        var saved = localStorage.getItem('vendetta-theme');
        var theme = (saved === 'dark' || saved === 'light') ? saved : (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
        document.documentElement.setAttribute('data-theme', theme);
        if (document.documentElement.dataset) {{
          document.documentElement.dataset.theme = theme;
        }}
      }} catch (e) {{
        var isDark = window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
        document.documentElement.setAttribute('data-theme', isDark ? 'dark' : 'light');
        if (document.documentElement.dataset) {{
          document.documentElement.dataset.theme = isDark ? 'dark' : 'light';
        }}
      }}
    }})();
  </script>
  <link rel="stylesheet" href="assets/css/theme.css">
  <link rel="stylesheet" href="assets/css/main.css">
  <link rel="stylesheet" href="assets/css/{mode_css}">
</head>
<body>
  <div style="display: none;">
    {SYMBOLS_SVG}
  </div>
  <div class="app-container">
    <aside class="sidebar">
      <div class="sidebar-header">
        <h1 class="sidebar-title">Chats ({})</h1>
        <div class="sidebar-tools">
          <button id="search-open-btn" class="btn-icon" title="Search (Ctrl+K)">
            <svg class="icon"><use href="#icon-search"></use></svg>
          </button>
          <button id="theme-toggle" class="btn-icon" title="Toggle Theme">
            <svg class="icon"><use href="#icon-moon"></use></svg>
          </button>
        </div>
      </div>
      <ul class="dialogs-list">
        {dialogs_html}
      </ul>
    </aside>

    <main class="chat-pane" style="align-items: center; justify-content: center; text-align: center; padding: 2rem;">
      <div style="max-width: 500px; background: var(--bg-primary); padding: 2.5rem; border-radius: 16px; box-shadow: var(--shadow-md); border: 1px solid var(--border-color);">
        <h2 style="font-size: 1.5rem; margin-bottom: 1rem;">Telegram Archive</h2>
        <p style="color: var(--text-secondary); margin-bottom: 1.5rem;">Select a conversation from the sidebar to view archived message history.</p>
        <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; text-align: left; background: var(--bg-secondary); padding: 1rem; border-radius: 8px; font-size: 0.875rem;">
          <div><strong>Chats:</strong> {}</div>
          <div><strong>Messages:</strong> {}</div>
          <div><strong>Chunks:</strong> {}</div>
          <div><strong>Media:</strong> {}</div>
        </div>
      </div>
    </main>
  </div>

  <div id="search-modal" class="modal-overlay" data-base-path="">
    <div class="modal-card">
      <div class="search-header">
        <svg class="icon" style="align-self: center; color: var(--text-muted);"><use href="#icon-search"></use></svg>
        <input type="text" id="search-input" class="search-input" placeholder="Search all messages or filter by chat..." autofocus>
      </div>
      <div class="search-filters">
        <select id="search-peer-filter" class="filter-select" title="Filter by chat">
          <option value="">All Chats</option>
        </select>
        <input type="text" id="search-sender-filter" class="filter-input" placeholder="Sender...">
        <input type="date" id="search-date-from" class="filter-input" title="Date from">
        <input type="date" id="search-date-to" class="filter-input" title="Date to">
        <select id="search-media-filter" class="filter-select" title="Filter by media type">
          <option value="">All Media</option>
          <option value="photo">Photos</option>
          <option value="video">Videos</option>
          <option value="audio">Audio</option>
          <option value="voice">Voice</option>
          <option value="document">Documents</option>
          <option value="sticker">Stickers</option>
        </select>
        <label class="filter-checkbox-label"><input type="checkbox" id="search-has-reply"> Reply</label>
        <label class="filter-checkbox-label"><input type="checkbox" id="search-is-edited"> Edited</label>
        <label class="filter-checkbox-label"><input type="checkbox" id="search-is-deleted"> Deleted</label>
        <label class="filter-checkbox-label"><input type="checkbox" id="search-is-forward"> Forwarded</label>
      </div>
      <ul id="search-results-list" class="search-results">
        <li class="search-result-item text-muted">Type to search messages...</li>
      </ul>
    </div>
  </div>

  <script src="assets/js/app.js"></script>
  <script src="assets/js/lightbox.js"></script>
  <script src="assets/js/search.js"></script>
</body>
</html>"##,
        peers.len(),
        summary.dialogs_count,
        summary.messages_count,
        summary.chunks_count,
        summary.media_copied_count
    )
}
