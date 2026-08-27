pub mod adapter_factory;
pub mod auth;
pub mod config;
pub mod dialogs;
pub mod exit_codes;
pub mod media;
pub mod progress;
pub mod render;
pub mod sync;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use vendetta_model::{MediaFilterPolicy, PeerId, PeerType};
use vendetta_render::{ExportOptions, MediaMode, PresentationMode, ThemeMode};
use vendetta_storage::ArchiveDb;

use crate::{config::CliConfig, exit_codes::*};

#[derive(Parser, Debug)]
#[command(
    name = "vendetta",
    author = "NixxO",
    version = env!("CARGO_PKG_VERSION"),
    about = "Telegram chats archive & HTML exporter",
    long_about = "A command-line tool designed to quickly export, index, verify and generate offline HTML documents of chat histories from a Telegram account"
)]
pub struct Cli {
    /// Optional path to JSON configuration file.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Account identifier for multi-account workspace layout.
    #[arg(long, global = true)]
    pub account: Option<String>,

    /// Suppress informational progress output.
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    /// Output report or results as structured machine-readable JSON.
    #[arg(long, global = true)]
    pub json: bool,

    /// Disable ANSI color output.
    #[arg(long, global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Authenticate with Telegram, check authorization status, or log out.
    #[command(name = "auth")]
    Auth {
        #[command(subcommand)]
        subcommand: Option<AuthSubcommands>,

        /// Telegram API ID (numeric).
        #[arg(long, global = true)]
        api_id: Option<i32>,

        /// Telegram API Hash (hex string).
        #[arg(long, global = true)]
        api_hash: Option<String>,

        /// Explicit path to session file.
        #[arg(long, global = true)]
        session: Option<PathBuf>,

        /// Phone number in international format (+1234567890).
        #[arg(long, global = true)]
        phone: Option<String>,

        /// Force re-authentication even if already authorized.
        #[arg(long, global = true)]
        force: bool,
    },

    /// Discover and list accessible Telegram chats, groups, and channels.
    #[command(name = "dialogs")]
    Dialogs {
        /// Telegram API ID (numeric).
        #[arg(long)]
        api_id: Option<i32>,

        /// Telegram API Hash (hex string).
        #[arg(long)]
        api_hash: Option<String>,

        /// Explicit path to session file.
        #[arg(long)]
        session: Option<PathBuf>,

        /// Maximum number of dialogs to display.
        #[arg(long)]
        limit: Option<usize>,

        /// Filter by peer type.
        #[arg(long, value_enum)]
        peer_type: Option<CliPeerType>,
    },

