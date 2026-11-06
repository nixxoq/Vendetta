# CLI reference

`vendetta` fetches Telegram history into a local SQLite archive, downloads media, and produces offline HTML. It also imports Telegram Desktop exports without connecting to Telegram.

For a typical account archive:

```sh
vendetta auth login
vendetta dialogs --limit 20
vendetta sync --peers 123456789,987654321
vendetta download-media --backfill
vendetta export-html --archive archives/default/archive.db --output dist/html
vendetta verify-html --html-dir dist/html
```

## Commands

| Command | Description |
| --- | --- |
| `auth` | Sign in, check the current session, or sign out. |
| `dialogs` | Find chat IDs and list chats accessible to the account. |
| `sync` | Save message history and reconcile subsequent messages, edits, and deletions. |
| `download-media` | Download attachments, profile pictures, and custom emoji reactions for archived chats. |
| `backfill-media` | Find attachments in archived messages and add them to the download queue without downloading them. |
| `media-stats` | See how many media objects are pending, downloaded, skipped, or failed. |
| `requeue-skipped` | Return all skipped media to the download queue. |
| `verify-media` | Check downloaded files against their recorded sizes and hashes. |
| `export-html` | Generate browsable HTML from a Vendetta archive. |
| `verify-html` | Check an HTML export for broken links, anchors, assets, and search data. |
| `verify-archive` | Audit the database and optionally check media, reply relationships, and HTML. |
| `import tdesktop` | Import a Telegram Desktop export into a new Vendetta database. |
| `convert tdesktop` | Convert a Telegram Desktop export straight to HTML through a temporary database. |
| `config` | Show settings or save defaults for later commands. |

## Shared options

These options work before or after the command name.

| Option | Meaning |
| --- | --- |
| `--config <PATH>` | Read settings from this file instead of searching the usual locations. |
| `--account <NAME>` | Choose the account used for default session and archive paths. Default: `default`. |
| `-q`, `--quiet` | Suppress informational progress output. |
| `--json` | Print results as JSON on stdout; progress uses stderr or is suppressed. Login can still prompt for a code or password. |
| `--no-color` | Disable ANSI colors. |
| `-h`, `--help` | Show help for the current command. |
| `-V`, `--version` | Show the version at the top level. |

The network commands (`auth`, `dialogs`, `sync`, and `download-media`) also accept:

| Option | Meaning |
| --- | --- |
| `--api-id <ID>` | Telegram API ID from `my.telegram.org`. Overrides the saved or environment value. |
| `--api-hash <HASH>` | Matching Telegram API hash. Overrides the saved or environment value. |
| `--session <PATH>` | Use a specific session file instead of the account's default session. |

## Authentication and chat

### `auth`

Run `vendetta auth` or `vendetta auth login` to sign in. Login asks for missing API credentials, a phone number, the Telegram login code, and a two-factor password when required. An authorized session is reused unless `--force` is supplied.

| Subcommand | Description |
| --- | --- |
| `login` | Sign in and save the session. This is the default when no subcommand is given. |
| `status` | Check whether the selected session is authorized. |
| `logout` | Revoke the session on Telegram and delete the local session file. |
| `logout --local-only` | Delete the local session file without revoking the session on Telegram. |
| `forget` | Alias for `logout --local-only`. |

| Option | Description |
| --- | --- |
| `--phone <PHONE>` | Supply the login phone number in international format, such as `+1234567890`. |
| `--force` | Run login again even if the selected session is already authorized. |

### `dialogs`

Lists accessible users, groups, and channels so you can choose IDs for `sync`.

| Option | Description |
| --- | --- |
| `--limit <N>` | Display at most this many matching dialogs. No limit by default. |
| `--peer-type <TYPE>` | Show only `user`, `group`, or `channel` dialogs. |

## Synchronization

### `sync`

Fetches history into SQLite and reconciles updates. Repeating the command resumes from saved progress. Without a peer selection, it uses the account's discovered dialogs.

| Option | Description |
| --- | --- |
| `-a`, `--archive <PATH>` | Destination SQLite database. Uses the configured or account-default archive when omitted. |
| `--peers <ID,ID...>` | Synchronize these peer IDs. Accepts comma-separated IDs or repeated `--peers` options. Explicit IDs take precedence over type filters and `--limit`. |
| `--peer-type <TYPE>` | Select discovered dialogs of type `user`, `group`, or `channel`. |
| `--exclude-peer-type <TYPE>` | Exclude this type from discovered dialogs. |
| `--limit <N>` | Synchronize at most this many discovered dialogs after filtering. |
| `--from <DATE>` | Fetch history starting at this date. |
| `--to <DATE>` | Fetch history through this date. |

**Note:** Dates accept `YYYY-MM-DD` or RFC3339 timestamps. 

```sh
vendetta sync --peer-type channel --limit 50
vendetta sync --peers 123456789 --from 2026-01-01 --to 2026-01-31
```

## Media

### `download-media`

