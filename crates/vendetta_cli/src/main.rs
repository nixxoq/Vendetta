mod args;
mod auth;
mod client;
mod config;
mod dialogs;
mod media;
mod progress;
mod prompt;
mod render;
mod sync;

use anyhow::{Context, Result, bail};
use clap::Parser;
use std::{path::Path, sync::Arc};
use vendetta_model::{MediaFilterPolicy, PeerId};
use vendetta_render::ExportOptions;
use vendetta_storage::ArchiveDb;

use crate::args::*;
use crate::config::CliConfig;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let env_filter = match std::env::var("RUST_LOG") {
        Ok(val) if !val.trim().is_empty() => tracing_subscriber::EnvFilter::new(val),
        _ => {
            if cli.quiet {
                tracing_subscriber::EnvFilter::new("warn")
            } else {
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
            ExitCode::Fatal
        }
    };

    if exit_code != ExitCode::Success {
        std::process::exit(exit_code.as_i32());
    }
}

async fn run(cli: Cli) -> Result<ExitCode> {
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
            let (eff_api_id, eff_api_hash) = config.resolve_credentials(api_id, api_hash);
            let eff_session = config.resolve_session_path(session, account);

            match subcommand {
                Some(AuthSubcommands::Status) => {
                    let is_auth =
                        auth::run_auth_status(eff_api_id, eff_api_hash, eff_session, json).await?;
                    Ok(if is_auth || json {
                        ExitCode::Success
                    } else {
                        ExitCode::Warning
                    })
                }
                Some(AuthSubcommands::Logout { local_only }) => {
                    auth::run_auth_logout(eff_api_id, eff_api_hash, &eff_session, local_only, json)
                        .await?;
                    Ok(ExitCode::Success)
                }
                Some(AuthSubcommands::Forget) => {
                    auth::run_auth_logout(None, None, &eff_session, true, json).await?;
                    Ok(ExitCode::Success)
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
                    Ok(ExitCode::Success)
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
            let (eff_api_id, eff_api_hash) = config.resolve_credentials(api_id, api_hash);
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

            Ok(ExitCode::Success)
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
            from_date,
            to_date,
        } => {
            let eff_archive = config.resolve_archive_path(archive, account);
            let (eff_api_id, eff_api_hash) = config.resolve_credentials(api_id, api_hash);
            let eff_session = config.resolve_session_path(session, account);
            let target_peers = peers.map(|v| v.into_iter().map(PeerId::new).collect());

            let parsed_from = from_date
                .as_deref()
                .map(|f| vendetta_core::parse_date_bound(f, false).map_err(anyhow::Error::msg))
                .transpose()?;
            let parsed_to = to_date
                .as_deref()
                .map(|t| vendetta_core::parse_date_bound(t, true).map_err(anyhow::Error::msg))
                .transpose()?;

            let summary = sync::run_sync(
                eff_api_id,
                eff_api_hash,
                &eff_session,
                &eff_archive,
                target_peers,
                peer_type.map(Into::into),
                exclude_peer_type.map(Into::into),
                limit,
                (parsed_from, parsed_to),
                quiet,
                json,
            )
            .await?;

            if !summary.is_requested_scope_clean() {
                Ok(ExitCode::Error)
            } else if !summary.is_clean() {
                Ok(ExitCode::Warning)
            } else {
                Ok(ExitCode::Success)
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
            photos_only,
            skip_videos,
            max_size_mb,
        } => {
            let eff_archive = config.resolve_archive_path(archive, account);
            let eff_media_dir = config.resolve_media_dir(media_dir, &eff_archive);
            let (eff_api_id, eff_api_hash) = config.resolve_credentials(api_id, api_hash);
            let eff_session = config.resolve_session_path(session, account);

            let concurrency = media::MediaDownloadConcurrency {
                min_workers,
                max_workers,
                max_dc_workers,
                initial_workers,
            };

            let policy = MediaFilterPolicy {
                allow_videos: !photos_only && !skip_videos,
                allow_video_notes: !photos_only && !skip_videos,
                allow_documents: !photos_only,
                allow_audio: !photos_only,
                allow_voice: !photos_only,
                allow_animations: !photos_only,
                max_size_bytes: max_size_mb.map(|mb| mb * 1024 * 1024),
                ..MediaFilterPolicy::default()
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
                policy,
                quiet,
                json,
            )
            .await?;

            if summary.permanently_failed_count > 0
                || summary.retry_wait_count > 0
                || summary.needs_reauth_count > 0
            {
                Ok(ExitCode::Warning)
            } else {
                Ok(ExitCode::Success)
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
            disable_forum_render,
            from_date,
            to_date,
            readable_names,
            split_by,
            date_structure,
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

            let parsed_from = from_date
                .as_deref()
                .map(|f| vendetta_core::parse_date_bound(f, false).map_err(anyhow::Error::msg))
                .transpose()?;
            let parsed_to = to_date
                .as_deref()
                .map(|t| vendetta_core::parse_date_bound(t, true).map_err(anyhow::Error::msg))
                .transpose()?;

            let summary = render::run_export_html(
                &archive,
                options,
                disable_forum_render,
                readable_names,
                (parsed_from, parsed_to),
                split_by.into(),
                date_structure.into(),
            )?;

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
            Ok(ExitCode::Success)
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
            Ok(ExitCode::Success)
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

            let exit_code = match report.summary.exit_code {
                0 => ExitCode::Success,
                1 => ExitCode::Warning,
                2 => ExitCode::Error,
                _ => ExitCode::Fatal,
            };

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
                println!("==================================================");
                println!("MEDIA BACKFILL COMPLETED");
                println!("==================================================");
                println!("Messages Scanned:  {}", res.messages_scanned);
                println!("Media Discovered:  {}", res.media_discovered);
                println!("Media Eligible:    {}", res.media_eligible);
                println!("Media Skipped:     {}", res.media_skipped);
            }
            Ok(ExitCode::Success)
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
                Ok(ExitCode::Error)
            } else {
                Ok(ExitCode::Success)
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
            Ok(ExitCode::Success)
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
                println!("==================================================");
                println!("REQUEUED SKIPPED MEDIA");
                println!("==================================================");
                println!("Re-queued Count:       {count}");
            }
            Ok(ExitCode::Success)
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

            let status = if has_updates { "saved" } else { "completed" };
            let api_hash_redacted = display_config
                .api_hash
                .as_ref()
                .map(|_| "*** (REDACTED)".to_string());
            let api_hash_configured = display_config.api_hash.is_some();

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "schema_version": 1,
                        "command": "config",
                        "status": status,
                        "config_path": if has_updates { Some(target_path.display().to_string()) } else { None },
                        "api_id": display_config.api_id,
                        "api_hash_configured": api_hash_configured,
                        "api_hash_redacted": api_hash_redacted,
                        "account": display_config.account,
                        "archive": display_config.archive,
                        "session": display_config.session,
                        "media_dir": display_config.media_dir,
                        "output": display_config.output,
                        "base_dir": display_config.base_dir,
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
                    display_config
                        .api_id
                        .map(|i| i.to_string())
                        .unwrap_or_else(|| "(not set)".to_string())
                );
                println!(
                    "API Hash:          {}",
                    api_hash_redacted.unwrap_or_else(|| "(not set)".to_string())
                );
                println!(
                    "Account:           {}",
                    display_config
                        .account
                        .unwrap_or_else(|| "default".to_string())
                );
                println!(
                    "Archive:           {}",
                    display_config
                        .archive
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(default layout)".to_string())
                );
                println!(
                    "Session:           {}",
                    display_config
                        .session
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(default layout)".to_string())
                );
                println!(
                    "Media Dir:         {}",
                    display_config
                        .media_dir
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(default layout)".to_string())
                );
                if let Some(out) = &display_config.output {
                    println!("Output:            {}", out.display());
                }
                if let Some(base) = &display_config.base_dir {
                    println!("Base Dir:          {}", base.display());
                }
            }
            Ok(ExitCode::Success)
        }

        Commands::Import { subcommand } => match subcommand {
            ImportSubcommands::Tdesktop {
                source,
                archive,
                media_dir,
            } => {
                let options = vendetta_import::ImportOptions {
                    source_path: source,
                    archive_path: archive,
                    media_dir,
                };
                let summary = vendetta_import::import_tdesktop(&options)?;
                if json {
                    let out = serde_json::json!({
                        "schema_version": 1,
                        "command": "import tdesktop",
                        "status": "completed",
                        "archive": summary.archive_path.display().to_string(),
                        "media_dir": summary.media_dir.display().to_string(),
                        "chats_count": summary.chats_count,
                        "messages_count": summary.messages_count,
                        "media_copied_count": summary.media_copied_count,
                        "media_skipped_count": summary.media_skipped_count,
                    });
                    println!("{}", serde_json::to_string_pretty(&out)?);
                } else {
                    println!("==================================================");
                    println!("TELEGRAM DESKTOP IMPORT COMPLETED");
                    println!("==================================================");
                    println!("Archive:           {}", summary.archive_path.display());
                    println!("Media Dir:         {}", summary.media_dir.display());
                    println!("Chats Imported:    {}", summary.chats_count);
                    println!("Messages Imported: {}", summary.messages_count);
                    println!("Media Copied:      {}", summary.media_copied_count);
                    println!("Media Skipped:     {}", summary.media_skipped_count);
                }
                Ok(ExitCode::Success)
            }
        },

        Commands::Convert { subcommand } => match subcommand {
            ConvertSubcommands::Tdesktop {
                source,
                output,
                mode,
                theme,
                chunk_size,
                replace,
                build_search_index,
                build_date_index,
                disable_forum_render,
                from_date,
                to_date,
                readable_names,
                split_by,
                date_structure,
            } => {
                let parsed_from = from_date
                    .as_deref()
                    .map(|f| vendetta_core::parse_date_bound(f, false).map_err(anyhow::Error::msg))
                    .transpose()?;
                let parsed_to = to_date
                    .as_deref()
                    .map(|t| vendetta_core::parse_date_bound(t, true).map_err(anyhow::Error::msg))
                    .transpose()?;

                let options = vendetta_import::ConvertOptions {
                    source_path: source,
                    output_dir: output.clone(),
                    presentation_mode: mode.into(),
                    theme: theme.into(),
                    chunk_size,
                    replace,
                    split_by: split_by.into(),
                    date_structure: date_structure.into(),
                    readable_names,
                    date_bounds: (parsed_from, parsed_to),
                    build_search_index,
                    build_date_index,
                    disable_forum_render,
                };

                let summary = vendetta_import::convert_tdesktop(&options)?;

                if json {
                    let out = serde_json::json!({
                        "schema_version": 1,
                        "command": "convert tdesktop",
                        "status": "completed",
                        "destination": summary.destination.display().to_string(),
                        "dialogs_count": summary.dialogs_count,
                        "messages_count": summary.messages_count,
                        "chunks_count": summary.chunks_count,
                        "media_copied_count": summary.media_copied_count,
                        "manifest_path": summary.manifest_path.display().to_string(),
                    });
                    println!("{}", serde_json::to_string_pretty(&out)?);
                } else {
                    println!("==================================================");
                    println!("TELEGRAM DESKTOP DIRECT CONVERSION COMPLETED");
                    println!("==================================================");
                    println!("Destination:       {}", summary.destination.display());
                    println!("Dialogs:           {}", summary.dialogs_count);
                    println!("Messages:          {}", summary.messages_count);
                    println!("Chunks:            {}", summary.chunks_count);
                    println!("Media Copied:      {}", summary.media_copied_count);
                    println!("Manifest:          {}", summary.manifest_path.display());
                }
                Ok(ExitCode::Success)
            }
        },
    }
}
