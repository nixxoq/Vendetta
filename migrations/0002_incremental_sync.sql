-- Migration 0002: Incremental Synchronization & Difference Updates Schema

CREATE TABLE IF NOT EXISTS account_sync_state (
    account_id TEXT PRIMARY KEY,
    pts INTEGER NOT NULL,
    qts INTEGER NOT NULL,
    date INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    sync_uncertain INTEGER NOT NULL DEFAULT 0,
    last_synced_at INTEGER NOT NULL
);

-- Ensure sync_state has sync_uncertain and poll_timeout_secs if not already present
ALTER TABLE sync_state ADD COLUMN poll_timeout_secs INTEGER;
ALTER TABLE sync_state ADD COLUMN sync_uncertain INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS sync_baseline (
    baseline_id TEXT PRIMARY KEY,
    common_pts INTEGER NOT NULL,
    common_qts INTEGER NOT NULL,
    common_date INTEGER NOT NULL,
    common_seq INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'in_progress', -- 'in_progress', 'completed', 'aborted'
    captured_at INTEGER NOT NULL,
    completed_at INTEGER
);

CREATE TABLE IF NOT EXISTS channel_sync_queue (
    peer_id INTEGER PRIMARY KEY,
    discovered_pts INTEGER NOT NULL,
    current_pts INTEGER,
    status TEXT NOT NULL DEFAULT 'pending', -- 'pending', 'in_progress', 'completed', 'failed'
    attempts INTEGER NOT NULL DEFAULT 0,
    poll_timeout INTEGER,
    last_error TEXT,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS unsupported_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_id INTEGER,
    constructor_id INTEGER NOT NULL,
    pts INTEGER,
    pts_count INTEGER,
    qts INTEGER,
    qts_count INTEGER,
    affects_sync_state INTEGER NOT NULL DEFAULT 0,
    diagnostic_info TEXT,
    raw_tl BLOB NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS common_deletion_tombstones (
    message_id INTEGER PRIMARY KEY,
    pts INTEGER,
    pts_count INTEGER,
    observed_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_integrity_reports (
    report_id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope TEXT NOT NULL, -- 'account_common', 'channel', 'peer', 'full_sync_run'
    peer_id INTEGER,
    fully_lossless_contiguous_sync INTEGER NOT NULL,
    current_history_repaired INTEGER NOT NULL,
    new_messages_recovered INTEGER NOT NULL,
    current_content_reconciled INTEGER NOT NULL,
    historical_edits_complete INTEGER NOT NULL,
    historical_deletions_complete INTEGER NOT NULL,
    event_window_lost INTEGER NOT NULL,
    channel_discovery_complete INTEGER NOT NULL,
    gap_summary TEXT,
    created_at INTEGER NOT NULL
);