    /// Synchronize Telegram account message history and incremental deltas into SQLite archive.
    #[command(name = "sync")]
    Sync {
        /// Path to SQLite archive file.
        #[arg(short, long)]
        archive: Option<PathBuf>,

        /// Telegram API ID (numeric).
        #[arg(long)]
        api_id: Option<i32>,

        /// Telegram API Hash (hex string).
        #[arg(long)]
        api_hash: Option<String>,

        /// Explicit path to session file.
        #[arg(long)]
        session: Option<PathBuf>,

        /// Specific peer IDs to synchronize (comma-separated or repeated). Explicit target override: type filters and dialog limit do not restrict this explicit peer list.
        #[arg(long, value_delimiter = ',')]
        peers: Option<Vec<i64>>,

        /// Filter to include specific dialog peer types (e.g. user, group, channel).
        #[arg(long, value_enum)]
        peer_type: Option<CliPeerType>,

        /// Exclude specific dialog peer types.
        #[arg(long, value_enum)]
        exclude_peer_type: Option<CliPeerType>,

        /// Maximum number of dialogs to synchronize.
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Download pending media items from Telegram into content-addressable storage.
    #[command(name = "download-media")]
    DownloadMedia {
        /// Path to SQLite archive file.
        #[arg(short, long)]
        archive: Option<PathBuf>,

        /// Path to root media storage directory.
        #[arg(long)]
        media_dir: Option<PathBuf>,

        /// Telegram API ID (numeric).
        #[arg(long)]
        api_id: Option<i32>,

        /// Telegram API Hash (hex string).
        #[arg(long)]
        api_hash: Option<String>,

        /// Explicit path to session file.
        #[arg(long)]
        session: Option<PathBuf>,

        /// Automatically backfill media objects from archived messages before downloading.
        #[arg(long, default_value_t = false)]
        backfill: bool,

        /// Synchronize ONLY peer and chat profile avatars, skipping message attachments.
        #[arg(long, default_value_t = false)]
        avatars_only: bool,

        /// Synchronize ONLY custom emoji reactions, skipping message attachments and avatars.
        #[arg(long, default_value_t = false)]
        reactions_only: bool,

        /// Minimum worker concurrency.
        #[arg(long, default_value_t = 1)]
        min_workers: usize,

        /// Maximum worker concurrency.
        #[arg(long, default_value_t = 8)]
        max_workers: usize,

        /// Maximum worker concurrency per Data Center.
        #[arg(long, default_value_t = 2)]
        max_dc_workers: usize,

        /// Initial worker concurrency.
        #[arg(long, default_value_t = 2)]
        initial_workers: usize,
    },

    /// Export canonical SQLite archive into a self-contained static HTML archive.
    #[command(name = "export-html")]
    ExportHtml {
        /// Path to SQLite archive file (e.g. archive.db).
        #[arg(short, long)]
        archive: PathBuf,

        /// Output directory for static HTML export.
        #[arg(short, long)]
        output: PathBuf,

        /// Visual presentation mode.
        #[arg(long, value_enum, default_value_t = CliPresentationMode::TelegramLike)]
        mode: CliPresentationMode,

        /// Media materialization policy.
        #[arg(long, value_enum, default_value_t = CliMediaMode::Copy)]
        media: CliMediaMode,

        /// Color theme default.
        #[arg(long, value_enum, default_value_t = CliThemeMode::System)]
        theme: CliThemeMode,

        /// Message chunk size per HTML page.
        #[arg(long, default_value_t = 250)]
        chunk_size: usize,

        /// Overwrite and replace output directory if it already exists.
        #[arg(long)]
        replace: bool,

        /// Source media directory (defaults to <archive_parent>/media).
        #[arg(long)]
        media_dir: Option<PathBuf>,

        /// Include service messages in export.
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        include_service_messages: bool,

        /// Include deleted messages in export.
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        include_deleted_messages: bool,

        /// Include message edit revisions in export.
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        include_edit_history: bool,

        /// Build client-side search index.
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        build_search_index: bool,

        /// Build date jump navigator index.
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        build_date_index: bool,
    },

    /// Verify integrity of a generated static HTML export.
    #[command(name = "verify-html")]
    VerifyHtml {
        /// Path to static HTML export directory.
        #[arg(long)]
        html_dir: PathBuf,
    },

    /// Deeply audit and verify Telegram archive integrity, reply graph, media binaries, and static HTML export.
    #[command(name = "verify-archive")]
    VerifyArchive {
        /// Path to SQLite archive file.
        #[arg(short, long)]
        archive: Option<PathBuf>,

        /// Path to static HTML export directory.
        #[arg(short = 'o', long = "html")]
        html_dir: Option<PathBuf>,

        /// Fast verification mode (schema, FK, basic counts; mutually exclusive with --full).
        #[arg(long, conflicts_with = "full")]
        fast: bool,

        /// Full core database and domain verification (default; excludes optional expensive reply graph and media filesystem scans).
        #[arg(long, conflicts_with = "fast")]
        full: bool,

        /// Scope audit to media objects and filesystem binary validation.
        #[arg(long)]
        media: bool,

        /// Scope audit to recursive reply and thread graph traversal.
        #[arg(long)]
        replies: bool,

        /// Scope audit to search manifest and shards.
        #[arg(long)]
        search: bool,

        /// Custom source media directory (defaults to <archive_parent>/media).
        #[arg(long)]
        media_dir: Option<PathBuf>,

        /// Perform full cryptographic SHA-256 rehashing on completed media binaries.
        #[arg(long)]
        rehash: bool,

        /// Strict mode: promote any warning to a non-zero exit code (exit code 2).
        #[arg(long)]
        strict: bool,

        /// Output report as structured machine-readable JSON.
        #[arg(long)]
        json: bool,
    },

    /// Plan and backfill media objects from existing messages in SQLite archive.
    #[command(name = "backfill-media")]
    BackfillMedia {
        /// Path to SQLite archive file.
        #[arg(short, long)]
        archive: PathBuf,
    },

    /// Verify all completed media files on disk against their SQLite records.
    #[command(name = "verify-media")]
    VerifyMedia {
        /// Path to SQLite archive file.
        #[arg(short, long)]
        archive: PathBuf,

        /// Path to root media storage directory.
        #[arg(long)]
        media_dir: Option<PathBuf>,
    },

    /// Display media archive statistics with status breakdown.
    #[command(name = "media-stats")]
    MediaStats {
        /// Path to SQLite archive file.
        #[arg(short, long)]
        archive: PathBuf,
    },

    /// Re-evaluate skipped media against current download filter policy.
    #[command(name = "requeue-skipped")]
    RequeueSkipped {
        /// Path to SQLite archive file.
        #[arg(short, long)]
        archive: PathBuf,
    },

    /// Display or update persistent configuration with secrets safely redacted.
    #[command(name = "config")]
    Config {
        /// Display active configuration.
        #[arg(long)]
        show: bool,

        /// Set Telegram API ID (numeric).
        #[arg(long)]
        api_id: Option<i32>,

        /// Set Telegram API Hash (hex string).
        #[arg(long)]
        api_hash: Option<String>,

        /// Set default account identifier.
        #[arg(long)]
        account: Option<String>,

        /// Set default path to SQLite archive file.
        #[arg(long)]
        archive: Option<PathBuf>,

        /// Set default path to session file.
        #[arg(long)]
        session: Option<PathBuf>,

        /// Set default media storage directory.
        #[arg(long)]
        media_dir: Option<PathBuf>,

        /// Set default HTML export output directory.
        #[arg(long)]
        output: Option<PathBuf>,

        /// Set default workspace base directory.
        #[arg(long)]
        base_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum AuthSubcommands {
    /// Perform interactive login flow (SMS code + 2FA password). Note: QR login is deferred on current pinned Grammers client.
    #[command(name = "login")]
    Login,

    /// Check authorization status of current session.
    #[command(name = "status")]
    Status,

    /// Sign out remotely with Telegram server and delete local session file.
    #[command(name = "logout")]
    Logout {
        /// Delete local session file only without contacting Telegram server.
        #[arg(long)]
        local_only: bool,
    },

    /// Shortcut alias for 'auth logout --local-only' (removes local session credentials without contacting Telegram server).
    #[command(name = "forget")]
    Forget,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliPeerType {
    #[value(name = "user")]
    User,
    #[value(name = "group")]
    Group,
    #[value(name = "channel")]
    Channel,
}

impl From<CliPeerType> for PeerType {
    fn from(p: CliPeerType) -> Self {
        match p {
            CliPeerType::User => PeerType::User,
            CliPeerType::Group => PeerType::Group,
            CliPeerType::Channel => PeerType::Channel,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliPresentationMode {
    #[value(name = "telegram-like")]
    TelegramLike,
    #[value(name = "archive-optimized")]
    ArchiveOptimized,
}

impl From<CliPresentationMode> for PresentationMode {
    fn from(m: CliPresentationMode) -> Self {
        match m {
            CliPresentationMode::TelegramLike => PresentationMode::TelegramLike,
            CliPresentationMode::ArchiveOptimized => PresentationMode::ArchiveOptimized,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliMediaMode {
    #[value(name = "copy")]
    Copy,
    #[value(name = "link")]
    Link,
}

impl From<CliMediaMode> for MediaMode {
    fn from(m: CliMediaMode) -> Self {
        match m {
            CliMediaMode::Copy => MediaMode::Copy,
            CliMediaMode::Link => MediaMode::Link,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliThemeMode {
    #[value(name = "light")]
    Light,
    #[value(name = "dark")]
    Dark,
    #[value(name = "system")]
    System,
}

impl From<CliThemeMode> for ThemeMode {
    fn from(t: CliThemeMode) -> Self {
        match t {
            CliThemeMode::Light => ThemeMode::Light,
            CliThemeMode::Dark => ThemeMode::Dark,
            CliThemeMode::System => ThemeMode::System,
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let env_filter = match std::env::var("RUST_LOG") {
        Ok(val) if !val.trim().is_empty() => tracing_subscriber::EnvFilter::new(val),
        _ => {
            if cli.quiet {
                tracing_subscriber::EnvFilter::new("warn")
            } else {
                // todo: remove it when i'll complete exporting topic groups
                tracing_subscriber::EnvFilter::new(
                    "info,grammers_mtsender=warn,grammers_mtproto=warn,grammers_client=warn",
                )
            }
        }
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .try_init();

    let exit_code = match run(cli).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e:?}");
            EXIT_FATAL
        }
    };

    if exit_code != EXIT_SUCCESS {
        std::process::exit(exit_code);
    }
}

fn resolve_credentials(
    api_id: Option<i32>,
    api_hash: Option<String>,
    config: &CliConfig,
) -> (Option<i32>, Option<String>) {
    (
        api_id.or(config.api_id),
        api_hash.or_else(|| config.api_hash.clone()),
    )
}

async fn run(cli: Cli) -> Result<i32> {
    let mut config = CliConfig::load(cli.config.as_deref());
    config.apply_account_override(cli.account.as_deref());
    let json = cli.json;
    let quiet = cli.quiet;
    let account = cli.account.as_deref();

    match cli.command {
        Commands::Auth {
            subcommand,
            api_id,
            api_hash,
            session,
            phone,
            force,
        } => {
            let (eff_api_id, eff_api_hash) = resolve_credentials(api_id, api_hash, &config);
            let eff_session = config.resolve_session_path(session, account);

            match subcommand {
                Some(AuthSubcommands::Status) => {
                    let is_auth =
                        auth::run_auth_status(eff_api_id, eff_api_hash, eff_session, json).await?;
                    Ok(if is_auth || json {
                        EXIT_SUCCESS
                    } else {
                        EXIT_WARNING
                    })
                }
                Some(AuthSubcommands::Logout { local_only }) => {
                    auth::run_auth_logout(eff_api_id, eff_api_hash, &eff_session, local_only, json)
                        .await?;
                    Ok(EXIT_SUCCESS)
                }
                Some(AuthSubcommands::Forget) => {
                    auth::run_auth_logout(None, None, &eff_session, true, json).await?;
                    Ok(EXIT_SUCCESS)
                }
                Some(AuthSubcommands::Login) | None => {
                    auth::run_auth(
                        eff_api_id,
                        eff_api_hash,
                        eff_session,
                        phone,
                        force,
                        json,
                        cli.config.as_deref(),
                    )
                    .await?;
                    Ok(EXIT_SUCCESS)
                }
            }
        }

        Commands::Dialogs {
            api_id,
            api_hash,
            session,
            limit,
            peer_type,
        } => {
            let (eff_api_id, eff_api_hash) = resolve_credentials(api_id, api_hash, &config);
            let eff_session = config.resolve_session_path(session, account);

            dialogs::run_dialogs(
                eff_api_id,
                eff_api_hash,
                &eff_session,
                limit,
                peer_type.map(Into::into),
                json,
            )
            .await?;

            Ok(EXIT_SUCCESS)
        }

        Commands::Sync {
            archive,
            api_id,
            api_hash,
            session,
            peers,
            peer_type,
            exclude_peer_type,
            limit,
        } => {
            let eff_archive = config.resolve_archive_path(archive, account);
            let (eff_api_id, eff_api_hash) = resolve_credentials(api_id, api_hash, &config);
            let eff_session = config.resolve_session_path(session, account);
            let target_peers = peers.map(|v| v.into_iter().map(PeerId::new).collect());

            let summary = sync::run_sync(
                eff_api_id,
                eff_api_hash,
                &eff_session,
                &eff_archive,
                target_peers,
                peer_type.map(Into::into),
                exclude_peer_type.map(Into::into),
                limit,
                quiet,
                json,
            )
            .await?;

            if !summary.is_requested_scope_clean() {
                Ok(EXIT_ERROR)
            } else if !summary.is_clean() {
                Ok(EXIT_WARNING)
            } else {
                Ok(EXIT_SUCCESS)
            }
        }

        Commands::DownloadMedia {
            archive,
            media_dir,
            api_id,
            api_hash,
            session,
            backfill,
            avatars_only,
            reactions_only,
            min_workers,
            max_workers,
            max_dc_workers,
            initial_workers,
        } => {
            let eff_archive = config.resolve_archive_path(archive, account);
            let eff_media_dir = config.resolve_media_dir(media_dir, &eff_archive);
            let (eff_api_id, eff_api_hash) = resolve_credentials(api_id, api_hash, &config);
            let eff_session = config.resolve_session_path(session, account);

            let concurrency = media::MediaDownloadConcurrency {
                min_workers,
                max_workers,
                max_dc_workers,
                initial_workers,
            };

            let summary = media::run_download_media(
                eff_api_id,
                eff_api_hash,
                &eff_session,
                &eff_archive,
                &eff_media_dir,
                concurrency,
                backfill,
                avatars_only,
                reactions_only,
                quiet,
                json,
            )
            .await?;

            if summary.permanently_failed_count > 0
                || summary.retry_wait_count > 0
                || summary.needs_reauth_count > 0
            {
                Ok(EXIT_WARNING)
            } else {
                Ok(EXIT_SUCCESS)
            }
        }

        Commands::ExportHtml {
            archive,
            output,
            mode,
            media,
            theme,
            chunk_size,
            replace,
            media_dir,
            include_service_messages,
            include_deleted_messages,
            include_edit_history,
            build_search_index,
            build_date_index,
        } => {
            let media_src = media_dir
                .unwrap_or_else(|| archive.parent().unwrap_or(Path::new(".")).to_path_buf());

            let options = ExportOptions {
                output_dir: output.clone(),
                presentation_mode: mode.into(),
                media_mode: media.into(),
                theme: theme.into(),
                chunk_size,
                replace,
                media_src_dir: Some(media_src),
                include_service_messages,
                include_deleted_messages,
                include_edit_history,
                build_search_index,
                build_date_index,
                target_peers: None,
            };

            let summary = render::run_export_html(&archive, options)?;
            if json {
                let out = serde_json::json!({
                    "schema_version": 1,
                    "command": "export-html",
                    "status": "completed",
                    "destination": output.display().to_string(),
                    "dialogs_count": summary.dialogs_count,
                    "messages_count": summary.messages_count,
                    "chunks_count": summary.chunks_count,
                    "media_copied_count": summary.media_copied_count,
                    "manifest_path": summary.manifest_path.display().to_string(),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                // TODO: perhaps it's better to use logging stuff to output smth like that?
                println!("==================================================");
                println!("HTML EXPORT COMPLETED SUCCESSFULLY");
                println!("==================================================");
                println!("Destination:       {}", output.display());
                println!("Dialogs:           {}", summary.dialogs_count);
                println!("Messages:          {}", summary.messages_count);
                println!("Chunks:            {}", summary.chunks_count);
                println!("Media Copied:      {}", summary.media_copied_count);
                println!("Manifest:          {}", summary.manifest_path.display());
            }
            Ok(EXIT_SUCCESS)
        }

        Commands::VerifyHtml { html_dir } => {
            let report = render::run_verify_html(&html_dir)?;
            if json {
                let out = serde_json::json!({
                    "schema_version": 1,
                    "command": "verify-html",
                    "status": "passed",
                    "html_dir": html_dir.display().to_string(),
                    "pages_checked": report.total_pages_checked,
                    "anchors_checked": report.total_anchors_checked,
                    "links_checked": report.total_links_checked,
                    "media_checked": report.total_media_checked,
                    "errors_count": 0,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                // TODO: perhaps it's better to use logging stuff to output smth like that?
                println!("==================================================");
                println!("HTML ARCHIVE INTEGRITY VERIFICATION PASSED");
                println!("==================================================");
                println!("Directory:         {}", html_dir.display());
                println!("Pages checked:     {}", report.total_pages_checked);
                println!("Anchors checked:   {}", report.total_anchors_checked);
                println!("Links checked:     {}", report.total_links_checked);
                println!("Media checked:     {}", report.total_media_checked);
                println!("Errors:            0");
            }
            Ok(EXIT_SUCCESS)
        }

        Commands::VerifyArchive {
            archive,
            html_dir,
            fast,
            full: _,
            media,
            replies,
            search,
            media_dir,
            rehash,
            strict,
            json: verify_json,
        } => {
            let effective_json = json || verify_json;

            if archive.is_none() && html_dir.is_none() {
                bail!("At least one of --archive <PATH> or --html <DIR> must be provided.");
            }

            if search && html_dir.is_none() {
                bail!(
                    "--search requires --html <EXPORT_DIR> because search index shards are part of the static HTML export."
                );
            }

            if rehash && archive.is_none() {
                bail!("--rehash requires --archive <PATH>.");
            }

            let mode = if fast {
                vendetta_verify::VerificationMode::Fast
            } else {
                vendetta_verify::VerificationMode::Full
            };

            let options = vendetta_verify::VerificationOptions {
                archive_path: archive,
                html_dir,
                media_dir,
                mode,
                scope_media: media || rehash,
                scope_replies: replies,
                scope_search: search,
                rehash_media: rehash,
                strict,
            };

            let report = vendetta_verify::VerificationEngine::new(options)
                .run()
                .context("Verification engine execution failed")?;

            let exit_code = report.summary.exit_code;

            // TODO: perhaps it's better to use logging stuff to output smth like that?
            if effective_json {
                let json_output = vendetta_verify::format_json(&report)
                    .context("Failed to serialize verification report to JSON")?;
                println!("{json_output}");
            } else {
                let human_output = vendetta_verify::format_human_readable(&report);
                print!("{human_output}");
            }

            Ok(exit_code)
        }

        Commands::BackfillMedia { archive } => {
            let db =
                Arc::new(ArchiveDb::open(&archive).context("Failed to open archive database")?);
            let policy = MediaFilterPolicy::default();
            let res = media::run_backfill_media(db, &policy)?;

            if json {
                let out = serde_json::json!({
                    "schema_version": 1,
                    "command": "backfill-media",
                    "status": "completed",
                    "archive": archive.display().to_string(),
                    "messages_scanned": res.messages_scanned,
                    "media_discovered": res.media_discovered,
                    "media_eligible": res.media_eligible,
                    "media_skipped": res.media_skipped,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                // TODO: perhaps it's better to use logging stuff to output smth like that?
                println!("==================================================");
                println!("MEDIA BACKFILL COMPLETED");
                println!("==================================================");
                println!("Messages Scanned:  {}", res.messages_scanned);
                println!("Media Discovered:  {}", res.media_discovered);
                println!("Media Eligible:    {}", res.media_eligible);
                println!("Media Skipped:     {}", res.media_skipped);
            }
            Ok(EXIT_SUCCESS)
        }

        Commands::VerifyMedia { archive, media_dir } => {
            let db =
                Arc::new(ArchiveDb::open(&archive).context("Failed to open archive database")?);
            let root = media_dir
                .unwrap_or_else(|| archive.parent().unwrap_or(Path::new(".")).to_path_buf());
            let report = media::run_verify_media(db, root, quiet, json)?;

            if json {
                let out = serde_json::json!({
                    "schema_version": 1,
                    "command": "verify-media",
                    "status": if report.missing_count == 0 && report.corrupted_size_count == 0 && report.corrupted_hash_count == 0 { "passed" } else { "failed" },
                    "archive": archive.display().to_string(),
                    "total_checked": report.total_checked,
                    "verified_count": report.verified_count,
                    "missing_count": report.missing_count,
                    "corrupted_size_count": report.corrupted_size_count,
                    "corrupted_hash_count": report.corrupted_hash_count,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                // TODO: perhaps it's better to use logging stuff to output smth like that?
                println!("==================================================");
                println!("MEDIA STORAGE VERIFICATION COMPLETED");
                println!("==================================================");
                println!("Total Checked:     {}", report.total_checked);
                println!("Verified:          {}", report.verified_count);
                println!("Missing:           {}", report.missing_count);
                println!("Corrupted Size:    {}", report.corrupted_size_count);
                println!("Corrupted Hash:    {}", report.corrupted_hash_count);
            }

            if report.missing_count > 0
                || report.corrupted_size_count > 0
                || report.corrupted_hash_count > 0
            {
                Ok(EXIT_ERROR)
            } else {
                Ok(EXIT_SUCCESS)
            }
        }

        Commands::MediaStats { archive } => {
            let db = ArchiveDb::open(&archive).context("Failed to open archive database")?;
            let stats = media::run_media_stats(&db)?;

            if json {
                let out = serde_json::json!({
                    "schema_version": 1,
                    "command": "media-stats",
                    "status": "completed",
                    "archive": archive.display().to_string(),
                    "total_count": stats.total_count,
                    "pending_count": stats.pending_count,
                    "downloading_count": stats.downloading_count,
                    "completed_count": stats.completed_count,
                    "verified_count": stats.verified_count,
                    "missing_file_count": stats.missing_file_count,
                    "corrupted_hash_count": stats.corrupted_hash_count,
                    "corrupted_size_count": stats.corrupted_size_count,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                // TODO: perhaps it's better to use logging stuff to output smth like that?
                println!("==================================================");
                println!("MEDIA ARCHIVE STATISTICS");
                println!("==================================================");
                println!("Total Media Objects:   {}", stats.total_count);
                println!("Pending:               {}", stats.pending_count);
                println!("Downloading:           {}", stats.downloading_count);
                println!("Completed:             {}", stats.completed_count);
                println!("Verified on Disk:      {}", stats.verified_count);
                println!("Missing on Disk:       {}", stats.missing_file_count);
                println!("Corrupted Hash:        {}", stats.corrupted_hash_count);
                println!("Corrupted Size:        {}", stats.corrupted_size_count);
            }
            Ok(EXIT_SUCCESS)
        }

        Commands::RequeueSkipped { archive } => {
            let db = ArchiveDb::open(&archive).context("Failed to open archive database")?;
            let policy = MediaFilterPolicy::default();
            let count = media::run_requeue_skipped(&db, &policy)?;

            if json {
                let out = serde_json::json!({
                    "schema_version": 1,
                    "command": "requeue-skipped",
                    "status": "completed",
                    "archive": archive.display().to_string(),
                    "requeued_count": count,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                // TODO: perhaps it's better to use logging stuff to output smth like that?
                println!("==================================================");
                println!("REQUEUED SKIPPED MEDIA");
                println!("==================================================");
                println!("Re-queued Count:       {count}");
            }
            Ok(EXIT_SUCCESS)
        }

        Commands::Config {
            show: _,
            api_id,
            api_hash,
            account: cfg_account,
            archive: cfg_archive,
            session: cfg_session,
            media_dir: cfg_media_dir,
            output: cfg_output,
            base_dir: cfg_base_dir,
        } => {
            let updates = CliConfig {
                api_id,
                api_hash,
                account: cfg_account,
                archive: cfg_archive,
                session: cfg_session,
                media_dir: cfg_media_dir,
                output: cfg_output,
                base_dir: cfg_base_dir,
            };

            let has_updates = updates != CliConfig::default();
            let target_path = CliConfig::resolve_write_config_path(cli.config.as_deref());

            let display_config = if has_updates {
                let mut current = CliConfig::load_file_or_default(&target_path);
                current.merge(updates);
                current.save_to_file(&target_path)?;
                current
            } else {
                config
            };

            let sanitized = display_config.sanitized_view();
            let status = if has_updates { "saved" } else { "completed" };

            // TODO: perhaps it's better to use logging stuff to output smth like that?
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "schema_version": 1,
                        "command": "config",
                        "status": status,
                        "config_path": if has_updates { Some(target_path.display().to_string()) } else { None },
                        "api_id": sanitized.api_id,
                        "api_hash_configured": sanitized.api_hash_configured,
                        "api_hash_redacted": sanitized.api_hash_redacted,
                        "account": sanitized.account,
                        "archive": sanitized.archive,
                        "session": sanitized.session,
                        "media_dir": sanitized.media_dir,
                        "output": sanitized.output,
                        "base_dir": sanitized.base_dir,
                    }))?
                );
            } else {
                let title = if has_updates {
                    "VENDETTA CONFIGURATION SAVED"
                } else {
                    "ACTIVE VENDETTA CONFIGURATION"
                };
                println!("==================================================");
                println!("{title}");
                println!("==================================================");
                if has_updates {
                    println!("Config File:       {}", target_path.display());
                }
                println!(
                    "API ID:            {}",
                    sanitized
                        .api_id
                        .map(|i| i.to_string())
                        .unwrap_or_else(|| "(not set)".to_string())
                );
                println!(
                    "API Hash:          {}",
                    sanitized
                        .api_hash_redacted
                        .unwrap_or_else(|| "(not set)".to_string())
                );
                println!(
                    "Account:           {}",
                    sanitized.account.unwrap_or_else(|| "default".to_string())
                );
                println!(
                    "Archive:           {}",
                    sanitized
                        .archive
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(default layout)".to_string())
                );
                println!(
                    "Session:           {}",
                    sanitized
                        .session
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(default layout)".to_string())
                );
                println!(
                    "Media Dir:         {}",
                    sanitized
                        .media_dir
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(default layout)".to_string())
                );
                if let Some(out) = &sanitized.output {
                    println!("Output:            {}", out.display());
                }
                if let Some(base) = &sanitized.base_dir {
                    println!("Base Dir:          {}", base.display());
                }
            }
            Ok(EXIT_SUCCESS)
        }
    }
}
