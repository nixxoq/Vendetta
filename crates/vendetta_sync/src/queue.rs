use std::{sync::Arc, time::Duration};

use tokio::time::sleep;
use tracing::{debug, error, warn};
use vendetta_core::now_unix_secs;
use vendetta_model::{ChannelQueueItem, ChannelQueueStatus, PeerId, PeerType, SyncIntegrityReport};
use vendetta_storage::ArchiveDb;
use vendetta_tg_adapter::{AdapterError, TelegramAdapter};

use crate::{
    diff::IncrementalSyncEngine,
    error::{SyncError, SyncResult},
};

pub struct ChannelQueueWorker<A: ?Sized + TelegramAdapter> {
    adapter: Arc<A>,
    storage: Arc<ArchiveDb>,
    sync_engine: Arc<IncrementalSyncEngine<A>>,
    flood_wait_scale: f64,
}

impl<A: ?Sized + TelegramAdapter> ChannelQueueWorker<A> {
    pub fn new(
        adapter: Arc<A>,
        storage: Arc<ArchiveDb>,
        sync_engine: Arc<IncrementalSyncEngine<A>>,
        _max_concurrency: usize,
    ) -> Self {
        Self {
            adapter,
            storage,
            sync_engine,
            flood_wait_scale: 1.0,
        }
    }

    pub fn new_with_scale(
        adapter: Arc<A>,
        storage: Arc<ArchiveDb>,
        sync_engine: Arc<IncrementalSyncEngine<A>>,
        flood_wait_scale: f64,
    ) -> Self {
        Self {
            adapter,
            storage,
            sync_engine,
            flood_wait_scale,
        }
    }

    pub fn max_concurrency(&self) -> usize {
        1
    }

    pub fn flood_wait_scale(&self) -> f64 {
        self.flood_wait_scale
    }

    pub async fn discover_and_enqueue_stale_channels(&self) -> SyncResult<usize> {
        self.discover_and_enqueue_stale_channels_filtered(None)
            .await
    }

    pub async fn discover_and_enqueue_stale_channels_filtered(
        &self,
        target_filter: Option<&[PeerId]>,
    ) -> SyncResult<usize> {
        let local_channels = match target_filter {
            Some(filter) => {
                let mut channel_targets = Vec::new();
                for &peer_id in filter {
                    let is_channel = self
                        .storage
                        .get_peer(peer_id)?
                        .is_some_and(|p| p.peer_type == PeerType::Channel)
                        || peer_id.raw() < 0;
                    if is_channel {
                        channel_targets.push(peer_id);
                    }
                }
                if channel_targets.is_empty() {
                    debug!(
                        "No channel peers in explicit target filter; skipping channel discovery"
                    );
                    return Ok(0);
                }
                channel_targets
            }
            None => self.storage.list_archived_channel_peer_ids()?,
        };

        debug!(
            local_channel_count = local_channels.len(),
            is_filtered = target_filter.is_some(),
            "Starting channel discovery"
        );

        let discovery = match self.adapter.get_all_dialogs_complete(&local_channels).await {
            Ok(d) => d,
            Err(e) => {
                warn!(
                    error = %e,
                    "Dialog enumeration failed partially; proceeding with degraded discovery"
                );
                self.storage
                    .record_sync_integrity_report(&SyncIntegrityReport {
                        scope: "channel_discovery".to_string(),
                        peer_id: None,
                        fully_lossless_contiguous_sync: false,
                        current_history_repaired: false,
                        new_messages_recovered: false,
                        current_content_reconciled: false,
                        historical_edits_complete: false,
                        historical_deletions_complete: false,
                        event_window_lost: false,
                        channel_discovery_complete: false,
                        gap_summary: Some(format!("Dialog discovery failed: {e}")),
                        created_at: now_unix_secs(),
                        provenance_version: 2,
                        deletion_reconciliation_performed: false,
                        deletion_reconciliation_complete: false,
                        deletion_event_gap_count: 0,
                        deletion_tombstones_reconciled: 0,
                        historical_message_reconciliation_performed: false,
                        historical_message_reconciliation_complete: false,
                        historical_message_gap_count: 0,
                    })?;
                return Err(SyncError::ChannelDiscoveryIncomplete(e.to_string()));
            }
        };

        let now = now_unix_secs();
        if !discovery.is_complete {
            warn!("Dialog discovery completed with degraded/partial completeness flag");
            self.storage
                .record_sync_integrity_report(&SyncIntegrityReport {
                    scope: "channel_discovery".to_string(),
                    peer_id: None,
                    fully_lossless_contiguous_sync: false,
                    current_history_repaired: false,
                    new_messages_recovered: false,
                    current_content_reconciled: false,
                    historical_edits_complete: false,
                    historical_deletions_complete: false,
                    event_window_lost: false,
                    channel_discovery_complete: false,
                    gap_summary: Some(
                        "Dialog discovery incomplete due to partial RPC failures; unresolved channels flagged."
                            .to_string(),
                    ),
                    created_at: now,
                    provenance_version: 2,
                    deletion_reconciliation_performed: false,
                    deletion_reconciliation_complete: false,
                    deletion_event_gap_count: 0,
                    deletion_tombstones_reconciled: 0,
                    historical_message_reconciliation_performed: false,
                    historical_message_reconciliation_complete: false,
                    historical_message_gap_count: 0,
                })?;
        }

        let mut enqueued = 0;

        for diag in discovery.discovered_dialogs {
            if let Some(filter) = target_filter
                && !filter.contains(&diag.peer_id)
            {
                continue;
            }

            let is_channel = diag.peer_type == Some(PeerType::Channel)
                || self
                    .storage
                    .get_peer(diag.peer_id)?
                    .is_some_and(|p| p.peer_type == PeerType::Channel);

            if is_channel {
                if diag.is_unresolved || diag.pts.is_none() {
                    debug!(
                        channel_id = diag.peer_id.raw(),
                        "Channel is unresolved or missing authoritative server PTS; marking BLOCKED"
                    );
                    self.storage.enqueue_channel(&ChannelQueueItem {
                        peer_id: diag.peer_id,
                        discovered_pts: 0,
                        current_pts: None,
                        status: ChannelQueueStatus::Blocked,
                        attempts: 0,
                        poll_timeout: None,
                        last_error: Some(
                            "Dialog discovery unresolved or missing authoritative server PTS"
                                .to_string(),
                        ),
                        updated_at: now,
                    })?;
                    continue;
                }

                let server_pts = diag.pts.unwrap();
                let local_state = self.storage.get_peer_sync_state(diag.peer_id)?;
                let local_pts = local_state.and_then(|s| s.pts);

                match local_pts {
                    Some(lp) => {
                        if server_pts > lp {
                            debug!(
                                channel_id = diag.peer_id.raw(),
                                server_pts,
                                local_pts = lp,
                                "Enqueuing stale channel for sync"
                            );

                            self.storage.enqueue_channel(&ChannelQueueItem {
                                peer_id: diag.peer_id,
                                discovered_pts: server_pts,
                                current_pts: Some(lp),
                                status: ChannelQueueStatus::Pending,
                                attempts: 0,
                                poll_timeout: None,
                                last_error: None,
                                updated_at: now,
                            })?;
                            enqueued += 1;
                        }
                    }
                    None => {
                        debug!(
                            channel_id = diag.peer_id.raw(),
                            server_pts,
                            "Fresh channel discovered with authoritative server PTS; initializing baseline cursor"
                        );

                        self.storage.enqueue_channel(&ChannelQueueItem {
                            peer_id: diag.peer_id,
                            discovered_pts: server_pts,
                            current_pts: Some(server_pts),
                            status: ChannelQueueStatus::Pending,
                            attempts: 0,
                            poll_timeout: None,
                            last_error: None,
                            updated_at: now,
                        })?;
                        enqueued += 1;
                    }
                }
            }
        }

        debug!(enqueued, "Stale channel discovery completed");
        Ok(enqueued)
    }

