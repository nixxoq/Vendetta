pub const THEME_CSS: &str = r##":root,
[data-theme="light"] {
  --font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
  --font-mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
  
  /* Light Theme Defaults */
  --bg-primary: #ffffff;
  --bg-secondary: #f4f4f5;
  --bg-sidebar: #f8f9fa;
  --bg-bubble-in: #ffffff;
  --bg-bubble-out: #effdde;
  --bg-system-event: rgba(0, 0, 0, 0.08);
  --bg-reply-preview: rgba(0, 0, 0, 0.04);
  --bg-spoiler: #e4e4e7;
  --bg-code: #f1f5f9;
  
  --text-primary: #18181b;
  --text-secondary: #71717a;
  --text-muted: #a1a1aa;
  --text-link: #2563eb;
  --text-bubble-out: #18181b;
  --accent-color: #3b82f6;
  --accent-hover: #2563eb;
  --border-color: #e4e4e7;
  
  --shadow-sm: 0 1px 2px 0 rgba(0, 0, 0, 0.05);
  --shadow-md: 0 4px 6px -1px rgba(0, 0, 0, 0.1), 0 2px 4px -1px rgba(0, 0, 0, 0.06);
  --shadow-bubble: 0 1px 2px rgba(16, 24, 40, 0.06);
  
  --badge-deleted-bg: #fee2e2;
  --badge-deleted-text: #b91c1c;
  --badge-edited-bg: #e0f2fe;
  --badge-edited-text: #0369a1;
  --bg-quote-fade-in: #ffffff;
  --bg-quote-fade-out: #effdde;

  --bg-reaction: rgba(0, 0, 0, 0.05);
  --border-reaction: rgba(0, 0, 0, 0.08);
  --bg-reaction-chosen: rgba(37, 99, 235, 0.12);
  --border-reaction-chosen: #2563eb;
  --bg-popover: #ffffff;
  --border-popover: #e4e4e7;
}

[data-theme="dark"] {
  --bg-primary: #18181b;
  --bg-secondary: #09090b;
  --bg-sidebar: #121215;
  --bg-bubble-in: #27272a;
  --bg-bubble-out: #2b5278;
  --bg-system-event: rgba(255, 255, 255, 0.12);
  --bg-reply-preview: rgba(255, 255, 255, 0.08);
  --bg-spoiler: #3f3f46;
  --bg-code: #1e293b;
  
  --text-primary: #fafafa;
  --text-secondary: #a1a1aa;
  --text-muted: #71717a;
  --text-link: #60a5fa;
  --text-bubble-out: #f8fafc;
  --accent-color: #60a5fa;
  --accent-hover: #93c5fd;
  --border-color: #27272a;
  
  --shadow-sm: 0 1px 2px 0 rgba(0, 0, 0, 0.5);
  --shadow-md: 0 4px 6px -1px rgba(0, 0, 0, 0.5);
  --shadow-bubble: 0 1px 3px rgba(0, 0, 0, 0.3);
  
  --badge-deleted-bg: #450a0a;
  --badge-deleted-text: #fca5a5;
  --badge-edited-bg: #082f49;
  --badge-edited-text: #7dd3fc;
  --bg-quote-fade-in: #27272a;
  --bg-quote-fade-out: #2b5278;

  --bg-reaction: rgba(255, 255, 255, 0.08);
  --border-reaction: rgba(255, 255, 255, 0.12);
  --bg-reaction-chosen: rgba(96, 165, 250, 0.22);
  --border-reaction-chosen: #60a5fa;
  --bg-popover: #1f1f23;
  --border-popover: #3f3f46;
}

@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]):not([data-theme="dark"]) {
    --bg-primary: #18181b;
    --bg-secondary: #09090b;
    --bg-sidebar: #121215;
    --bg-bubble-in: #27272a;
    --bg-bubble-out: #2b5278;
    --bg-system-event: rgba(255, 255, 255, 0.12);
    --bg-reply-preview: rgba(255, 255, 255, 0.08);
    --bg-spoiler: #3f3f46;
    --bg-code: #1e293b;
    
    --text-primary: #fafafa;
    --text-secondary: #a1a1aa;
    --text-muted: #71717a;
    --text-link: #60a5fa;
    --text-bubble-out: #f8fafc;
    --accent-color: #60a5fa;
    --accent-hover: #93c5fd;
    --border-color: #27272a;
    
    --shadow-sm: 0 1px 2px 0 rgba(0, 0, 0, 0.5);
    --shadow-md: 0 4px 6px -1px rgba(0, 0, 0, 0.5);
    --shadow-bubble: 0 1px 3px rgba(0, 0, 0, 0.3);
    
    --badge-deleted-bg: #450a0a;
    --badge-deleted-text: #fca5a5;
    --badge-edited-bg: #082f49;
    --badge-edited-text: #7dd3fc;
    --bg-quote-fade-in: #27272a;
    --bg-quote-fade-out: #2b5278;

    --bg-reaction: rgba(255, 255, 255, 0.08);
    --border-reaction: rgba(255, 255, 255, 0.12);
    --bg-reaction-chosen: rgba(96, 165, 250, 0.22);
    --border-reaction-chosen: #60a5fa;
    --bg-popover: #1f1f23;
    --border-popover: #3f3f46;
    --bg-quote-fade-out: #2b5278;
  }
}
"##;

