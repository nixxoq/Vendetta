-- Migration 0003: Media Engine, Resumable Chunk Downloads & Deduplication
-- Non-destructive migration preserving 100% of legacy media and message_media data.

-- Ensure legacy tables exist so migration queries run cleanly on both new databases and existing ones
CREATE TABLE IF NOT EXISTS media (
    media_key TEXT PRIMARY KEY,
    mime_type TEXT,
    size_bytes INTEGER,
    local_rel_path TEXT,
    sha256 TEXT,
    download_status TEXT NOT NULL DEFAULT 'pending',
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
    PRIMARY KEY (peer_id, message_id, media_key)
);

-- 1. Create target media_objects table
CREATE TABLE IF NOT EXISTS media_objects (
    media_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    mime_type TEXT,
    size_bytes INTEGER,
    file_name TEXT,
    size_type TEXT,
    width INTEGER,
    height INTEGER,
    dc_id INTEGER NOT NULL DEFAULT 0,
    source_location_tl BLOB,
    file_reference BLOB,
    local_rel_path TEXT,
    sha256 TEXT,
    download_status TEXT NOT NULL DEFAULT 'pending',
    downloaded_bytes INTEGER NOT NULL DEFAULT 0,
    chunk_size INTEGER NOT NULL DEFAULT 524288,
    retry_count INTEGER NOT NULL DEFAULT 0,
    max_retries INTEGER NOT NULL DEFAULT 5,
    next_retry_at INTEGER,
    claimed_at INTEGER,
    worker_id TEXT,
    last_error TEXT,
    filter_decision TEXT,
    filter_reason TEXT,
    policy_version INTEGER NOT NULL DEFAULT 1,
    verification_status TEXT NOT NULL DEFAULT 'unverified',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_media_objects_status ON media_objects(download_status, next_retry_at);
CREATE INDEX IF NOT EXISTS idx_media_objects_sha256 ON media_objects(sha256) WHERE sha256 IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_media_objects_dc ON media_objects(dc_id);

-- 2. Create target message_media_v2 table
CREATE TABLE IF NOT EXISTS message_media_v2 (
    peer_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL,
    media_id TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'attachment',
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (peer_id, message_id, media_id, role),
    FOREIGN KEY (peer_id, message_id) REFERENCES messages(peer_id, message_id) ON DELETE CASCADE,
    FOREIGN KEY (media_id) REFERENCES media_objects(media_id)
);

-- 3. Perform safe two-phase migration with collision remapping
CREATE TEMP TABLE IF NOT EXISTS _media_id_migration_map (
    legacy_key TEXT PRIMARY KEY,
    migrated_id TEXT NOT NULL
);

-- Populate default mapping
INSERT OR IGNORE INTO _media_id_migration_map (legacy_key, migrated_id)
SELECT media_key, media_key FROM media;

-- Handle contradictory verified content collision:
-- If media_objects already has media_id with a DIFFERENT verified sha256, assign a collision-safe ID
UPDATE _media_id_migration_map
SET migrated_id = legacy_key || '_legacy_conflict'
WHERE EXISTS (
    SELECT 1 FROM media_objects mo
    JOIN media m ON m.media_key = _media_id_migration_map.legacy_key
    WHERE mo.media_id = m.media_key
      AND mo.verification_status = 'verified'
      AND m.verified = 1
      AND mo.sha256 IS NOT NULL
      AND m.sha256 IS NOT NULL
      AND mo.sha256 != m.sha256
);

-- Insert / merge legacy media rows into media_objects
INSERT INTO media_objects (
    media_id, kind, mime_type, size_bytes, file_name, size_type, width, height, dc_id,
    source_location_tl, file_reference, local_rel_path, sha256,
    download_status, downloaded_bytes, chunk_size, retry_count, max_retries,
    next_retry_at, claimed_at, worker_id, last_error, filter_decision, filter_reason,
    policy_version, verification_status, created_at, updated_at
)
SELECT
    map.migrated_id,
    'other',
    m.mime_type,
    m.size_bytes,
    NULL,
    NULL,
    NULL,
    NULL,
    0,
    NULL,
    NULL,
    m.local_rel_path,
    m.sha256,
    m.download_status,
    m.downloaded_bytes,
    524288,
    0,
    5,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    NULL,
    1,
    CASE WHEN m.verified = 1 THEN 'verified' ELSE 'unverified' END,
    m.created_at,
    m.updated_at
FROM media m
JOIN _media_id_migration_map map ON m.media_key = map.legacy_key
ON CONFLICT(media_id) DO UPDATE SET
    download_status = CASE WHEN excluded.download_status = 'completed' THEN 'completed' ELSE media_objects.download_status END,
    verification_status = CASE WHEN excluded.verification_status = 'verified' THEN 'verified' ELSE media_objects.verification_status END,
    local_rel_path = COALESCE(media_objects.local_rel_path, excluded.local_rel_path),
    sha256 = COALESCE(media_objects.sha256, excluded.sha256),
    downloaded_bytes = MAX(media_objects.downloaded_bytes, excluded.downloaded_bytes),
    updated_at = MAX(media_objects.updated_at, excluded.updated_at);

-- Migrate legacy message_media rows into message_media_v2 using remapped IDs
INSERT OR IGNORE INTO message_media_v2 (peer_id, message_id, media_id, role, position)
SELECT
    mm.peer_id,
    mm.message_id,
    COALESCE(map.migrated_id, mm.media_key),
    'attachment',
    mm.position
FROM message_media mm
LEFT JOIN _media_id_migration_map map ON mm.media_key = map.legacy_key;

-- 4. Swap and archive legacy tables only if migration succeeds
DROP TABLE IF EXISTS message_media;
ALTER TABLE message_media_v2 RENAME TO message_media;

CREATE INDEX IF NOT EXISTS idx_message_media_media_id ON message_media(media_id);
CREATE INDEX IF NOT EXISTS idx_message_media_msg ON message_media(peer_id, message_id);

-- Archive legacy media table
ALTER TABLE media RENAME TO legacy_media_backup;

DROP TABLE IF EXISTS _media_id_migration_map;
