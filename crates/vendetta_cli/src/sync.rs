use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};
use vendetta_model::{PeerId, PeerType};
use vendetta_storage::ArchiveDb;
use vendetta_sync::pipeline::{CoordinatedSyncPipeline, FullSyncRunSummary};
use vendetta_tg_adapter::TelegramAdapter;

use crate::progress::{CliProgress, SyncProgressTracker};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRunOutput {
    pub schema_version: u32,
    pub command: String,
    pub status: String,
    pub archive: String,
    pub summary: SyncRunSummaryPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRunSummaryPayload {
    pub scope: String,
    pub requested_peers_count: usize,
    pub baseline_pts: i32,
    pub final_pts: i32,
    pub history_messages_ingested: usize,
    pub delta_messages_ingested: usize,
    pub edits_applied: usize,
    pub deletes_applied: usize,
    pub channels_synchronized: usize,
    pub failed_channels_count: usize,
    pub failed_channels: Vec<SyncFailedChannelEntry>,
}

impl SyncRunSummaryPayload {
    pub fn new(summary: &FullSyncRunSummary, is_explicit_scope: bool) -> Self {
        let failed_channels = summary
            .failed_channels
            .iter()
            .map(|(p, e)| SyncFailedChannelEntry {
                peer_id: p.raw(),
                error: e.clone(),
            })
            .collect::<Vec<_>>();

        let scope = if is_explicit_scope {
            "explicit_target"
        } else {
            "global_account"
        };

        Self {
            scope: scope.to_string(),
            requested_peers_count: summary.requested_peers_count,
            baseline_pts: summary.baseline_pts,
            final_pts: summary.final_pts,
            history_messages_ingested: summary.history_messages_ingested,
            delta_messages_ingested: summary.delta_messages_ingested,
            edits_applied: summary.edits_applied,
            deletes_applied: summary.deletes_applied,
            channels_synchronized: summary.channels_synchronized,
            failed_channels_count: failed_channels.len(),
            failed_channels,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncFailedChannelEntry {
    pub peer_id: i64,
    pub error: String,
}

fn print_sync_output(
    archive_path: &Path,
    summary: &FullSyncRunSummary,
    is_explicit_scope: bool,
    json: bool,
) -> Result<()> {
    if json {
        let status = if summary.failed_channels.is_empty() {
            "completed"
        } else {
            "completed_with_warnings"
        };

        let out = SyncRunOutput {
            schema_version: 1,
            command: "sync".to_string(),
            status: status.to_string(),
            archive: archive_path.display().to_string(),
            summary: SyncRunSummaryPayload::new(summary, is_explicit_scope),
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        // TODO: perhaps it's better to use logging stuff to output smth like that?
        let scope_label = if is_explicit_scope {
            "Explicit Target"
        } else {
            "Global Account"
        };
        let suffix = if summary.requested_peers_count == 1 {
            ""
        } else {
            "s"
        };

        println!("==================================================");
        println!("ARCHIVE SYNCHRONIZATION COMPLETED");
        println!("==================================================");
        println!("Archive Database:          {}", archive_path.display());
        println!(
            "Sync Scope:                {} ({} target{})",
            scope_label, summary.requested_peers_count, suffix
        );
        println!("Baseline PTS (S0):         {}", summary.baseline_pts);
        println!("Final PTS:                 {}", summary.final_pts);
        println!(
            "History Messages Ingested: {}",
            summary.history_messages_ingested
        );
        println!(
            "Delta Messages Ingested:   {}",
            summary.delta_messages_ingested
        );
        println!("Edits Applied:             {}", summary.edits_applied);
        println!("Deletions Applied:         {}", summary.deletes_applied);
        println!(
            "Channels Synchronized:     {}",
            summary.channels_synchronized
        );

        if !summary.failed_channels.is_empty() {
            println!(
                "Failed Channels:           {}",
                summary.failed_channels.len()
            );
            println!("--------------------------------------------------");
            println!("FAILED CHANNEL DETAILS:");
            for (pid, err) in &summary.failed_channels {
                println!("  - Channel ID {}: {}", pid.raw(), err);
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn run_sync_with_adapter(
    adapter: Arc<dyn TelegramAdapter>,
    db: Arc<ArchiveDb>,
    archive_path: &Path,
    target_peers: Option<Vec<PeerId>>,
    peer_type_filter: Option<PeerType>,
    exclude_peer_type: Option<PeerType>,
    limit: Option<usize>,
    mut progress: CliProgress,
    json: bool,
) -> Result<FullSyncRunSummary> {
    progress.stage("Step 1/4: Discovering dialogs and resolving target peers");
    let is_explicit_scope = target_peers.as_ref().is_some_and(|p| !p.is_empty());

    let mut dialogs = adapter
        .get_dialogs()
        .await
        .context("Failed to retrieve dialogs")?;

    for peer in &dialogs {
        db.upsert_peer(peer)?;
    }

    let target_peers = match target_peers {
        Some(peers) if !peers.is_empty() => peers,
        _ => {
            if let Some(inc) = peer_type_filter {
                dialogs.retain(|d| d.peer_type == inc);
            }
            if let Some(exc) = exclude_peer_type {
                dialogs.retain(|d| d.peer_type != exc);
            }
            if let Some(lim) = limit {
                dialogs.truncate(lim);
            }
            dialogs.into_iter().map(|d| d.peer_id).collect()
        }
    };
    progress.update(format!("Target peers selected: {}", target_peers.len()));

    progress.stage("Step 2/4: Initializing coordinated sync pipeline & baseline state S0");
    let pipeline = CoordinatedSyncPipeline::new(Arc::clone(&adapter), Arc::clone(&db));
    let mut sync_tracker = SyncProgressTracker::new(progress.is_quiet(), json);

    progress.stage("Step 3/4: Ingesting message history & reconciling delta stream");
    let summary = pipeline
        .run_full_sync_with_scope(&target_peers, is_explicit_scope, |event| {
            sync_tracker.on_progress(event);
        })
        .await
        .context("Full synchronization pipeline execution failed")?;

    sync_tracker.finish();

    progress.stage("Step 4/4: Finalizing archive commit and sync integrity");
    progress.finish(&format!(
        "{} history msgs, {} delta msgs, {} edits, {} deletes, {} channels",
        summary.history_messages_ingested,
        summary.delta_messages_ingested,
        summary.edits_applied,
        summary.deletes_applied,
        summary.channels_synchronized
    ));

    print_sync_output(archive_path, &summary, is_explicit_scope, json)?;
    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
pub async fn run_sync(
    api_id: Option<i32>,
    api_hash: Option<String>,
    session_path: &Path,
    archive_path: &Path,
    target_peers: Option<Vec<PeerId>>,
    peer_type_filter: Option<PeerType>,
    exclude_peer_type: Option<PeerType>,
    limit: Option<usize>,
    quiet: bool,
    json: bool,
) -> Result<FullSyncRunSummary> {
    let adapter = crate::adapter_factory::resolve_adapter(api_id, api_hash, session_path).await?;
    let db = Arc::new(ArchiveDb::open(archive_path).context("Failed to open archive database")?);

    let progress = CliProgress::new(quiet, json);
    run_sync_with_adapter(
        adapter,
        db,
        archive_path,
        target_peers,
        peer_type_filter,
        exclude_peer_type,
        limit,
        progress,
        json,
    )
    .await
}
