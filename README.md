# Vendetta

Vendetta is a, currently, command-line interface designed to replace the export feature in original Telegram Desktop

---

## Features

- UI:
  * Rendering custom emojis (you need to synchronize reactions first)
  * Viewing forum messages just like in Telegram
  * Telegram-like UI (compact mode available)
- Incremental synchronization using `updates.getDifference` and `updates.getChannelDifference`
- Tracks edits and deletions (**experimental**)
- Media saving (almost all)
- Client-size search indexing (**experimental**)
- Can convert the original export from Telegram Desktop in JSON/HTML format into the Vendetta one

---

## Build

```bash
cargo build [--release] --workspace
```

---

## CLI

See [docs/cli.md](docs/cli.md) for the complete CLI reference and other options.

---

## Other docs

- [Project Architecture](docs/architecture.md)
- [Database Design](docs/database.md)