Normally downloads profile pictures, custom emoji reactions, and queued message attachments. It scans archived messages for attachments automatically when the media table is empty. Use `--backfill` to force that scan after synchronizing more messages.

| Option | Description |
| --- | --- |
| `-a`, `--archive <PATH>` | Archive to read and update. Uses the configured or account-default archive when omitted. |
| `--media-dir <DIR>` | Download root. Uses the configured directory, otherwise `media` beside the database. |
| `--backfill` | Scan archived raw TL messages and register attachments before downloading. |
| `--avatars-only` | Download profile pictures and skip reactions and message attachments. |
| `--reactions-only` | Download custom emoji reactions and skip profile pictures and attachments. |
| `--photos-only` | Skip videos, video notes, documents, audio, voice messages, and animations. Stickers and thumbnails remain eligible. |
| `--skip-videos` | Skip video attachments and video notes. |
| `--max-size-mb <N>` | Skip message attachments larger than `N × 1,048,576` bytes. No size limit by default. |
| `--min-workers <N>` | Minimum attachment download concurrency. Default: `1`. |
| `--max-workers <N>` | Maximum attachment download concurrency. Default: `8`. |
| `--max-dc-workers <N>` | Maximum concurrent attachment downloads per Telegram data center. Default: `2`. |
| `--initial-workers <N>` | Attachment download concurrency at startup. Default: `2`. |

```sh
vendetta download-media --backfill --photos-only --max-size-mb 20
vendetta download-media --min-workers 2 --max-workers 8 --max-dc-workers 4
```

### Queue maintenance and file checks

These commands require an explicit `-a`, `--archive <PATH>`.

| Command | What it does |
| --- | --- |
| `backfill-media` | Scans raw TL stored in messages and registers media with the default policy. Does not fetch files. |
| `media-stats` | Prints the archive's media counts by status. |
| `requeue-skipped` | Resets all skipped items to pending. The next download pass applies its filters again. This command does not accept download filter options. |
| `verify-media` | Checks completed downloads for missing files, size mismatches, and hash mismatches; records verification results in SQLite. |

`verify-media` also accepts `--media-dir <DIR>`. Its fallback is the database's parent directory, unlike `download-media`. Pass the download root explicitly when checking files downloaded into `media`:

```sh
vendetta verify-media --archive archives/default/archive.db --media-dir archives/default/media
```

## HTML export

### `export-html`

Reads a Vendetta database and writes an HTML archive. It requires both `--archive` and `--output`, even if paths are saved in configuration.

| Option | Description |
| --- | --- |
| `-a`, `--archive <PATH>` | Required source SQLite database. |
| `-o`, `--output <DIR>` | Required destination for generated HTML and assets. |
| `--media-dir <DIR>` | Where to find source media. Defaults to the database's parent directory; the exporter checks both that directory and its `media` subdirectory. |
| `--media <MODE>` | `copy` copies media into the export. `link` creates symlinks on Unix and copies on other platforms. Default: `copy`. Symlinked exports depend on the source files remaining in place. |
| `--include-service-messages <BOOL>` | Include events such as joins and title changes. Default: `true`. |
| `--include-deleted-messages <BOOL>` | Include messages marked deleted in the archive. Default: `true`. |
| `--include-edit-history <BOOL>` | Include captured edit revisions. Default: `true`. |

The following options work with both `export-html` and `convert tdesktop`:

| Option | Description |
| --- | --- |
| `--mode <MODE>` | `telegram-like` uses a chat-style layout; `archive-optimized` uses an archive-oriented layout. Default: `telegram-like`. |
| `--theme <THEME>` | Initial theme: `light`, `dark`, or `system`. Default: `system`. |
| `--split-by <STRATEGY>` | `messages` splits pages by message count; `day` groups messages by calendar day. Default: `messages`. |
| `--chunk-size <N>` | Messages per page when splitting by message count. Default: `250`. |
| `--date-structure <STRUCTURE>` | Layout for day-based pages: `flat` puts them together, `tree` groups them in year/month directories. Default: `flat`. |
| `--readable-names` | Use sanitized chat titles for chat directory names instead of peer ID tokens. |
| `--disable-forum-render` | Render forum chats as a single chronological stream instead of separate topics. |
| `--from <DATE>` | Include messages starting at this date. Uses the same date formats as `sync`. |
| `--to <DATE>` | Include messages through this date. |
| `--build-search-index <BOOL>` | Build the browser search index. Default: `true`. |
| `--build-date-index <BOOL>` | Build the date-jump navigation index. Default: `true`. |
| `--replace` | Allow replacing an existing output directory. |

`export-html` can update an existing day-based export with a manifest without `--replace`. Other existing destinations are rejected unless replacement is requested. `convert tdesktop` requires `--replace` whenever its destination exists.

Options marked `<BOOL>` require `true` or `false`; for example, `--include-edit-history false`. Other switches, such as `--readable-names`, take no value.

