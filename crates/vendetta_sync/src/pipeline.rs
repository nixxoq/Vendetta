use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use tracing::{debug, warn};
use vendetta_core::now_unix_secs;
use vendetta_model::{PeerId, SyncBaseline, SyncBaselineStatus, SyncIntegrityReport};
use vendetta_storage::ArchiveDb;
use vendetta_tg_adapter::TelegramAdapter;

use crate::{
    diff::IncrementalSyncEngine, error::SyncResult, ingest::HistoryIngestionPipeline,
    queue::ChannelQueueWorker,
};

static BASELINE_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Default)]
pub struct FullSyncRunSummary {
    pub baseline_pts: i32,
    pub history_messages_ingested: usize,
    pub delta_messages_ingested: usize,
    pub edits_applied: usize,
    pub deletes_applied: usize,
    pub channels_synchronized: usize,
    pub final_pts: i32,
    pub integrity: Option<SyncIntegrityReport>,
    pub requested_peers_count: usize,
    pub failed_channels: Vec<(PeerId, String)>,
    pub is_explicit_scope: bool,
}

impl FullSyncRunSummary {
    pub fn is_clean(&self) -> bool {
        self.failed_channels.is_empty()
            && self
                .integrity
                .as_ref()
                .is_none_or(|i| i.fully_lossless_contiguous_sync)
    }

