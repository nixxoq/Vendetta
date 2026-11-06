# Database Design

## 1. Concurrency Architecture

```text
                     SQLite Database (WAL Mode)
                                │
               ┌────────────────┴────────────────┐
               ▼                                 ▼
    Dedicated Write Worker               Read Connection Pool
  (Single Actor Thread / MPSC)         (Concurrent Readers / Pool)
               │                                 │
     ├── Ingestion batches             ├── HTML export workers
     ├── Revision tracking             ├── Verification checks
     ├── Media status updates          ├── FTS5 search queries
     └── Sync cursor updates           └── Reply graph traversal
```


## 2. Structures

### Message entity

```sql
CREATE TABLE messages (
    peer_id                 INTEGER NOT NULL,
    message_id              INTEGER NOT NULL,
    sender_id               INTEGER,
    date                    INTEGER NOT NULL,
    text                    TEXT,
    entities_json           TEXT,
    reply_to_msg_id         INTEGER,
    reply_to_top_id         INTEGER,
    reply_to_peer_id        INTEGER,
    grouped_id              INTEGER,
    forward_json            TEXT,
    reactions_json          TEXT,
    views                   INTEGER,
    forwards_count          INTEGER,
    state                   TEXT NOT NULL DEFAULT 'active',
    edit_date               INTEGER,
    raw_tl                  BLOB,
    PRIMARY KEY (peer_id, message_id),
    FOREIGN KEY (peer_id) REFERENCES peers(peer_id)
);

CREATE INDEX idx_messages_date ON messages(peer_id, date);
CREATE INDEX idx_messages_grouped ON messages(grouped_id) WHERE grouped_id IS NOT NULL;
CREATE INDEX idx_messages_reply_target ON messages(peer_id, reply_to_msg_id) WHERE reply_to_msg_id IS NOT NULL;
```

### Message states
* `active` - Standard message.
* `edited` - Message modified after initial dispatch (revisions stored in `message_revisions`).
* `deleted` - Deleted message (detected during incremental sync)
* `empty` - Empty/unavailable message placeholder (`messageEmpty#90a6ca84`).
* `inaccessible` - Message restricted or blocked

### Message revisions

```sql
CREATE TABLE message_revisions (
    peer_id                 INTEGER NOT NULL,
    message_id              INTEGER NOT NULL,
    revision_id             INTEGER NOT NULL,
    captured_at             INTEGER NOT NULL,
    edit_date               INTEGER,
    text                    TEXT,
    entities_json           TEXT,
    raw_tl                  BLOB,
    PRIMARY KEY (peer_id, message_id, revision_id),
    FOREIGN KEY (peer_id, message_id) REFERENCES messages(peer_id, message_id)
);
```

### Message replies

```sql
CREATE TABLE message_replies (
    source_peer_id          INTEGER NOT NULL,
    source_message_id       INTEGER NOT NULL,
    target_peer_id          INTEGER NOT NULL,
    target_message_id       INTEGER NOT NULL,
    top_message_id          INTEGER,
    resolution_status       TEXT NOT NULL,
    PRIMARY KEY (source_peer_id, source_message_id),
    FOREIGN KEY (source_peer_id, source_message_id) REFERENCES messages(peer_id, message_id)
);
```

* `resolution_status` values: `resolved`, `context_only`, `missing`, `deleted`, `inaccessible`, `not_requested`.

### Media objects

```sql
CREATE TABLE media_objects (
    media_key               TEXT PRIMARY KEY,
    media_type              TEXT NOT NULL,
    size_bytes              INTEGER,
    mime_type               TEXT,
    sha256                  TEXT,
    local_path              TEXT,
    download_status         TEXT NOT NULL DEFAULT 'pending',
    downloaded_bytes        INTEGER NOT NULL DEFAULT 0,
    filter_reason           TEXT,
    worker_lease            TEXT,
    lease_expires_at        INTEGER,
    source_location_tl      BLOB,
    created_at              INTEGER NOT NULL,
    completed_at            INTEGER
);

CREATE TABLE message_media (
    peer_id                 INTEGER NOT NULL,
    message_id              INTEGER NOT NULL,
    media_key               TEXT NOT NULL,
    PRIMARY KEY (peer_id, message_id, media_key),
    FOREIGN KEY (peer_id, message_id) REFERENCES messages(peer_id, message_id),
    FOREIGN KEY (media_key) REFERENCES media_objects(media_key)
);
```

### Full-text search

**Note:** Full-text search is implemented using SQLite FTS5 with `unicode61` tokenization

```sql
CREATE VIRTUAL TABLE messages_fts USING fts5(
    text,
    peer_id UNINDEXED,
    message_id UNINDEXED,
    sender_id UNINDEXED,
    date UNINDEXED,
    state UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);
```

---

## 3. Schema Migrations

| Version | Migration File | Description |
| :---: | :--- | :--- |
| **`1`** | `0001_initial_schema.sql` | Initial schema |
| **`2`** | `0002_incremental_sync.sql` | added delta synchronization ( `sync_state_common`, `sync_state_channels`, `sync_events_queue`, `sync_integrity_reports`, `unsupported_events`) |
| **`3`** | `0003_media_engine.sql` | Added `media_objects` |
| **`4`** | `0004_deletion_provenance.sql` | Mark deleted/edited messages |
| **`5`** | `0005_fts5_search.sql` | FTS5 search |