pub const MAIN_CSS: &str = r##"* {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

body {
  font-family: var(--font-family);
  background-color: var(--bg-secondary);
  color: var(--text-primary);
  line-height: 1.5;
  -webkit-font-smoothing: antialiased;
}

a {
  color: var(--text-link);
  text-decoration: none;
}

a:hover {
  text-decoration: underline;
}

.icon {
  width: 1.25rem;
  height: 1.25rem;
  display: inline-block;
  vertical-align: middle;
}

.app-container {
  display: flex;
  height: 100vh;
  overflow: hidden;
}

/* Sidebar */
.sidebar {
  width: 320px;
  background-color: var(--bg-sidebar);
  border-right: 1px solid var(--border-color);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
}

.sidebar-header {
  padding: 1rem;
  border-bottom: 1px solid var(--border-color);
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.sidebar-title {
  font-size: 1.125rem;
  font-weight: 600;
}

.sidebar-tools {
  display: flex;
  gap: 0.5rem;
}

.btn-icon {
  background: transparent;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  padding: 0.375rem;
  cursor: pointer;
  color: var(--text-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.btn-icon:hover {
  background: var(--bg-secondary);
  color: var(--text-primary);
}

.dialogs-list {
  flex: 1;
  overflow-y: auto;
  list-style: none;
}

.dialog-item {
  display: flex;
  padding: 0.75rem 1rem;
  border-bottom: 1px solid var(--border-color);
  gap: 0.75rem;
  align-items: center;
  transition: background-color 0.15s;
}

.dialog-item:hover, .dialog-item.active {
  background-color: var(--bg-primary);
}

.dialog-avatar {
  width: 44px;
  height: 44px;
  border-radius: 50%;
  background: var(--accent-color);
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  font-weight: 600;
  font-size: 1.125rem;
  flex-shrink: 0;
  overflow: hidden;
}

.dialog-info {
  flex: 1;
  min-width: 0;
}

.dialog-top-row {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
}

.dialog-name {
  font-weight: 600;
  font-size: 0.9375rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dialog-time {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.dialog-snippet {
  font-size: 0.8125rem;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* Topics */
.topics-sidebar {
  width: 260px;
  background: var(--bg-secondary);
  border-right: 1px solid var(--border-color);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  overflow: hidden;
}

.topics-header {
  padding: 0.875rem 1rem;
  border-bottom: 1px solid var(--border-color);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
}

.topics-heading {
  font-weight: 700;
  font-size: 0.9375rem;
  color: var(--text-primary);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.topics-list {
  flex: 1;
  overflow-y: auto;
  list-style: none;
  padding: 0.5rem;
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.topic-item {
  border-radius: 6px;
  transition: background-color 0.15s ease;
}

.topic-link {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.375rem 0.625rem;
  color: inherit;
  text-decoration: none;
  border-radius: 6px;
  min-height: 38px;
  box-sizing: border-box;
}

.topic-item:hover {
  background-color: rgba(0, 0, 0, 0.04);
}

[data-theme="dark"] .topic-item:hover {
  background-color: rgba(255, 255, 255, 0.04);
}

.topic-item.active {
  background-color: var(--bg-primary);
  box-shadow: var(--shadow-sm);
}

.topic-icon {
  font-size: 0.9375rem;
  font-weight: 800;
  color: var(--accent-color);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 1.375rem;
  height: 1.375rem;
  border-radius: 4px;
  background: var(--bg-system-event);
  flex-shrink: 0;
  overflow: hidden;
}

.topic-icon-img {
  width: 1.375rem;
  height: 1.375rem;
  border-radius: 4px;
  object-fit: contain;
  flex-shrink: 0;
}

.topic-title {
  font-weight: 500;
  font-size: 0.875rem;
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  color: var(--text-primary);
}

.topic-count {
  font-size: 0.75rem;
  background: var(--bg-system-event);
  border: 1px solid transparent;
  padding: 0.125rem 0.375rem;
  border-radius: 10px;
  color: var(--text-secondary);
  font-weight: 500;
  flex-shrink: 0;
}

.topic-item.active .topic-count {
  background: var(--accent-color);
  color: #fff;
}

/* Compact mode (for topics) */
[data-compact-topics="true"] .topics-list,
.topics-sidebar.compact-mode .topics-list {
  padding: 0.375rem;
  gap: 1px;
}

[data-compact-topics="true"] .topic-link,
.topics-sidebar.compact-mode .topic-link {
  padding: 0.25rem 0.5rem;
  min-height: 28px;
  gap: 0.375rem;
}

[data-compact-topics="true"] .topic-icon,
.topics-sidebar.compact-mode .topic-icon {
  width: 1.125rem;
  height: 1.125rem;
  font-size: 0.75rem;
  border-radius: 3px;
}

[data-compact-topics="true"] .topic-icon-img,
.topics-sidebar.compact-mode .topic-icon-img {
  width: 1.125rem;
  height: 1.125rem;
}

[data-compact-topics="true"] .topic-title,
.topics-sidebar.compact-mode .topic-title {
  font-size: 0.8125rem;
}

[data-compact-topics="true"] .topic-count,
.topics-sidebar.compact-mode .topic-count {
  font-size: 0.6875rem;
  padding: 0.0625rem 0.25rem;
  border-radius: 6px;
}

/* Chat Pane */
.chat-pane {
  flex: 1;
  display: flex;
  flex-direction: column;
  background-color: var(--bg-secondary);
  min-width: 0;
}

.chat-header {
  height: 60px;
  background-color: var(--bg-primary);
  border-bottom: 1px solid var(--border-color);
  padding: 0 1.25rem;
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-shrink: 0;
}

.chat-title-info h2 {
  font-size: 1.125rem;
  font-weight: 600;
}

.chat-subtitle {
  font-size: 0.75rem;
  color: var(--text-secondary);
}

.chat-header-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

/* Date Navigation Dropdown & Jump Menu */
.date-nav-dropdown {
  position: relative;
  display: inline-block;
}

.date-nav-dropdown summary {
  list-style: none;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
}

.date-nav-dropdown summary::-webkit-details-marker {
  display: none;
}

.date-nav-menu {
  position: absolute;
  top: calc(100% + 8px);
  right: 0;
  width: 290px;
  max-height: 420px;
  overflow-y: auto;
  background: var(--bg-card);
  border: 1px solid var(--border-color);
  border-radius: 12px;
  box-shadow: var(--shadow-lg);
  padding: 0.875rem 1rem;
  z-index: 1000;
}

.date-nav-title {
  font-size: 0.875rem;
  font-weight: 700;
  color: var(--text-primary);
  margin-bottom: 0.75rem;
  padding-bottom: 0.375rem;
  border-bottom: 1px solid var(--border-color);
}

.date-nav-list {
  list-style: none;
  padding: 0;
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.date-year-group {
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
}

.date-month-heading {
  font-size: 0.75rem;
  font-weight: 700;
  color: var(--accent-color);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.date-days-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.date-jump-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 28px;
  height: 24px;
  padding: 0 4px;
  font-size: 0.75rem;
  font-weight: 500;
  border-radius: 4px;
  background: var(--bg-system-event);
  color: var(--text-primary);
  text-decoration: none;
  transition: background 0.15s ease, color 0.15s ease;
}

.date-jump-btn:hover {
  background: var(--accent-color);
  color: #fff;
}

.chat-messages {
  flex: 1;
  overflow-y: auto;
  padding: 1rem 1.5rem;
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
}

/* Date Separator */
.date-separator {
  align-self: center;
  text-align: center;
  margin: 1rem 0 0.5rem 0;
  position: static;
  z-index: 1;
}

.date-separator-pill {
  display: inline-block;
  background: var(--bg-system-event);
  backdrop-filter: blur(8px);
  color: var(--text-secondary);
  font-size: 0.75rem;
  font-weight: 600;
  padding: 0.25rem 0.75rem;
  border-radius: 12px;
  box-shadow: var(--shadow-sm);
}

/* System Event */
.system-event {
  align-self: center;
  text-align: center;
  margin: 0.5rem 0;
}

.system-event-bubble {
  background: var(--bg-system-event);
  color: var(--text-secondary);
  font-size: 0.8125rem;
  padding: 0.25rem 0.75rem;
  border-radius: 8px;
}

/* Pagination Footer */
.pagination-footer {
  background: var(--bg-primary);
  border-top: 1px solid var(--border-color);
  padding: 0.75rem 1.25rem;
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.btn-nav {
  padding: 0.375rem 0.75rem;
  background: var(--bg-secondary);
  border: 1px solid var(--border-color);
  border-radius: 6px;
  font-size: 0.875rem;
  font-weight: 500;
  color: var(--text-primary);
}

.btn-nav:hover {
  background: var(--border-color);
  text-decoration: none;
}

.btn-nav.disabled {
  opacity: 0.4;
  pointer-events: none;
}

/* Search Modal */
.modal-overlay {
  display: none;
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  backdrop-filter: blur(4px);
  z-index: 100;
  align-items: center;
  justify-content: center;
}

.modal-overlay.open {
  display: flex;
}

.modal-card {
  width: 90%;
  max-width: 650px;
  max-height: 80vh;
  background: var(--bg-primary);
  border-radius: 12px;
  border: 1px solid var(--border-color);
  box-shadow: var(--shadow-md);
  display: flex;
  flex-direction: column;
}

.search-header {
  padding: 1rem;
  border-bottom: 1px solid var(--border-color);
  display: flex;
  gap: 0.5rem;
}

.search-input {
  flex: 1;
  padding: 0.625rem 0.875rem;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--bg-secondary);
  color: var(--text-primary);
  font-size: 0.9375rem;
  outline: none;
}

.search-filters {
  padding: 0.5rem 1rem;
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  border-bottom: 1px solid var(--border-color);
  background: var(--bg-sidebar);
  align-items: center;
}

.filter-select {
  padding: 0.25rem 0.5rem;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--bg-primary);
  color: var(--text-primary);
  font-size: 0.8125rem;
}

.filter-input {
  padding: 0.25rem 0.5rem;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--bg-primary);
  color: var(--text-primary);
  font-size: 0.8125rem;
  max-width: 120px;
}

.filter-checkbox-label {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  font-size: 0.75rem;
  color: var(--text-secondary);
  cursor: pointer;
  user-select: none;
}

.dense-media-link {
  color: var(--accent-color);
  font-weight: 500;
  text-decoration: none;
}
.dense-media-link:hover {
  text-decoration: underline;
}
.dense-media-unavailable {
  color: var(--text-muted);
  font-style: italic;
}

.search-results {
  flex: 1;
  overflow-y: auto;
  padding: 0.5rem;
  list-style: none;
}

.search-result-item {
  padding: 0.75rem;
  border-radius: 8px;
  transition: background 0.15s;
}

.search-highlight {
  background-color: #fef08a;
  color: #854d0e;
  padding: 0 2px;
  border-radius: 2px;
  font-weight: 600;
}

[data-theme="dark"] .search-highlight {
  background-color: #854d0e;
  color: #fef08a;
}

.search-res-text {
  font-size: 0.875rem;
  color: var(--text-primary);
  line-height: 1.4;
}

.chat-title-info {
  cursor: pointer;
  transition: opacity 0.15s;
}

.chat-title-info:hover {
  opacity: 0.85;
}

.chat-info-card {
  padding: 1.5rem;
  width: 90%;
  max-width: 480px;
}

.chat-info-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
}

.chat-info-close {
  background: transparent;
  border: none;
  font-size: 1.5rem;
  cursor: pointer;
  color: var(--text-secondary);
}

.chat-info-avatar {
  width: 72px;
  height: 72px;
  border-radius: 50%;
  margin: 0 auto 0.75rem;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 2rem;
  font-weight: 600;
  color: #fff;
  background: var(--accent-color);
}

.chat-info-name {
  text-align: center;
  font-size: 1.25rem;
  margin-bottom: 0.25rem;
}

.chat-info-type {
  text-align: center;
  display: block;
  margin: 0 auto 1rem;
  width: fit-content;
}

.chat-info-details {
  background: var(--bg-secondary);
  border-radius: 8px;
  padding: 0.75rem 1rem;
  margin-bottom: 1rem;
  font-size: 0.875rem;
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
}

.chat-info-media-section h4 {
  font-size: 0.875rem;
  color: var(--text-secondary);
  margin-bottom: 0.5rem;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.media-category-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.5rem;
  font-size: 0.8125rem;
}

.media-cat-item {
  background: var(--bg-secondary);
  padding: 0.5rem 0.75rem;
  border-radius: 6px;
}

@keyframes targetPulse {
  0% { background-color: var(--bg-system-event); }
  50% { background-color: rgba(59, 130, 246, 0.25); }
  100% { background-color: transparent; }
}

.highlight-target {
  animation: targetPulse 2.5s ease-out;
  border-radius: 8px;
}

/* Lightbox Modal */
.lightbox-modal {
  display: none;
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.9);
  z-index: 200;
  align-items: center;
  justify-content: center;
}

.lightbox-modal.open {
  display: flex;
}

.lightbox-img {
  max-width: 90vw;
  max-height: 90vh;
  border-radius: 6px;
  box-shadow: var(--shadow-md);
}

.lightbox-close {
  position: absolute;
  top: 1rem;
  right: 1.5rem;
  color: #fff;
  font-size: 2rem;
  cursor: pointer;
}

@media (max-width: 768px) {
  .app-container {
    flex-direction: column;
  }
  .sidebar {
    width: 100%;
    height: 240px;
  }
  .topics-sidebar {
    width: 100%;
    height: auto;
    max-height: 180px;
    border-right: none;
    border-bottom: 1px solid var(--border-color);
  }
  .topics-list {
    padding: 0.375rem;
    gap: 1px;
  }
  .topic-link {
    padding: 0.25rem 0.5rem;
    min-height: 28px;
    gap: 0.375rem;
  }
  .topic-icon {
    width: 1.125rem;
    height: 1.125rem;
    font-size: 0.75rem;
    border-radius: 3px;
  }
  .topic-icon-img {
    width: 1.125rem;
    height: 1.125rem;
  }
  .topic-title {
    font-size: 0.8125rem;
  }
  .topic-count {
    font-size: 0.6875rem;
    padding: 0.0625rem 0.25rem;
    border-radius: 6px;
  }
  .chat-messages {
    padding: 1rem;
  }
}
"##;

pub const TELEGRAM_LIKE_CSS: &str = r##"/* Message Rows */
.message-row {
  display: flex;
  gap: 0.5rem;
  align-items: flex-end;
  margin-bottom: 0;
  position: relative;
}

.album-sub-anchor {
  position: absolute;
  width: 0;
  height: 0;
  margin: 0;
  padding: 0;
  pointer-events: none;
  visibility: hidden;
}

.message-row:target .message-bubble {
  animation: highlight-pulse 2s ease-out;
}

@keyframes highlight-pulse {
  0% { transform: scale(1.02); box-shadow: 0 0 0 4px var(--accent-color); }
  100% { transform: scale(1); box-shadow: var(--shadow-bubble); }
}

.msg-incoming {
  align-self: flex-start;
}

.msg-outgoing {
  align-self: flex-end;
  flex-direction: row-reverse;
}

.avatar {
  width: 34px;
  height: 34px;
  border-radius: 50%;
  background: var(--accent-color);
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 0.875rem;
  font-weight: 600;
  flex-shrink: 0;
  overflow: hidden;
}

.avatar-lg {
  width: 72px;
  height: 72px;
  font-size: 1.75rem;
}

.avatar-img {
  width: 100%;
  height: 100%;
  border-radius: 50%;
  object-fit: cover;
  display: block;
}

.avatar-placeholder {
  visibility: hidden;
}

.message-bubble {
  max-width: 620px;
  padding: 0.5rem 0.75rem;
  border-radius: 12px;
  box-shadow: var(--shadow-bubble);
  word-break: break-word;
  position: relative;
}

.msg-incoming .message-bubble {
  background-color: var(--bg-bubble-in);
  color: var(--text-primary);
  border-bottom-left-radius: 4px;
}

.msg-outgoing .message-bubble {
  background-color: var(--bg-bubble-out);
  color: var(--text-bubble-out);
  border-bottom-right-radius: 4px;
}

.message-sender {
  font-size: 0.8125rem;
  font-weight: 600;
  color: var(--accent-color);
  margin-bottom: 0.25rem;
}

.message-text {
  font-size: 0.9375rem;
  line-height: 1.45;
}

/* Telegram-like Embedded Reply / Quote Block */
.msg-reply-preview,
.reply-card {
  display: flex;
  align-items: stretch;
  background: var(--bg-reply-preview);
  border-radius: 4px 8px 8px 4px;
  padding: 0;
  margin-bottom: 0.375rem;
  font-size: 0.8125rem;
  text-decoration: none;
  cursor: pointer;
  overflow: hidden;
  position: relative;
  transition: background-color 0.15s ease;
  user-select: none;
  max-width: 100%;
}

.msg-reply-preview:hover,
.reply-card:hover {
  background: rgba(0, 0, 0, 0.08);
  text-decoration: none;
}

[data-theme="dark"] .msg-reply-preview:hover,
[data-theme="dark"] .reply-card:hover {
  background: rgba(255, 255, 255, 0.12);
}

.reply-accent-bar {
  width: 3px;
  background-color: var(--accent-color);
  flex-shrink: 0;
  border-radius: 3px 0 0 3px;
}

.msg-outgoing .reply-accent-bar {
  background-color: var(--accent-hover, var(--accent-color));
}

.reply-content {
  padding: 0.25rem 0.5rem;
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 2px;
}

.reply-header {
  display: flex;
  align-items: center;
  gap: 0.375rem;
}

.reply-sender {
  font-weight: 600;
  font-size: 0.8125rem;
  color: var(--accent-color);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.msg-outgoing .reply-sender {
  color: var(--accent-hover, var(--accent-color));
}

.reply-body {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  font-size: 0.8125rem;
  color: var(--text-secondary);
  line-height: 1.3;
}

.msg-outgoing .reply-body {
  color: var(--text-secondary);
}

.reply-snippet {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.reply-badge-deleted {
  background: var(--badge-deleted-bg);
  color: var(--badge-deleted-text);
  font-size: 0.6875rem;
  font-weight: 600;
  padding: 1px 4px;
  border-radius: 3px;
}

.reply-badge-missing,
.reply-badge-inaccessible {
  background: var(--bg-system-event);
  color: var(--text-muted);
  font-size: 0.6875rem;
  font-weight: 600;
  padding: 1px 4px;
  border-radius: 3px;
}

.reply-unlinked {
  cursor: default;
  opacity: 0.9;
}

/* State Badges */
.state-badge {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  font-size: 0.75rem;
  font-weight: 600;
  padding: 0.125rem 0.5rem;
  border-radius: 4px;
  margin-bottom: 0.25rem;
}

.badge-deleted {
  background: var(--badge-deleted-bg);
  color: var(--badge-deleted-text);
  border: 1px dashed var(--badge-deleted-text);
}

.badge-empty, .badge-inaccessible {
  background: var(--bg-secondary);
  color: var(--text-secondary);
}

/* Forward Header */
.forward-header {
  font-size: 0.8125rem;
  color: var(--text-secondary);
  margin-bottom: 0.375rem;
  padding-left: 0.5rem;
  border-left: 2px solid var(--accent-color, #3390ec);
  display: flex;
  flex-direction: column;
  gap: 0.125rem;
}

.forward-header .fwd-top {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.25rem;
}

.forward-header .fwd-avatar {
  width: 1.125rem;
  height: 1.125rem;
  min-width: 1.125rem;
  min-height: 1.125rem;
  border-radius: 50%;
  overflow: hidden;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-secondary);
  font-size: 0.625rem;
  font-weight: 600;
  vertical-align: middle;
}

.forward-header .fwd-avatar .avatar-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.forward-header .fwd-origin {
  font-weight: 600;
  color: var(--accent-color);
  text-decoration: none;
}

.forward-header a.fwd-origin:hover {
  text-decoration: underline;
}

.forward-header .fwd-username {
  color: var(--text-secondary);
  font-size: 0.75rem;
}

.forward-header .fwd-sig {
  color: var(--text-secondary);
  font-style: italic;
}

.forward-header .fwd-meta {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.375rem;
  font-size: 0.6875rem;
  color: var(--text-muted);
  margin-top: 0.0625rem;
}

.forward-header .fwd-id,
.forward-header .fwd-msg-id {
  background: var(--bg-secondary);
  padding: 0.0625rem 0.25rem;
  border-radius: 3px;
  border: 1px solid var(--border-color);
  font-family: var(--font-mono, monospace);
}

/* Edit History */
.edit-history {
  margin-top: 0.375rem;
  font-size: 0.8125rem;
  border-top: 1px solid var(--border-color);
  padding-top: 0.25rem;
}

.edit-history summary {
  cursor: pointer;
  color: var(--accent-color);
  font-weight: 500;
}

.revision-timeline {
  padding-top: 0.375rem;
  display: flex;
  flex-direction: column;
  gap: 0.375rem;
}

.revision-entry {
  background: var(--bg-secondary);
  border-radius: 6px;
  padding: 0.375rem 0.5rem;
}

.revision-meta {
  font-size: 0.75rem;
  color: var(--text-secondary);
  margin-bottom: 0.125rem;
}

/* Message Meta (Footer) */
.message-meta {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 0.375rem;
  font-size: 0.6875rem;
  color: var(--text-muted);
  margin-top: 0.25rem;
}

.meta-edited {
  font-style: italic;
}

/* Rich text tags */
.tg-spoiler,
.spoiler {
  background: var(--bg-spoiler);
  color: transparent;
  border-radius: 3px;
  cursor: pointer;
  transition: color 0.2s, background 0.2s;
}

.tg-spoiler:hover, .tg-spoiler.revealed,
.spoiler:hover, .spoiler.revealed {
  color: inherit;
  background: transparent;
}

.inline-code {
  font-family: var(--font-mono);
  background: var(--bg-code);
  padding: 0.125rem 0.25rem;
  border-radius: 4px;
  font-size: 0.875em;
}

.code-block {
  font-family: var(--font-mono);
  background: var(--bg-code);
  padding: 0.5rem;
  border-radius: 6px;
  display: block;
  overflow-x: auto;
  font-size: 0.875rem;
  margin: 0.375rem 0;
}

/* Telegram-like Blockquote Entity */
blockquote,
.tg-blockquote {
  margin: 0.375rem 0;
  padding: 0.375rem 0.625rem;
  background-color: var(--bg-reply-preview);
  border-left: 3px solid var(--accent-color);
  border-radius: 0 6px 6px 0;
  font-size: 0.9375rem;
  line-height: 1.45;
  color: var(--text-primary);
  position: relative;
  word-break: break-word;
  display: block;
}

.msg-outgoing blockquote,
.msg-outgoing .tg-blockquote {
  border-left-color: var(--accent-hover, var(--accent-color));
  background-color: rgba(0, 0, 0, 0.05);
}

[data-theme="dark"] .msg-outgoing blockquote,
[data-theme="dark"] .msg-outgoing .tg-blockquote {
  background-color: rgba(255, 255, 255, 0.1);
}

.tg-blockquote-collapsed {
  max-height: 130px;
  overflow: hidden;
  cursor: pointer;
  position: relative;
  padding-bottom: 28px !important;
  transition: max-height 0.25s ease-out;
}

.tg-blockquote-collapsed::after {
  content: "▾ Expand quote";
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  height: 64px;
  background: linear-gradient(
    to bottom,
    rgba(0, 0, 0, 0) 0%,
    var(--bg-quote-fade-in) 45%,
    var(--bg-quote-fade-in) 100%
  );
  display: flex;
  align-items: flex-end;
  justify-content: center;
  padding-bottom: 5px;
  font-size: 0.8125rem;
  font-weight: 600;
  color: var(--accent-color);
  pointer-events: none;
}

.msg-outgoing .tg-blockquote-collapsed::after {
  background: linear-gradient(
    to bottom,
    rgba(0, 0, 0, 0) 0%,
    var(--bg-quote-fade-out) 45%,
    var(--bg-quote-fade-out) 100%
  );
  color: var(--accent-hover, var(--accent-color));
}

.tg-blockquote-collapsed.expanded {
  max-height: none;
  padding-bottom: 0.375rem !important;
  cursor: pointer;
}

.tg-blockquote-collapsed.expanded::after {
  display: none;
}

/* Media Cards */
.media-photo img {
  max-width: 100%;
  max-height: 420px;
  border-radius: 8px;
  display: block;
}

.media-video video {
  max-width: 100%;
  max-height: 420px;
  border-radius: 8px;
}

.media-document {
  display: flex;
  gap: 0.75rem;
  background: var(--bg-secondary);
  border-radius: 8px;
  padding: 0.5rem 0.75rem;
  align-items: center;
}

.media-album {
  max-width: 460px;
  margin-bottom: 0.375rem;
}

.media-album .album-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 4px;
  border-radius: 10px;
  overflow: hidden;
}

.album-count-1 .album-grid {
  grid-template-columns: 1fr;
  max-width: 380px;
}

.album-count-2 .album-grid {
  grid-template-columns: repeat(2, 1fr);
}

.album-count-3 .album-grid {
  grid-template-columns: repeat(2, 1fr);
}

.album-count-3 .album-item:first-child {
  grid-column: span 2;
}

.album-count-4 .album-grid {
  grid-template-columns: repeat(2, 1fr);
}

.album-count-5 .album-grid,
.album-count-6 .album-grid {
  grid-template-columns: repeat(3, 1fr);
}

.album-item {
  position: relative;
  overflow: hidden;
  border-radius: 4px;
  background: var(--bg-system-event);
}

.album-item img {
  width: 100%;
  height: 180px;
  object-fit: cover;
  display: block;
}

.album-item video {
  width: 100%;
  height: 180px;
  object-fit: cover;
  display: block;
}

.album-count-1 .album-item img,
.album-count-1 .album-item video {
  height: auto;
  max-height: 420px;
}

.album-continuation-badge {
  font-size: 0.75rem;
  margin: 6px 0;
  text-align: center;
}

.album-continuation-badge a.continuation-link {
  display: inline-block;
  background: var(--bg-system-event);
  color: var(--accent-primary);
  padding: 3px 10px;
  border-radius: 12px;
  text-decoration: none;
  font-weight: 600;
  transition: opacity 0.15s ease;
}

.album-continuation-badge a.continuation-link:hover {
  background: var(--accent-primary);
  color: #ffffff;
}

/* Rendering FUCKING TELEGRAM Sticker(s) */
.msg-sticker .message-bubble {
  background: transparent !important;
  box-shadow: none !important;
  padding: 0 !important;
}

.media-sticker {
  display: inline-block;
  max-width: 200px;
  max-height: 200px;
}

.media-sticker .sticker-img,
.media-sticker .sticker-video,
.media-sticker .sticker-canvas {
  max-width: 200px;
  max-height: 200px;
  width: auto;
  height: auto;
  object-fit: contain;
  display: block;
}

.media-sticker .sticker-fallback {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  padding: 0.5rem 0.75rem;
  border-radius: 8px;
  background: var(--bg-secondary);
  color: var(--text-secondary);
  font-size: 0.8125rem;
}

/* Channel Post & Comments Presentation */
// currently placeholder, soon i'll implement dumping comments from post too :)
.channel-post {
  width: 100%;
}

.channel-post .message-bubble {
  max-width: 640px;
  width: auto;
}

.channel-comments-bar {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 0.75rem;
  margin-top: 0.375rem;
  margin-left: -0.75rem;
  margin-right: -0.75rem;
  margin-bottom: -0.5rem;
  border-top: 1px solid var(--border-color);
  border-bottom-left-radius: inherit;
  border-bottom-right-radius: inherit;
  color: var(--accent-color);
  font-size: 0.875rem;
  font-weight: 500;
  cursor: default;
  user-select: none;
  background: rgba(0, 0, 0, 0.02);
}

[data-theme="dark"] .channel-comments-bar {
  background: rgba(255, 255, 255, 0.02);
}

.channel-comments-bar .comments-icon {
  font-size: 1rem;
}

.channel-comments-bar .comments-label {
  color: inherit;
}

/* Reaction Badges & Popover */
.message-reactions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 6px;
  margin-bottom: 2px;
  position: relative;
}

.reaction-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  background: var(--bg-reaction);
  border: 1px solid var(--border-reaction);
  border-radius: 14px;
  padding: 2px 8px;
  font-size: 0.8125rem;
  line-height: 1.2;
  cursor: pointer;
  user-select: none;
  position: relative;
  transition: background 0.15s ease, border-color 0.15s ease, transform 0.1s ease;
}

.reaction-badge:hover,
.reaction-badge:focus-visible,
.reaction-badge.popover-open {
  background: var(--bg-secondary);
  border-color: var(--accent-color);
  outline: none;
}

.reaction-badge.reaction-chosen {
  background: var(--bg-reaction-chosen);
  border-color: var(--border-reaction-chosen);
  font-weight: 600;
}

.reaction-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 1rem;
}

.reaction-emoji {
  display: inline-block;
  line-height: 1;
  font-family: "Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", "Android Emoji", "EmojiSymbols", sans-serif;
  font-variant-emoji: emoji;
}

.reaction-custom-icon {
  width: 18px;
  height: 18px;
  object-fit: contain;
  vertical-align: middle;
  display: inline-block;
}

.reaction-custom-fallback {
  display: inline-block;
  font-size: 0.875rem;
}

.reaction-count {
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text-primary);
}

/* Reactor List Popover */
.reaction-popover {
  display: none;
  position: absolute;
  bottom: calc(100% + 6px);
  left: 0;
  z-index: 120;
  min-width: 180px;
  max-width: 260px;
  background: var(--bg-popover);
  border: 1px solid var(--border-popover);
  border-radius: 8px;
  box-shadow: var(--shadow-md);
  padding: 6px 0;
  text-align: left;
  white-space: normal;
  pointer-events: auto;
}

.msg-outgoing .reaction-popover {
  left: auto;
  right: 0;
}

.reaction-badge:hover .reaction-popover,
.reaction-badge:focus-within .reaction-popover,
.reaction-badge.popover-open .reaction-popover {
  display: block;
}

.reaction-popover-header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px 6px;
  border-bottom: 1px solid var(--border-popover);
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--text-secondary);
}

