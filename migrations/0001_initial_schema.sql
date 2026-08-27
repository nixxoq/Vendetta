-- Initial SQLite Schema for Vendetta Telegram Archive
-- Canonical Message Key is (peer_id, message_id)

CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY,
    applied_at INTEGER NOT NULL,
    description TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS peers (
    peer_id INTEGER PRIMARY KEY,
    peer_type TEXT NOT NULL, -- 'user', 'group', 'channel'
    name TEXT,
    username TEXT,
    phone TEXT,
    raw_tl BLOB,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    peer_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    date INTEGER NOT NULL,
    sender_id INTEGER,
    text TEXT,
    entities_json TEXT,
    edit_date INTEGER,
    state TEXT NOT NULL DEFAULT 'active', -- 'active', 'edited', 'deleted'
    reply_to_msg_id INTEGER,
    reply_to_top_id INTEGER,
    reply_to_peer_id INTEGER,
    grouped_id INTEGER,
    forward_json TEXT,
    reactions_json TEXT,
    views INTEGER,
    forwards_count INTEGER,
    raw_tl BLOB,
    PRIMARY KEY (peer_id, message_id)
);

CREATE INDEX IF NOT EXISTS idx_messages_peer_date ON messages(peer_id, date);
CREATE INDEX IF NOT EXISTS idx_messages_grouped_id ON messages(peer_id, grouped_id) WHERE grouped_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_messages_sender ON messages(sender_id) WHERE sender_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS message_revisions (
    revision_id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    captured_at INTEGER NOT NULL,
    edit_date INTEGER,
    text TEXT,
    entities_json TEXT,
    raw_tl BLOB,
    FOREIGN KEY (peer_id, message_id) REFERENCES messages(peer_id, message_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_revisions_msg ON message_revisions(peer_id, message_id);

CREATE TABLE IF NOT EXISTS message_replies (
    source_peer_id INTEGER NOT NULL,
    source_message_id INTEGER NOT NULL,
    target_peer_id INTEGER NOT NULL,
    target_message_id INTEGER NOT NULL,
    top_message_id INTEGER,
    resolution_status TEXT NOT NULL, -- 'resolved', 'context_only', 'missing', 'deleted', 'inaccessible', 'not_requested'
    PRIMARY KEY (source_peer_id, source_message_id)
);

CREATE INDEX IF NOT EXISTS idx_replies_target ON message_replies(target_peer_id, target_message_id);

CREATE TABLE IF NOT EXISTS media (
    media_key TEXT PRIMARY KEY,
    mime_type TEXT,
    size_bytes INTEGER,
    local_rel_path TEXT,
    sha256 TEXT,
    download_status TEXT NOT NULL DEFAULT 'pending', -- 'pending', 'downloading', 'completed', 'failed', 'skipped'
    downloaded_bytes INTEGER NOT NULL DEFAULT 0,
    verified INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS message_media (
    peer_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    media_key TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (peer_id, message_id, media_key),
    FOREIGN KEY (media_key) REFERENCES media(media_key)
);

CREATE TABLE IF NOT EXISTS sync_state (
    peer_id INTEGER PRIMARY KEY,
    pts INTEGER,
    qts INTEGER,
    date INTEGER,
    seq INTEGER,
    min_message_id INTEGER,
    max_message_id INTEGER,
    last_synced_at INTEGER NOT NULL
);
