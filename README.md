# Vendetta

Vendetta is a (currently) CLI designed to quickly export, index, verify and generate offline HTML documents of chat histories from a Telegram account

---

## Features

- Message identity is strictly `(peer_id, message_id)`. Reply, thread, and forum relationships are resolved via MTProto headers rather than pagination order
- Incremental delta synchronization using `updates.getDifference` and `updates.getChannelDifference`. Handles sequence gap detection, ring-buffer overflow recovery, and tracks edits and deletions
- Content-addressed media storage (SHA-256) with hardlink deduplication across identical files. Downloading scales via dynamic worker concurrency and handles FLOOD_WAIT using exponential backoff
- Static offline HTML export with self-contained views for discussion threads, forum topics, grouped media albums, reactions, edit histories, deletion markers, and client-side search indexing (**very expiremental**)
- Read-only integrity auditor that checks database constraints, foreign keys, reply-graph cycles, media hashes, static HTML references, and versioned provenance (**very expiremental**)


---

## Build

```bash
cargo build [--release] --workspace
```

---

## CLI

```bash
# 1. Authenticate with Telegram
vendetta auth --phone +1234567890

# 2. List accessible dialogs
vendetta dialogs

# 3. Synchronize message history and incremental updates
vendetta sync

# 4. Download media attachments
vendetta download-media

# 5. Export canonical archive to static HTML
vendetta export-html --archive archives/default/archive.db --output dist/html

# 6. Verify HTML link and asset integrity
vendetta verify-html --html-dir dist/html

# 7. Audit full archive database, media files, and reply graph
vendetta verify-archive --archive archives/default/archive.db --html dist/html --media --replies --search --strict
```

See [docs/cli.md](docs/cli.md) for the complete CLI reference and other options.

---

## Other docs (useful for AI only)

- [Project Architecture](docs/architecture.md)
- [Database Design](docs/database.md). SQLite schema, migrations, WAL mode, and FTS5 search index
- [MTProto](docs/mtproto.md). Some info about mtproto, containing various parameters, chunk rules, error recovery, and invariants

