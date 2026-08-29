use std::{collections::HashSet, fmt::Write};

use vendetta_model::{PeerId, PeerType};

use crate::{
    assets::SYMBOLS_SVG,
    entity::html_escape,
    message::{bubble::render_chat_item, compute_item_grouping_contexts, edits::days_to_ymd},
    model::{PresentationMode, RenderItem, RenderPeer, ThemeMode},
    url_builder::{ArchiveUrlBuilder, render_avatar_markup},
};

pub struct DialogPageContext<'a> {
    pub current_peer: &'a RenderPeer,
    pub all_peers: &'a [RenderPeer],
    pub current_topic: Option<&'a crate::model::RenderTopic>,
    pub topics: &'a [crate::model::RenderTopic],
    pub items: &'a [RenderItem],
    pub page_index: usize,
    pub total_pages: usize,
    pub presentation_mode: PresentationMode,
    pub theme: ThemeMode,
    pub date_nav_html: Option<&'a str>,
    pub available_avatars: &'a HashSet<PeerId>,
}

pub fn render_dialog_page(ctx: &DialogPageContext) -> String {
    let mode_css = match ctx.presentation_mode {
        PresentationMode::TelegramLike => "telegram_like.css",
        PresentationMode::ArchiveOptimized => "archive_dense.css",
    };

    let theme_attr = match ctx.theme {
        ThemeMode::Light => "data-theme=\"light\"",
        ThemeMode::Dark => "data-theme=\"dark\"",
        ThemeMode::System => "",
    };

    let is_group = ctx.current_peer.peer_type != PeerType::User;
    let grouping_ctxs = compute_item_grouping_contexts(ctx.items, is_group);

    let mut dialogs_html = String::with_capacity(ctx.all_peers.len() * 256);
    for peer in ctx.all_peers {
        let default_tid = if peer.is_forum {
            peer.topics.first().map(|t| t.topic_id)
        } else {
            None
        };
        let chat_url = ArchiveUrlBuilder::topic_chat_root_url(2, peer.peer_id, default_tid);
        let avatar_html = render_avatar_markup(
            Some(peer.peer_id),
            &peer.name,
            2,
            false,
            "dialog-avatar",
            ctx.available_avatars,
        );
        let active_cls = if peer.peer_id == ctx.current_peer.peer_id {
            " active"
        } else {
            ""
        };

        let _ = write!(
            dialogs_html,
            r#"<li class="dialog-item{active_cls}">
  <a href="{chat_url}" style="display: flex; gap: 0.75rem; width: 100%; color: inherit; text-decoration: none;">
    {avatar_html}
    <div class="dialog-info">
      <div class="dialog-top-row">
        <span class="dialog-name">{}</span>
      </div>
      <div class="dialog-snippet">{} messages</div>
    </div>
  </a>
</li>
"#,
            html_escape(&peer.name),
            peer.total_messages
        );
    }

    let mut topics_sidebar_html = String::new();
    if !ctx.topics.is_empty() {
        let mut topic_items_html = String::with_capacity(ctx.topics.len() * 256);

        for topic in ctx.topics {
            let topic_url = ArchiveUrlBuilder::topic_page_file_name(topic.topic_id, 0);
            let is_active = ctx
                .current_topic
                .map(|t| t.topic_id == topic.topic_id)
                .unwrap_or(false);
            let active_cls = if is_active { " active" } else { "" };
            let icon_html = if let Some(ref asset_rel) = topic.icon_asset {
                format!(
                    "<img src=\"{}\" alt=\"{}\" class=\"topic-icon-img\" loading=\"lazy\">",
                    html_escape(asset_rel),
                    html_escape(&topic.title),
                )
            } else if let Some(color) = topic.icon_color {
                let hex_color = format!("#{:06x}", color & 0xFFFFFF);
                format!("<span class=\"topic-icon\" style=\"color: {hex_color};\">#</span>")
            } else {
                "<span class=\"topic-icon\">#</span>".to_string()
            };

            let _ = write!(
                topic_items_html,
                r#"<li class="topic-item{active_cls}">
  <a href="{topic_url}" class="topic-link">
    {icon_html}
    <span class="topic-title">{}</span>
    <span class="topic-count">{}</span>
  </a>
</li>
"#,
                html_escape(&topic.title),
                topic.total_messages
            );
        }

        topics_sidebar_html = format!(
            r##"<aside class="topics-sidebar">
  <div class="topics-header">
    <span class="topics-heading">Topics</span>
    <button id="compact-topics-toggle" class="btn-icon" title="Toggle Compact View" aria-label="Toggle Compact View">
      <svg class="icon"><use href="#icon-list-compact"></use></svg>
    </button>
  </div>
  <ul class="topics-list">
    {topic_items_html}
  </ul>
</aside>
"##
        );
    }

    let mut messages_html = String::with_capacity(ctx.items.len() * 512);
    let mut last_date_day = None;

    for (idx, item) in ctx.items.iter().enumerate() {
        let item_date = match item {
            RenderItem::Message(m) => m.date,
            RenderItem::Album(a) => a.messages.first().map(|m| m.date).unwrap_or(0),
        };

        let days = item_date / 86400;
        if last_date_day != Some(days) {
            let (y, m, d) = days_to_ymd(days);
            let date_label = format!("{y:04}-{m:02}-{d:02}");
            let _ = writeln!(
                messages_html,
                "<div class=\"date-separator\" id=\"d-{date_label}\"><span class=\"date-separator-pill\">{date_label}</span></div>"
            );
            last_date_day = Some(days);
        }

        let g_ctx = grouping_ctxs.get(idx).copied().unwrap_or_default();
        messages_html.push_str(&render_chat_item(
            item,
            &g_ctx,
            ctx.presentation_mode,
            ctx.page_index,
            ctx.total_pages,
            ctx.available_avatars,
        ));
    }

    let prev_link = if ctx.page_index > 0 {
        let prev_file = if let Some(top) = ctx.current_topic {
            ArchiveUrlBuilder::topic_page_file_name(top.topic_id, ctx.page_index - 1)
        } else {
            ArchiveUrlBuilder::page_file_name(ctx.page_index - 1)
        };
        format!("<a href=\"{prev_file}\" class=\"btn-nav\">← Previous Page</a>")
    } else {
        "<span class=\"btn-nav disabled\">← Previous Page</span>".to_string()
    };

    let next_link = if ctx.page_index + 1 < ctx.total_pages {
        let next_file = if let Some(top) = ctx.current_topic {
            ArchiveUrlBuilder::topic_page_file_name(top.topic_id, ctx.page_index + 1)
        } else {
            ArchiveUrlBuilder::page_file_name(ctx.page_index + 1)
        };
        format!("<a href=\"{next_file}\" class=\"btn-nav\">Next Page →</a>")
    } else {
        "<span class=\"btn-nav disabled\">Next Page →</span>".to_string()
    };

    let cur_page_display = ctx.page_index + 1;
    let total_pages_display = ctx.total_pages.max(1);
    let escaped_name = html_escape(&ctx.current_peer.name);

    let display_title = if let Some(top) = ctx.current_topic {
        format!("{escaped_name} › {}", html_escape(&top.title))
    } else {
        escaped_name.clone()
    };

    let subtitle = if let Some(top) = ctx.current_topic {
        format!("{} messages", top.total_messages)
    } else if let Some(un) = &ctx.current_peer.username {
        format!(
            "@{} &bull; {} messages",
            html_escape(un),
            ctx.current_peer.total_messages
        )
    } else {
        format!("{} messages", ctx.current_peer.total_messages)
    };

    let username_row = if let Some(un) = &ctx.current_peer.username {
        format!(
            "<div class=\"info-row\"><strong>Username:</strong> @{}</div>",
            html_escape(un)
        )
    } else {
        String::new()
    };

    let phone_row = if let Some(ph) = &ctx.current_peer.phone {
        format!(
            "<div class=\"info-row\"><strong>Phone:</strong> {}</div>",
            html_escape(ph)
        )
    } else {
        String::new()
    };

    let date_menu = ctx.date_nav_html.unwrap_or_default();
    let header_avatar = render_avatar_markup(
        Some(ctx.current_peer.peer_id),
        &ctx.current_peer.name,
        2,
        false,
        "avatar",
        ctx.available_avatars,
    );
    let modal_avatar = render_avatar_markup(
        Some(ctx.current_peer.peer_id),
        &ctx.current_peer.name,
        2,
        true,
        "avatar",
        ctx.available_avatars,
    );

    let has_topics_cls = if !ctx.topics.is_empty() {
        " has-topics"
    } else {
        ""
    };

    format!(
        r##"<!DOCTYPE html>
<html lang="en" {theme_attr}>
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{escaped_name} - Telegram Archive</title>
  <script>
    (function() {{
      try {{
        var saved = localStorage.getItem('vendetta-theme');
        var theme = (saved === 'dark' || saved === 'light') ? saved : (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
        document.documentElement.setAttribute('data-theme', theme);
        if (document.documentElement.dataset) {{
          document.documentElement.dataset.theme = theme;
        }}
        var savedCompact = localStorage.getItem('vendetta-compact-topics');
        if (savedCompact === 'true') {{
          document.documentElement.setAttribute('data-compact-topics', 'true');
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
  <link rel="stylesheet" href="../../assets/css/theme.css">
  <link rel="stylesheet" href="../../assets/css/main.css">
  <link rel="stylesheet" href="../../assets/css/{mode_css}">
</head>
<body>
  <div style="display: none;">
    {SYMBOLS_SVG}
  </div>
  <div class="app-container{has_topics_cls}">
    <aside class="sidebar">
      <div class="sidebar-header">
        <a href="../../index.html" class="sidebar-title" style="color: inherit;">← All Chats</a>
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
    {topics_sidebar_html}

    <main class="chat-pane">
      <header class="chat-header">
        <div class="chat-title-info" title="Click to view chat info" style="display: flex; align-items: center; gap: 0.75rem; cursor: pointer;">
          {header_avatar}
          <div>
            <h2>{display_title}</h2>
            <div class="chat-subtitle">{subtitle}</div>
          </div>
        </div>
        <div class="chat-header-actions">
          {date_menu}
        </div>
      </header>

      <div class="chat-messages">
        {messages_html}
      </div>

      <footer class="pagination-footer">
        <div>{prev_link}</div>
        <div class="page-indicator">Page {cur_page_display} of {total_pages_display}</div>
        <div>{next_link}</div>
      </footer>
    </main>
  </div>

  <div id="chat-info-modal" class="modal-overlay">
    <div class="modal-card chat-info-card">
      <div class="chat-info-header">
        <h3>Chat Information</h3>
        <button class="chat-info-close" id="chat-info-close" aria-label="Close">&times;</button>
      </div>
      <div class="chat-info-body">
        <div class="chat-info-avatar">{modal_avatar}</div>
        <h2 class="chat-info-name">{escaped_name}</h2>
        <div class="chat-info-type badge-type">{}</div>
        <div class="chat-info-details">
          <div class="info-row"><strong>Canonical Peer ID:</strong> <code>{}</code></div>
          {username_row}
          {phone_row}
          <div class="info-row"><strong>Archived Messages:</strong> {}</div>
        </div>
      </div>
    </div>
  </div>

  <div id="search-modal" class="modal-overlay" data-base-path="../../">
    <div class="modal-card">
      <div class="search-header">
        <svg class="icon" style="align-self: center; color: var(--text-muted);"><use href="#icon-search"></use></svg>
        <input type="text" id="search-input" class="search-input" placeholder="Search messages..." autofocus>
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

  <script src="../../assets/js/app.js"></script>
  <script src="../../assets/js/lightbox.js"></script>
  <script src="../../assets/js/search.js"></script>
</body>
</html>"##,
        html_escape(ctx.current_peer.peer_type.as_ref()),
        ctx.current_peer.peer_id.raw(),
        ctx.current_peer.total_messages
    )
}