```sh
vendetta export-html --archive archives/default/archive.db --output dist/html \
  --split-by day --date-structure tree --readable-names \
  --include-service-messages false
```

## Verification

### `verify-html`

Checks generated pages, links, message anchors, assets, and search data. Its only command-specific option is the required `--html-dir <DIR>`.

```sh
vendetta verify-html --html-dir dist/html
```

### `verify-archive`

Audits SQLite records and synchronization evidence. Supply `--archive`, `--html`, or both explicitly; this command does not use the configured archive path. The default database audit is `--full`. Media filesystem scans and reply-graph traversal must be requested separately.

| Option | Description |
| --- | --- |
| `-a`, `--archive <PATH>` | Database to audit. Required unless `--html` is supplied. |
| `-o`, `--html <DIR>` | HTML export to audit. Can be used with or without a database. |
| `--fast` | Run basic schema, foreign-key, and count checks. Cannot be combined with `--full`. |
| `--full` | Run the full core database and domain audit. This is the default; it does not enable media filesystem or reply-graph scans. |
| `--media` | Add media file checks when auditing a database. |
| `--replies` | Add reply and thread graph checks when auditing a database. |
| `--search` | Request search-index checks. Requires `--html`. |
| `--media-dir <DIR>` | Media root for file checks. Defaults to `media` beside the database. |
| `--rehash` | Recompute SHA-256 hashes for completed media files. Requires `--archive` and also enables media checks. |
| `--strict` | Treat verification warnings as errors. |

```sh
vendetta verify-archive --archive archives/default/archive.db --html dist/html \
  --media --replies --search --rehash --strict
```

## Telegram Desktop import

Both commands accept a Telegram Desktop JSON or HTML export as a directory or ZIP file

### `import tdesktop`

Creates a new Vendetta database and copies available media. The destination database must not already exist.

| Option | Description |
| --- | --- |
| `-s`, `--source <PATH>` | Required export directory or ZIP file. |
| `-a`, `--archive <PATH>` | Required destination for the new SQLite database. |
| `--media-dir <DIR>` | Destination for imported media. Defaults to `media` beside the new database. |

```sh
vendetta import tdesktop --source ~/Downloads/ChatExport.zip --archive imported/archive.db
```

### `convert tdesktop`

Produces HTML without keeping a Vendetta database. It imports into temporary storage, renders the result, and removes the temporary archive.

| Option | Description |
| --- | --- |
| `-s`, `--source <PATH>` | Required export directory or ZIP file. |
| `-o`, `--output <DIR>` | Required destination for the HTML export. |

It also accepts the shared HTML layout, date, index, and replacement options listed under [HTML export](#html-export). It does not accept `--media`, `--media-dir`, or the `--include-*` switches from `export-html`.

```sh
vendetta convert tdesktop --source ~/Downloads/ChatExport.zip --output dist/imported \
  --split-by day --date-structure tree
```

## Configuration

Run `vendetta config` or `vendetta config --show` to display effective settings with the API hash redacted. Supplying setting options saves those values to a configuration file and displays the saved settings.

| Option | Setting |
| --- | --- |
| `--show` | Display settings. Not needed when no updates are supplied. |
| `--api-id <ID>` | Telegram API ID. |
| `--api-hash <HASH>` | Telegram API hash. |
| `--account <NAME>` | Default account name. |
| `--archive <PATH>` | Default database for commands that resolve an optional archive path. |
| `--session <PATH>` | Default Telegram session file. |
| `--media-dir <DIR>` | Default download directory. |
| `--output <DIR>` | Save an output path. `export-html` and `convert tdesktop` still require an explicit `--output`. |
| `--base-dir <DIR>` | Base directory for the account and archive layout. Default: the current directory. |

Explicit command options override environment variables, which override the configuration file, which overrides built-in defaults. Without `--config`, Vendetta reads the first valid file from `./vendetta.json` and then `~/.config/vendetta/config.json`. It does not merge the two files.

Updates go to `--config` when supplied, otherwise to an existing `./vendetta.json`, otherwise to `~/.config/vendetta/config.json`. If `HOME` is unavailable, the fallback is `vendetta.json`. New configuration files are created with mode `0600` on Unix.

| Environment variable | Setting |
| --- | --- |
| `VENDETTA_API_ID`, fallback `TG_API_ID` | Telegram API ID. |
| `VENDETTA_API_HASH`, fallback `TG_API_HASH` | Telegram API hash. |
| `VENDETTA_ACCOUNT` | Account name. |
| `VENDETTA_ARCHIVE` | Default SQLite database. |
| `VENDETTA_SESSION` | Default session file. |
| `VENDETTA_MEDIA_DIR` | Default download directory. |
| `VENDETTA_OUTPUT` | Saved output setting; export commands still require `--output`. |
| `VENDETTA_BASE_DIR` | Base directory for account and archive paths. |

**NOTE:** with no overrides, account `default` uses `accounts/default/session.json`, `archives/default/archive.db`, and `archives/default/media` under the base directory
