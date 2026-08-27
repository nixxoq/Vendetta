# System Architecture

Vendetta is structured as a modular Rust workspace that separates Telegram network operations, domain modeling, canonical SQLite persistence, media streaming, static HTML generation, and archive verification.

---

## 1. System Dataflow Pipeline

```text
Telegram MTProto API
        │
        ▼
Telegram Adapter (`vendetta_tg_adapter`)
        │
        ▼
Ingestion & Normalization
        │
        ▼
Canonical SQLite Archive (`vendetta_storage`)
   ├── Peer & User Metadata
   ├── Message Records & Revision Snapshots
   ├── Reply & Thread Graph
   ├── Media Index & Worker Leases
   ├── FTS5 Search Index
   └── Incremental Sync State & Provenance
        │
   ┌────┴────────────────────────┐
   ▼                             ▼
Static HTML Renderer      Verification Engine
(`vendetta_render`)       (`vendetta_verify`)
```

---

## 2. Ingestion Task Graph

```text
Peer Discovery
      │
      ▼
Page Ingestion Loop
  ├── Fetch History Page (`messages.getHistory`)
  ├── Normalize TL Objects to Domain Types
  ├── Atomic SQLite Transaction Commit
  │     ├── Upsert Peers & Users
  │     ├── Insert Message & Revisions
  │     └── Advance Sync Cursors
  ├── Extract Unresolved Reply & Thread Roots
  │     └── Targeted Fetch (`messages.getMessages`)
  └── Register Media Records into Queue
        │
        ▼
Media Download Execution
  ├── Content-Addressed Chunk Streaming (`upload.getFile`)
  ├── Dynamic Concurrency & Flood Wait Backoff
  └── SHA-256 Checksum Validation & DB Status Update
        │
        ▼
Downstream Processing
  ├── Static HTML Generation (`export-html`)
  └── Integrity Verification (`verify-archive`)
```

---

## 3. Crate Responsibilities

| Crate | Responsibility |
| :--- | :--- |
| **`vendetta_core`** | Shared core utilities, configuration types, and base error definitions. |
| **`vendetta_model`** | Normalized domain entities (`Peer`, `Message`, `MessageRevision`, `MediaObject`, `PeerType`, `MessageState`). Completely decoupled from MTProto library types. |
| **`vendetta_tg_adapter`** | Encapsulation of `grammers-client`. Implements authentication, session management, dialog enumeration, history pagination, Takeout sessions, and raw TL primitives. |
| **`vendetta_storage`** | Canonical SQLite persistence layer. Owns schema migrations (`0001`–`0005`), WAL single-writer actor, read connection pools, FTS5 full-text indexing, and raw TL storage. |
| **`vendetta_sync`** | Orchestration of full baseline export and incremental synchronization. Implements delta reconciliation (`updates.getDifference` / `updates.getChannelDifference`), gap recovery, and tombstone deletion tracking. |
| **`vendetta_media`** | Resumable media download engine. Handles content-addressed deduplication, dynamic worker concurrency, per-DC limits, FLOOD_WAIT backoff, range hash checks, and disk verification. |
| **`vendetta_render`** | Static HTML archive generator. Produces self-contained offline HTML with message chunking, thread views, media galleries, edit history, and sharded client-side search index. |
| **`vendetta_verify`** | Read-only integrity verifier. Audits schema invariants, foreign key constraints, reply graph cycles, media hashes, HTML link structures, and versioned provenance records. |
| **`vendetta_cli`** | Command-line dispatch for the 12 subcommands, configuration precedence resolution, secret sanitization, stderr progress rendering, and JSON stdout formatting. |

---

## 4. Architectural Invariants & Performance Rules

1. **Storage Decoupling**: MTProto library types do not leak into `vendetta_storage`, `vendetta_render`, or `vendetta_verify`. All external responses are normalized into `vendetta_model` representations before persistence.
2. **Crash Safety & Resumability**: Long-running operations persist state to SQLite on every transaction batch. Interruptions (SIGINT, network failure, process termination) allow resumption from durable database cursors.
3. **Bounded Memory Execution**: Query results and file downloads are streamed in chunks (e.g. 500–5,000 database rows; 1 MB file fragments). Large histories or media binaries are never buffered entirely in RAM.
4. **Isolated Rendering**: `vendetta_render` operates exclusively against local SQLite database records and filesystem assets; it never initiates network requests.
5. **Read-Only Verification**: `vendetta_verify` is strictly non-mutating, validating database consistency and export integrity without modifying state.