    pub fn is_requested_scope_clean(&self) -> bool {
        self.failed_channels.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStep {
    DiscoveringDialogs,
    CapturingBaseline,
    IngestingHistory,
    ReconcilingUpdates,
    ChannelDiscovery,
    ChannelQueue,
    Finalizing,
}

impl SyncStep {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DiscoveringDialogs => "Discovering dialogs",
            Self::CapturingBaseline => "Capturing baseline state",
            Self::IngestingHistory => "Ingesting history",
            Self::ReconcilingUpdates => "Reconciling updates",
            Self::ChannelDiscovery => "Discovering channels",
            Self::ChannelQueue => "Synchronizing channel queues",
            Self::Finalizing => "Finalizing archive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncProgressEvent {
    pub step: SyncStep,
    pub peer_index: usize,
    pub total_peers: usize,
    pub current_peer_id: Option<PeerId>,
    pub current_peer_name: Option<String>,
    pub current_peer_messages: usize,
    pub total_messages_processed: usize,
    pub total_batches_completed: usize,
    pub flood_wait_seconds: Option<u32>,
    pub status_detail: Option<String>,
}

fn emit_step_progress<F>(
    on_progress: &mut F,
    total_peers: usize,
    step: SyncStep,
    processed: usize,
    batches: usize,
    detail: &'static str,
) where
    F: FnMut(&SyncProgressEvent),
{
    on_progress(&SyncProgressEvent {
        step,
        peer_index: total_peers,
        total_peers,
        current_peer_id: None,
        current_peer_name: None,
        current_peer_messages: 0,
        total_messages_processed: processed,
        total_batches_completed: batches,
        flood_wait_seconds: None,
        status_detail: Some(detail.to_string()),
    });
}

pub struct CoordinatedSyncPipeline<A: ?Sized + TelegramAdapter> {
    adapter: Arc<A>,
    storage: Arc<ArchiveDb>,
    sync_engine: Arc<IncrementalSyncEngine<A>>,
    queue_worker: Arc<ChannelQueueWorker<A>>,
    history_pipeline: HistoryIngestionPipeline,
}

impl<A: ?Sized + TelegramAdapter> CoordinatedSyncPipeline<A> {
    pub fn new(adapter: Arc<A>, storage: Arc<ArchiveDb>) -> Self {
        let sync_engine = Arc::new(IncrementalSyncEngine::new(
            Arc::clone(&adapter),
            Arc::clone(&storage),
        ));
        let queue_worker = Arc::new(ChannelQueueWorker::new(
            Arc::clone(&adapter),
            Arc::clone(&storage),
            Arc::clone(&sync_engine),
            1,
        ));
        let history_pipeline = HistoryIngestionPipeline::new(100);

        Self {
            adapter,
            storage,
            sync_engine,
            queue_worker,
            history_pipeline,
        }
    }

    pub async fn run_full_sync(&self, target_peers: &[PeerId]) -> SyncResult<FullSyncRunSummary> {
        let is_explicit = !target_peers.is_empty();
        self.run_full_sync_with_scope(target_peers, is_explicit, |_| {})
            .await
    }

    pub async fn run_full_sync_with_progress<F>(
        &self,
        target_peers: &[PeerId],
        on_progress: F,
    ) -> SyncResult<FullSyncRunSummary>
    where
        F: FnMut(&SyncProgressEvent),
    {
        let is_explicit = !target_peers.is_empty();
        self.run_full_sync_with_scope(target_peers, is_explicit, on_progress)
            .await
    }

    pub async fn run_full_sync_with_scope<F>(
        &self,
        target_peers: &[PeerId],
        is_explicit_scope: bool,
        mut on_progress: F,
    ) -> SyncResult<FullSyncRunSummary>
    where
        F: FnMut(&SyncProgressEvent),
    {
        let mut summary = FullSyncRunSummary::default();
        let now = now_unix_secs();
        let seq = BASELINE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);

        let total_peers_count = target_peers.len();
        let mut total_batches_completed = 0;

        emit_step_progress(
            &mut on_progress,
            total_peers_count,
            SyncStep::CapturingBaseline,
            0,
            0,
            "Capturing baseline S0",
        );

        debug!("Step 1: Capturing pre-scan baseline state S0");
        let initial_state = self.adapter.get_state().await?;
        let pid = std::process::id();
        let baseline_id = format!(
            "baseline_{}_{}_{}_{}_{}",
            initial_state.pts, now, nanos, pid, seq
        );
        let baseline = SyncBaseline {
            baseline_id: baseline_id.clone(),
            common_pts: initial_state.pts,
            common_qts: initial_state.qts,
            common_date: initial_state.date,
            common_seq: initial_state.seq,
            status: SyncBaselineStatus::InProgress,
            captured_at: now,
            completed_at: None,
        };
        self.storage.record_sync_baseline(&baseline)?;
        self.storage.upsert_account_sync_state(&initial_state)?;
        summary.baseline_pts = initial_state.pts;

        debug!("Step 2: Ingesting message history across target peers");
        for (idx, &peer_id) in target_peers.iter().enumerate() {
            let peer_index = idx + 1;
            let peer_name = self
                .storage
                .get_peer(peer_id)
                .ok()
                .flatten()
                .and_then(|p| p.name);

            on_progress(&SyncProgressEvent {
                step: SyncStep::IngestingHistory,
                peer_index,
                total_peers: total_peers_count,
                current_peer_id: Some(peer_id),
                current_peer_name: peer_name.clone(),
                current_peer_messages: 0,
                total_messages_processed: summary.history_messages_ingested,
                total_batches_completed,
                flood_wait_seconds: None,
                status_detail: None,
            });

            let hist_summary = self
                .history_pipeline
                .ingest_history_with_progress(
                    self.adapter.as_ref(),
                    self.storage.as_ref(),
                    peer_id,
                    |batch_progress| {
                        let processed = summary.history_messages_ingested
                            + batch_progress.current_peer_messages_count;
                        total_batches_completed += 1;
                        on_progress(&SyncProgressEvent {
                            step: SyncStep::IngestingHistory,
                            peer_index,
                            total_peers: total_peers_count,
                            current_peer_id: Some(peer_id),
                            current_peer_name: peer_name.clone(),
                            current_peer_messages: batch_progress.current_peer_messages_count,
                            total_messages_processed: processed,
                            total_batches_completed,
                            flood_wait_seconds: None,
                            status_detail: None,
                        });
                    },
                )
                .await?;

            summary.history_messages_ingested += hist_summary.messages_ingested;
        }

        emit_step_progress(
            &mut on_progress,
            total_peers_count,
            SyncStep::ReconcilingUpdates,
            summary.history_messages_ingested,
            total_batches_completed,
            "Reconciling updates from baseline S0",
        );

        debug!("Step 3: Reconciling delta updates from baseline S0");
        let common_summary = self.sync_engine.sync_common().await?;

        summary.delta_messages_ingested = common_summary.new_messages_ingested;
        summary.edits_applied = common_summary.edits_applied;
        summary.deletes_applied = common_summary.deletes_applied;
        summary.final_pts = common_summary.final_pts;

        self.storage
            .complete_sync_baseline(&baseline.baseline_id, now_unix_secs())?;

        let total_processed = summary.history_messages_ingested + summary.delta_messages_ingested;
        emit_step_progress(
            &mut on_progress,
            total_peers_count,
            SyncStep::ChannelDiscovery,
            total_processed,
            total_batches_completed,
            "Discovering modified channels",
        );

        let target_channel_filter = is_explicit_scope.then_some(target_peers);

        debug!("Step 4: Discovering and synchronizing modified channels");
        let discovery_res = self
            .queue_worker
            .discover_and_enqueue_stale_channels_filtered(target_channel_filter)
            .await;

        let blocked_channels_count = self
            .storage
            .count_blocked_or_failed_channels_filtered(target_channel_filter)?;
        let channel_discovery_complete = match discovery_res {
            Ok(_) if blocked_channels_count == 0 => true,
            Ok(_) => {
                warn!(
                    blocked_channels_count,
                    "Channel discovery finished with blocked/unresolved channels remaining in queue"
                );
                false
            }
            Err(e) => {
                warn!("Channel discovery failed or was incomplete during full sync run: {e}");
                false
            }
        };

        emit_step_progress(
            &mut on_progress,
            total_peers_count,
            SyncStep::ChannelQueue,
            total_processed,
            total_batches_completed,
            "Processing channel queues",
        );

        let queue_res = self
            .queue_worker
            .process_queue_filtered(target_channel_filter)
            .await?;
        summary.channels_synchronized = queue_res.processed_count;
        summary.delta_messages_ingested += queue_res.new_messages_ingested;
        summary.edits_applied += queue_res.edits_applied;
        summary.deletes_applied += queue_res.deletes_applied;
        summary.failed_channels = queue_res.failed_channels;
        summary.requested_peers_count = total_peers_count;
        summary.is_explicit_scope = is_explicit_scope;

        let remaining_unresolved = self
            .storage
            .count_blocked_or_failed_channels_filtered(target_channel_filter)?;
        let final_discovery_complete = channel_discovery_complete && remaining_unresolved == 0;

        let fully_lossless = !common_summary.had_buffer_overflow && final_discovery_complete;
        let integrity = SyncIntegrityReport {
            scope: if is_explicit_scope {
                "explicit_peers_sync".to_string()
            } else {
                "full_sync_run".to_string()
            },
            peer_id: None,
            fully_lossless_contiguous_sync: fully_lossless,
            current_history_repaired: true,
            new_messages_recovered: true,
            current_content_reconciled: true,
            historical_edits_complete: !common_summary.had_buffer_overflow,
            historical_deletions_complete: false,
            event_window_lost: common_summary.had_buffer_overflow,
            channel_discovery_complete: final_discovery_complete,
            gap_summary: if common_summary.had_buffer_overflow {
                Some("Full export completed with differenceTooLong recovery; intermediate edit history and unverified deletions in gap were purged by server.".to_string())
            } else if !final_discovery_complete {
                Some(format!(
                    "Sync completed but channel discovery/synchronization had {remaining_unresolved} unresolved or failed channels."
                ))
            } else {
                None
            },
            created_at: now_unix_secs(),
            provenance_version: 2,
            deletion_reconciliation_performed: false,
            deletion_reconciliation_complete: false,
            deletion_event_gap_count: 0,
            deletion_tombstones_reconciled: 0,
            historical_message_reconciliation_performed: false,
            historical_message_reconciliation_complete: false,
            historical_message_gap_count: 0,
        };
        self.storage.record_sync_integrity_report(&integrity)?;
        summary.integrity = Some(integrity);

        emit_step_progress(
            &mut on_progress,
            total_peers_count,
            SyncStep::Finalizing,
            summary.history_messages_ingested + summary.delta_messages_ingested,
            total_batches_completed,
            "Sync run complete",
        );

        if summary.failed_channels.is_empty() {
            debug!(
                history_msgs = summary.history_messages_ingested,
                delta_msgs = summary.delta_messages_ingested,
                final_pts = summary.final_pts,
                "Full synchronized archive run completed successfully"
            );
        } else {
            warn!(
                failed_channels = summary.failed_channels.len(),
                "Synchronized archive run completed with failed channels"
            );
        }

        Ok(summary)
    }
}