.reaction-popover-title {
  text-transform: capitalize;
}

.reaction-popover-empty {
  padding: 8px 10px;
  font-size: 0.75rem;
  color: var(--text-muted);
}

.reaction-reactor-list {
  list-style: none;
  margin: 0;
  padding: 4px 0 0;
  max-height: 180px;
  overflow-y: auto;
}

.reaction-reactor-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 10px;
  transition: background 0.1s ease;
}

.reaction-reactor-item:hover {
  background: var(--bg-secondary);
}

.reactor-avatar {
  width: 24px;
  height: 24px;
  border-radius: 50%;
  overflow: hidden;
  flex-shrink: 0;
}

.reactor-avatar .avatar,
.reactor-avatar .avatar-img,
.reactor-avatar .avatar-text {
  width: 24px;
  height: 24px;
  font-size: 0.65rem;
  line-height: 24px;
}

.reactor-info {
  display: flex;
  flex-direction: column;
  min-width: 0;
  flex: 1;
}

.reactor-name {
  font-size: 0.75rem;
  font-weight: 500;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.reactor-username {
  font-size: 0.6875rem;
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.reaction-reactor-more {
  padding: 4px 10px 2px;
  font-size: 0.6875rem;
  color: var(--text-muted);
  text-align: center;
  font-weight: 500;
}
"##;

pub const ARCHIVE_DENSE_CSS: &str = r##".dense-row {
  display: flex;
  gap: 0.75rem;
  padding: 0.375rem 0.5rem;
  border-bottom: 1px solid var(--border-color);
  font-size: 0.875rem;
  align-items: baseline;
}

.dense-row:hover {
  background: var(--bg-primary);
}

.dense-row:target {
  background: var(--bg-bubble-out);
}

.dense-time {
  font-family: var(--font-mono);
  font-size: 0.75rem;
  color: var(--text-muted);
  flex-shrink: 0;
  width: 50px;
}

.dense-sender {
  flex-shrink: 0;
  width: 120px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  color: var(--accent-color);
}

.dense-body {
  flex: 1;
  min-width: 0;
}

.dense-reply-pill {
  font-size: 0.75rem;
  background: var(--bg-secondary);
  padding: 0.125rem 0.375rem;
  border-radius: 4px;
  color: var(--text-secondary);
}

.dense-media-badge {
  font-size: 0.75rem;
  color: var(--text-link);
  font-weight: 500;
}

.dense-edited-badge {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.dense-reaction-pill {
  font-size: 0.75rem;
  background: var(--bg-secondary);
  padding: 0.125rem 0.375rem;
  border-radius: 4px;
  color: var(--text-primary);
  margin-left: 2px;
}
"##;