    pub async fn process_queue(&self) -> SyncResult<usize> {
        self.process_queue_filtered(None)
            .await
            .map(|s| s.processed_count)
    }

    pub async fn process_queue_filtered(
        &self,
        target_filter: Option<&[PeerId]>,
    ) -> SyncResult<ChannelQueueProcessSummary> {
        let recovered_stale = self.storage.reset_stale_in_progress_channels()?;
        if recovered_stale > 0 {
            debug!(
                recovered_stale,
                "Reset stale in_progress channel queue items to pending after restart"
            );
        }

        let mut summary = ChannelQueueProcessSummary::default();

        loop {
            let next_item = self
                .storage
                .pop_next_pending_channel_filtered(target_filter)?;
            let item = match next_item {
                Some(it) => it,
                None => break,
            };

            debug!(
                channel_id = item.peer_id.raw(),
                discovered_pts = item.discovered_pts,
                attempts = item.attempts,
                "Processing queued channel"
            );

            match self
                .sync_engine
                .sync_channel(item.peer_id, item.current_pts)
                .await
            {
                Ok(chan_res) => {
                    summary.processed_count += 1;
                    summary.new_messages_ingested += chan_res.new_messages_ingested;
                    summary.edits_applied += chan_res.edits_applied;
                    summary.deletes_applied += chan_res.deletes_applied;
                }
                Err(SyncError::Adapter(AdapterError::FloodWait { seconds })) => {
                    warn!(
                        channel_id = item.peer_id.raw(),
                        seconds, "FLOOD_WAIT encountered during channel sync; pausing and retrying"
                    );

                    self.storage.enqueue_channel(&ChannelQueueItem {
                        peer_id: item.peer_id,
                        discovered_pts: item.discovered_pts,
                        current_pts: item.current_pts,
                        status: ChannelQueueStatus::Pending,
                        attempts: item.attempts,
                        poll_timeout: Some(seconds as i32),
                        last_error: Some(format!("FLOOD_WAIT {seconds}s")),
                        updated_at: now_unix_secs(),
                    })?;

                    let wait_secs = (seconds as f64 + 1.0) * self.flood_wait_scale;
                    let wait_dur = Duration::from_secs_f64(wait_secs.max(0.001));
                    warn!(
                        channel_id = item.peer_id.raw(),
                        wait_seconds = wait_secs,
                        "Backing off for FLOOD_WAIT duration"
                    );
                    sleep(wait_dur).await;
                }
                Err(e) => {
                    error!(
                        channel_id = item.peer_id.raw(),
                        error = %e,
                        "Channel sync failed"
                    );
                    self.storage.mark_channel_queue_failed(
                        item.peer_id,
                        &e.to_string(),
                        now_unix_secs(),
                    )?;
                    summary.failed_channels.push((item.peer_id, e.to_string()));
                }
            }
        }

        Ok(summary)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChannelQueueProcessSummary {
    pub processed_count: usize,
    pub new_messages_ingested: usize,
    pub edits_applied: usize,
    pub deletes_applied: usize,
    pub failed_channels: Vec<(PeerId, String)>,
}
