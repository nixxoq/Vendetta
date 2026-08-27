# Command-Line Interface (CLI) Reference

Vendetta provides a unified command-line binary (`vendetta`) for account synchronization, media management, static HTML rendering, and archive verification.

---

## 1. Command Matrix

| Command | Subcommands / Key Flags | Description | JSON | Exit Codes |
| :--- | :--- | :--- | :---: | :--- |
| `vendetta auth` | `login`, `status`, `logout [--local-only]`, `forget`, `--phone <PHONE>`, `--force` | Interactive/headless MTProto authentication, status query, or session revocation. | Yes | `0, 1, 2, 3` |
| `vendetta dialogs` | `--limit <N>`, `--peer-type <user\|group\|channel>` | Enumerate accessible dialogs, groups, and channels. | Yes | `0, 2, 3` |
| `vendetta sync` | `--archive <PATH>`, `--peers <ID,ID...>`, `--peer-type <TYPE>`, `--exclude-peer-type <TYPE>`, `--limit <N>` | Synchronize message history and incremental deltas into SQLite. | Yes | `0, 1, 2, 3` |
| `vendetta download-media` | `--archive <PATH>`, `--media-dir <DIR>`, `--backfill`, `--avatars-only`, `--reactions-only`, `--min-workers <N>`, `--max-workers <N>`, `--max-dc-workers <N>` | Download pending media binaries with dynamic worker concurrency. | Yes | `0, 1, 2, 3` |
| `vendetta backfill-media` | `--archive <PATH>` | Scan archived messages and register pending media objects. | Yes | `0, 2, 3` |
| `vendetta verify-media` | `--archive <PATH>`, `--media-dir <DIR>` | Validate completed media files on disk against database records. | Yes | `0, 2, 3` |
| `vendetta media-stats` | `--archive <PATH>` | Output media object counts grouped by lifecycle state. | Yes | `0, 2, 3` |
| `vendetta requeue-skipped` | `--archive <PATH>` | Re-queue skipped media objects for filter re-evaluation. | Yes | `0, 2, 3` |
| `vendetta export-html` | `--archive <PATH>`, `--output <DIR>`, `--mode <MODE>`, `--media <copy\|link>`, `--theme <THEME>`, `--replace`, `--chunk-size <N>` | Export canonical SQLite archive into static offline HTML. | Yes | `0, 2, 3` |
| `vendetta verify-html` | `--html-dir <DIR>` | Audit links, message anchors, and asset paths in HTML export. | Yes | `0, 1, 2, 3` |
| `vendetta verify-archive` | `--archive <PATH>`, `--html <DIR>`, `--fast`, `--media`, `--replies`, `--search`, `--rehash`, `--strict` | Run multi-scope audit over database, media files, and HTML. | Yes | `0, 1, 2, 3` |
| `vendetta config` | `--show`, `--api-id <ID>`, `--api-hash <HASH>`, `--account <NAME>`, `--archive <PATH>`, `--session <PATH>`, `--media-dir <DIR>`, `--output <DIR>`, `--base-dir <DIR>` | Inspect or update persistent configuration with secrets redacted. | Yes | `0` |

---

## 2. Configuration Hierarchy & Precedence

Configuration values are resolved according to strict precedence:

$$\text{CLI Arguments} > \text{Config File} > \text{Environment Variables} > \text{Compiled Defaults}$$

### Environment Variables

| Variable | Type | Description |
| :--- | :--- | :--- |
| `VENDETTA_API_ID` (or `TG_API_ID`) | Integer | Telegram API ID from `my.telegram.org`. |
| `VENDETTA_API_HASH` (or `TG_API_HASH`) | String | Telegram API Hash from `my.telegram.org`. |
| `VENDETTA_ACCOUNT` | String | Active account identifier (default: `"default"`). |
| `VENDETTA_ARCHIVE` | Path | Path to target SQLite `archive.db`. |
| `VENDETTA_SESSION` | Path | Path to MTProto `session.json`. |
| `VENDETTA_MEDIA_DIR` | Path | Root directory for content-addressed media storage. |
| `VENDETTA_OUTPUT` | Path | Default static HTML export directory. |
| `VENDETTA_BASE_DIR` | Path | Workspace base directory containing accounts and archives. |

### Configuration File (`~/.config/vendetta/config.json` or `vendetta.json`)

```json
{
  "api_id": 1234567,
  "api_hash": "0123456789abcdef0123456789abcdef",
  "account": "default",
  "base_dir": "/var/data/vendetta"
}
```

On Unix platforms, configuration files containing secrets written by `vendetta config` or `vendetta auth` are created with `0600` permissions.

---

## 3. Exit Code Specifications

| Exit Code | Classification | Condition |
| :---: | :--- | :--- |
| **`0`** | `SUCCESS` | Operation completed without errors or unresolved warnings. |
| **`1`** | `WARNING` | Completed with non-fatal degraded conditions (e.g. non-fatal verification findings, unauthenticated session status query). |
| **`2`** | `ERROR` | Operational error, data validation failure, media corruption, or verification warnings promoted under `--strict`. |
| **`3`** | `FATAL` | Unrecoverable runtime fault, missing database file, or missing credentials when network access is required. |

---

## 4. Global Flags & Output Formatting

* **`--json`**: Emits structured JSON results to `stdout`. Interactive progress indicators are routed to `stderr` or suppressed.
* **`-q, --quiet`**: Suppresses non-error logging and terminal progress bars.
* **`--no-color`**: Strips ANSI escape codes from output.
* **`--config <PATH>`**: Explicit configuration file path override.
* **`--account <NAME>`**: Selects named account workspace directory.
* **Secret Redaction**: API hashes and authorization tokens are redacted (`*** (REDACTED)`) in human-readable and JSON configuration outputs. Password input is captured via terminal without echo.

---

## 5. Usage Examples

### Authentication
```bash
# Interactive login
vendetta auth --phone +1234567890

# Query authorization status
vendetta auth status

# Sign out remotely and purge session
vendetta auth logout
```

### Synchronization & Media Download
```bash
# Synchronize specific dialogs
vendetta sync --peers 123456789,987654321

# Synchronize all channels up to limit
vendetta sync --peer-type channel --limit 50

# Download pending media with bounded concurrency
vendetta download-media --min-workers 2 --max-workers 8 --max-dc-workers 4
```

### HTML Export & Verification
```bash
# Generate static HTML archive
vendetta export-html --archive archives/default/archive.db --output dist/html --mode telegram-like --media copy

# Audit HTML export links and anchors
vendetta verify-html --html-dir dist/html

# Perform full verification across SQLite, media hashes, and reply graph
vendetta verify-archive --archive archives/default/archive.db --html dist/html --media --replies --search --strict
```
