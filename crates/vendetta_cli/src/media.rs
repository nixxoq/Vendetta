use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tracing::{debug, info};
use vendetta_media::{
    BackfillResult, MediaBackfillPlanner, MediaEngine, MediaVerifier, SchedulerSummary,
    StorageLayoutManager, VerificationReport, sync_all_custom_reactions, sync_all_peer_avatars,
};
use vendetta_model::{MediaFilterPolicy, MediaQueueStats, MediaStats};
use vendetta_storage::ArchiveDb;
use vendetta_tg_adapter::TelegramAdapter;

use crate::progress::{CliProgress, MediaDownloadProgressTracker, MediaVerifyProgressTracker};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaDownloadOutput {
    pub schema_version: u32,
    pub command: String,
    pub status: String,
    pub summary: MediaDownloadSummaryPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaDownloadSummaryPayload {
    pub completed_count: usize,
    pub retry_wait_count: usize,
    pub permanently_failed_count: usize,
    pub needs_reauth_count: usize,
}

impl From<&SchedulerSummary> for MediaDownloadSummaryPayload {
    fn from(s: &SchedulerSummary) -> Self {
        Self {
            completed_count: s.completed_count,
            retry_wait_count: s.retry_wait_count,
            permanently_failed_count: s.permanently_failed_count,
            needs_reauth_count: s.needs_reauth_count,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MediaDownloadConcurrency {
    pub min_workers: usize,
    pub max_workers: usize,
    pub max_dc_workers: usize,
    pub initial_workers: usize,
}

pub fn run_backfill_media(
    db: Arc<ArchiveDb>,
    policy: &MediaFilterPolicy,
) -> Result<BackfillResult> {
    let planner = MediaBackfillPlanner::new(db);
    let res = planner.plan_media_from_archive(policy)?;
    info!(
        "Backfill results: {} scanned, {} discovered, {} eligible, {} skipped",
        res.messages_scanned, res.media_discovered, res.media_eligible, res.media_skipped
    );
    Ok(res)
}

fn print_media_json(status: &str, summary: MediaDownloadSummaryPayload) -> Result<()> {
    let out = MediaDownloadOutput {
        schema_version: 1,
        command: "download-media".to_string(),
        status: status.to_string(),
        summary,
    };
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

fn print_scheduler_summary(summary: &SchedulerSummary, json: bool) -> Result<()> {
    let status = if summary.permanently_failed_count > 0
        || summary.retry_wait_count > 0
        || summary.needs_reauth_count > 0
    {
        "warnings"
    } else {
        "completed"
    };

    if json {
        print_media_json(status, MediaDownloadSummaryPayload::from(summary))?;
    } else {
        // TODO: perhaps it's better to use logging stuff to output smth like that?
        println!("==================================================");
        println!("MEDIA DOWNLOAD PASS COMPLETED");
        println!("==================================================");
        println!("Completed Downloads:       {}", summary.completed_count);
        println!("Waiting on Retry/Cooldown: {}", summary.retry_wait_count);
        println!(
            "Permanently Failed:        {}",
            summary.permanently_failed_count
        );
        println!("Needs Re-authentication:   {}", summary.needs_reauth_count);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_download_media_with_adapter(
    db: Arc<ArchiveDb>,
    adapter: Arc<dyn TelegramAdapter>,
    archive_root: impl Into<PathBuf>,
    concurrency: MediaDownloadConcurrency,
    backfill: bool,
    avatars_only: bool,
    reactions_only: bool,
    mut progress: CliProgress,
    json: bool,
) -> Result<SchedulerSummary> {
    if !adapter
        .is_authorized()
        .await
        .context("Failed to check session authorization")?
    {
        bail!("Telegram session is not authorized. Please run 'vendetta auth' to log in.");
    }

    let root_path = archive_root.into();
    let storage_layout = StorageLayoutManager::new(root_path.clone());
    let _ = storage_layout.ensure_dirs();

    if reactions_only {
        progress.stage("Synchronizing custom emoji reactions");
        let res = sync_all_custom_reactions(&db, &adapter, &storage_layout, |_s| {}).await?;

        progress.finish(&format!(
            "{} downloaded, {} already present, {} unavailable, {} failed",
            res.downloaded, res.already_existed, res.unavailable, res.failed
        ));

        let status = if res.failed > 0 {
            "failed"
        } else {
            "completed"
        };
        if json {
            print_media_json(
                status,
                MediaDownloadSummaryPayload {
                    completed_count: res.downloaded,
                    retry_wait_count: res.failed,
                    permanently_failed_count: res.unavailable,
                    needs_reauth_count: 0,
                },
            )?;
        } else {
            // TODO: perhaps it's better to use logging stuff to output smth like that?
            println!("==================================================");
            println!("CUSTOM REACTION SYNC COMPLETED");
            println!("==================================================");
            println!("Total Discovered:          {}", res.total_discovered);
            println!("Newly Downloaded:          {}", res.downloaded);
            println!("Already Present on Disk:   {}", res.already_existed);
            println!("Unavailable / Not Found:   {}", res.unavailable);
            println!("Failed Downloads:          {}", res.failed);
            println!("Total Bytes:               {}", res.total_bytes);
        }

        return Ok(SchedulerSummary::default());
    }

    progress.stage("Synchronizing peer and chat profile avatars");
    let avatar_summary = sync_all_peer_avatars(&db, &adapter, &storage_layout, |_s| {}).await?;

    debug!(
        "Avatar sync summary: {} discovered, {} downloaded, {} already existed, {} unavailable, {} failed",
        avatar_summary.avatars_discovered,
        avatar_summary.downloaded,
        avatar_summary.already_existed,
        avatar_summary.unavailable,
        avatar_summary.failed
    );

    if avatars_only {
        progress.finish(&format!(
            "{} downloaded, {} already present, {} unavailable, {} failed",
            avatar_summary.downloaded,
            avatar_summary.already_existed,
            avatar_summary.unavailable,
            avatar_summary.failed
        ));

        let status = if avatar_summary.failed > 0 {
            "failed"
        } else {
            "completed"
        };
        if json {
            print_media_json(
                status,
                MediaDownloadSummaryPayload {
                    completed_count: avatar_summary.downloaded,
                    retry_wait_count: avatar_summary.failed,
                    permanently_failed_count: avatar_summary.unavailable,
                    needs_reauth_count: 0,
                },
            )?;
        } else {
            // TODO: perhaps it's better to use logging stuff to output smth like that?
            println!("==================================================");
            println!("AVATARS DOWNLOAD PASS COMPLETED");
            println!("==================================================");
            println!("Total Peers Checked:       {}", avatar_summary.total_peers);
            println!(
                "Avatars Discovered:        {}",
                avatar_summary.avatars_discovered
            );
            println!("Newly Downloaded:          {}", avatar_summary.downloaded);
            println!(
                "Already Present on Disk:   {}",
                avatar_summary.already_existed
            );
            println!("Unavailable / No Photo:    {}", avatar_summary.unavailable);
            println!("Failed Downloads:          {}", avatar_summary.failed);
            println!("Total Bytes:               {}", avatar_summary.total_bytes);
        }

        return Ok(SchedulerSummary::default());
    }

    progress.stage("Synchronizing custom emoji reactions");
    let reaction_summary =
        sync_all_custom_reactions(&db, &adapter, &storage_layout, |_s| {}).await?;
    debug!(
        "Custom reaction sync summary: {} discovered, {} downloaded, {} already existed, {} unavailable, {} failed",
        reaction_summary.total_discovered,
        reaction_summary.downloaded,
        reaction_summary.already_existed,
        reaction_summary.unavailable,
        reaction_summary.failed
    );

    if backfill {
        progress.stage("Backfilling media objects from archived messages");
        let backfill_res = run_backfill_media(Arc::clone(&db), &MediaFilterPolicy::default())?;
        progress.update(format!(
            "Backfill: {} messages scanned, {} media discovered ({} eligible, {} skipped)",
            backfill_res.messages_scanned,
            backfill_res.media_discovered,
            backfill_res.media_eligible,
            backfill_res.media_skipped
        ));
    }

    progress.stage("Initializing media storage engine and scheduler");
    let engine = MediaEngine::new(
        db,
        adapter,
        root_path,
        concurrency.min_workers,
        concurrency.max_workers,
        concurrency.max_dc_workers,
        concurrency.initial_workers,
    );

    progress.stage("Running startup reconciliation across filesystem and SQLite state");
    let rec = engine.reconcile_startup()?;
    debug!("Reconciled states: {:?}", rec);
    if rec.missing_file_marked_count > 0 || rec.corrupted_file_marked_count > 0 {
        progress.update(format!(
            "Reconciliation: {} missing marked (re-queued for recovery), {} corrupted, {} downloading reset",
            rec.missing_file_marked_count,
            rec.corrupted_file_marked_count,
            rec.downloading_reset_count
        ));
    }

    progress.stage("Executing media download loop with dynamic concurrency");
    let queue_stats = engine
        .get_queue_stats()
        .unwrap_or_else(|_| MediaQueueStats::default());
    let total_eligible = queue_stats.eligible_count;
    let total_bytes = (queue_stats.all_sizes_known && queue_stats.expected_bytes > 0)
        .then_some(queue_stats.expected_bytes);

    let tracker = Arc::new(Mutex::new(MediaDownloadProgressTracker::new(
        total_eligible,
        total_bytes,
        progress.is_quiet(),
        progress.is_json(),
    )));

    let tracker_clone = Arc::clone(&tracker);
    let summary = engine
        .download_batch_with_progress("cli_worker", move |event| {
            if let Ok(mut t) = tracker_clone.lock() {
                t.on_progress(event);
            }
        })
        .await;

    if let Ok(t) = tracker.lock() {
        t.finish();
    }

    progress.finish(&format!(
        "{} completed, {} retry-wait, {} permanent failures",
        summary.completed_count, summary.retry_wait_count, summary.permanently_failed_count
    ));

    print_scheduler_summary(&summary, json)?;
    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
pub async fn run_download_media(
    api_id: Option<i32>,
    api_hash: Option<String>,
    session_path: &Path,
    archive_path: &Path,
    media_dir: &Path,
    concurrency: MediaDownloadConcurrency,
    backfill: bool,
    avatars_only: bool,
    reactions_only: bool,
    quiet: bool,
    json: bool,
) -> Result<SchedulerSummary> {
    let adapter = crate::adapter_factory::resolve_adapter(api_id, api_hash, session_path).await?;
    let db = Arc::new(ArchiveDb::open(archive_path).context("Failed to open archive database")?);

    let progress = CliProgress::new(quiet, json);
    run_download_media_with_adapter(
        db,
        adapter,
        media_dir.to_path_buf(),
        concurrency,
        backfill,
        avatars_only,
        reactions_only,
        progress,
        json,
    )
    .await
}

pub fn run_verify_media(
    db: Arc<ArchiveDb>,
    archive_root: impl Into<PathBuf>,
    quiet: bool,
    json: bool,
) -> Result<VerificationReport> {
    let layout = StorageLayoutManager::new(archive_root);
    let verifier = MediaVerifier::new(db, layout);
    let tracker = Mutex::new(None);

    let report = verifier.verify_all_completed_with_progress(|current, total, r| {
        if let Ok(mut guard) = tracker.lock() {
            let t =
                guard.get_or_insert_with(|| MediaVerifyProgressTracker::new(total, quiet, json));
            t.on_progress(current, total, r);
        }
    })?;

    if let Ok(Some(t)) = tracker.into_inner() {
        t.finish();
    }

    info!(
        "Verification report: {} checked, {} verified, {} missing, {} corrupted size, {} corrupted hash",
        report.total_checked,
        report.verified_count,
        report.missing_count,
        report.corrupted_size_count,
        report.corrupted_hash_count
    );
    Ok(report)
}

pub fn run_media_stats(db: &ArchiveDb) -> Result<MediaStats> {
    let stats = db.get_media_stats()?;
    Ok(stats)
}

pub fn run_requeue_skipped(db: &ArchiveDb, policy: &MediaFilterPolicy) -> Result<usize> {
    let count = db.requeue_skipped_media(policy)?;
    info!("Requeued {} skipped media items.", count);
    Ok(count)
}
