use std::sync::Arc;

use tracing::{debug, warn};
use vendetta_core::now_unix_secs;
use vendetta_model::{
    AccountSyncState, MessageId, MessageRecord, NormalizedUpdate, PeerId, PeerRecord,
    PeerSyncState, SyncIntegrityReport,
};
use vendetta_storage::ArchiveDb;
use vendetta_tg_adapter::{ChannelDifferenceResult, CommonDifferenceResult, TelegramAdapter};

use crate::error::{SyncError, SyncResult};

#[derive(Debug, Clone, Default)]
pub struct CommonSyncSummary {
    pub slices_processed: usize,
    pub new_messages_ingested: usize,
    pub edits_applied: usize,
    pub deletes_applied: usize,
    pub channel_catchups_queued: usize,
    pub final_pts: i32,
    pub had_buffer_overflow: bool,
}

#[derive(Debug, Clone)]
pub struct ChannelSyncSummary {
    pub state: PeerSyncState,
    pub new_messages_ingested: usize,
    pub edits_applied: usize,
    pub deletes_applied: usize,
}

pub struct IncrementalSyncEngine<A: ?Sized + TelegramAdapter> {
    adapter: Arc<A>,
    storage: Arc<ArchiveDb>,
}

impl<A: ?Sized + TelegramAdapter> IncrementalSyncEngine<A> {
    pub fn new(adapter: Arc<A>, storage: Arc<ArchiveDb>) -> Self {
        Self { adapter, storage }
    }

    pub async fn sync_common(&self) -> SyncResult<CommonSyncSummary> {
        let mut summary = CommonSyncSummary::default();

        let mut current_state = match self.storage.get_account_sync_state("default")? {
            Some(st) => st,
            None => {
                let initial = self.adapter.get_state().await?;
                self.storage.upsert_account_sync_state(&initial)?;
                initial
            }
        };

        let mut recovery_attempts: u32 = 0;

        loop {
            debug!(
                pts = current_state.pts,
                date = current_state.date,
                qts = current_state.qts,
                sync_uncertain = current_state.sync_uncertain,
                "Querying updates.getDifference from last safe state"
            );

            let result = self
                .adapter
                .get_difference(current_state.pts, current_state.date, current_state.qts)
                .await?;

            match result {
                CommonDifferenceResult::Empty { date, seq } => {
                    debug!(
                        date,
                        seq, "Received differenceEmpty; common sync up to date"
                    );
                    current_state.date = date;
                    current_state.seq = seq;
                    current_state.sync_uncertain = false;
                    current_state.last_synced_at = now_unix_secs();
                    self.storage.upsert_account_sync_state(&current_state)?;
                    break;
                }
                CommonDifferenceResult::Slice {
                    new_messages,
                    other_updates,
                    auxiliary_peers,
                    mut intermediate_state,
                } => {
                    summary.slices_processed += 1;
                    if let Some(unsupported) = self.find_unsupported_state_affecting(&other_updates)
                    {
                        self.handle_unsupported_common(
                            unsupported,
                            &current_state,
                            &mut recovery_attempts,
                        )?;
                        continue;
                    }

                    intermediate_state.sync_uncertain = false;
                    let msg_count = self.storage.apply_common_difference_slice(
                        &new_messages,
                        &other_updates,
                        &auxiliary_peers,
                        &intermediate_state,
                    )?;

                    summary.new_messages_ingested += msg_count;
                    self.tally_updates(&other_updates, &mut summary);
                    current_state = intermediate_state;
                    recovery_attempts = 0;
                }
                CommonDifferenceResult::Difference {
                    new_messages,
                    other_updates,
                    auxiliary_peers,
                    mut state,
                } => {
                    summary.slices_processed += 1;
                    if let Some(unsupported) = self.find_unsupported_state_affecting(&other_updates)
                    {
                        self.handle_unsupported_common(
                            unsupported,
                            &current_state,
                            &mut recovery_attempts,
                        )?;
                        continue;
                    }

                    state.sync_uncertain = false;
                    let msg_count = self.storage.apply_common_difference_slice(
                        &new_messages,
                        &other_updates,
                        &auxiliary_peers,
                        &state,
                    )?;

                    summary.new_messages_ingested += msg_count;
                    self.tally_updates(&other_updates, &mut summary);
                    current_state = state;
                    debug!(
                        pts = current_state.pts,
                        "Completed common difference synchronization successfully"
                    );
                    break;
                }
                CommonDifferenceResult::TooLong { pts } => {
                    warn!(
                        old_pts = current_state.pts,
                        new_pts = pts,
                        "Common difference ring buffer overflow (differenceTooLong)"
                    );
                    summary.had_buffer_overflow = true;

                    let final_state = self.recover_common_difference_too_long(pts).await?;
                    current_state = final_state;
                    break;
                }
            }
        }

        summary.final_pts = current_state.pts;
        Ok(summary)
    }

    fn handle_unsupported_common(
        &self,
        unsupported: &NormalizedUpdate,
        current_state: &AccountSyncState,
        recovery_attempts: &mut u32,
    ) -> SyncResult<()> {
        let now = now_unix_secs();
        if let NormalizedUpdate::Unsupported {
            constructor_id,
            pts,
            pts_count,
            qts,
            qts_count,
            diagnostic_info,
            raw_tl,
            ..
        } = unsupported
        {
            self.storage
                .persist_account_unsupported_event_and_mark_uncertain(
                    *constructor_id,
                    *pts,
                    Some(*pts_count),
                    *qts,
                    Some(*qts_count),
                    diagnostic_info.as_deref(),
                    raw_tl,
                    current_state,
                    now,
                )?;

            if qts.is_some() && pts.is_none() {
                warn!(
                    constructor_id,
                    qts,
                    "Encountered unsupported QTS/secret-chat update. Secret-chat sync is deferred in Milestone 4."
                );
                return Err(SyncError::UnsupportedStateAffectingUpdate {
                    constructor_id: *constructor_id,
                    pts: None,
                    pts_count: 0,
                });
            }

            if *recovery_attempts >= 1 {
                return Err(SyncError::UnsupportedStateAffectingUpdate {
                    constructor_id: *constructor_id,
                    pts: *pts,
                    pts_count: *pts_count,
                });
            }

            *recovery_attempts += 1;
            warn!(
                constructor_id,
                pts = current_state.pts,
                "Encountered unsupported state-affecting update. Retrying difference from last safe cursor."
            );
        }
        Ok(())
    }

    pub async fn sync_channel(
        &self,
        channel_id: PeerId,
        known_pts: Option<i32>,
    ) -> SyncResult<ChannelSyncSummary> {
        let mut current_pts = match known_pts {
            Some(p) if p >= 1 => p,
            _ => {
                let existing = self.storage.get_peer_sync_state(channel_id)?;
                match existing.and_then(|s| s.pts) {
                    Some(p) if p >= 1 => p,
                    _ => {
                        return Err(SyncError::MissingChannelBaseline {
                            peer_id: channel_id,
                        });
                    }
                }
            }
        };

        let mut recovery_attempts: u32 = 0;
        let mut new_messages_ingested = 0;
        let mut edits_applied = 0;
        let mut deletes_applied = 0;

        loop {
            debug!(
                channel_id = channel_id.raw(),
                pts = current_pts,
                "Querying updates.getChannelDifference"
            );

            let result = self
                .adapter
                .get_channel_difference(channel_id, current_pts, 100)
                .await?;

            match result {
                ChannelDifferenceResult::Empty {
                    final_state: _,
                    pts,
                    timeout,
                } => {
                    debug!(
                        channel_id = channel_id.raw(),
                        pts, "Received channelDifferenceEmpty"
                    );
                    let now = now_unix_secs();
                    let state = PeerSyncState {
                        peer_id: channel_id,
                        pts: Some(pts),
                        min_message_id: None,
                        max_message_id: None,
                        has_gap: false,
                        sync_uncertain: false,
                        poll_timeout_secs: timeout,
                        last_synced_at: now,
                    };
                    self.storage
                        .complete_channel_sync_and_queue(channel_id, pts, timeout, now)?;
                    return Ok(ChannelSyncSummary {
                        state,
                        new_messages_ingested,
                        edits_applied,
                        deletes_applied,
                    });
                }
                ChannelDifferenceResult::Difference {
                    final_state,
                    pts,
                    timeout,
                    new_messages,
                    other_updates,
                    auxiliary_peers,
                } => {
                    if let Some(unsupported) = self.find_unsupported_state_affecting(&other_updates)
                    {
                        let now = now_unix_secs();
                        if let NormalizedUpdate::Unsupported {
                            constructor_id,
                            pts: u_pts,
                            pts_count,
                            qts,
                            qts_count,
                            diagnostic_info,
                            raw_tl,
                            ..
                        } = unsupported
                        {
                            self.storage
                                .persist_channel_unsupported_event_and_mark_uncertain(
                                    channel_id,
                                    *constructor_id,
                                    *u_pts,
                                    Some(*pts_count),
                                    *qts,
                                    Some(*qts_count),
                                    diagnostic_info.as_deref(),
                                    raw_tl,
                                    current_pts,
                                    timeout,
                                    now,
                                )?;

                            if recovery_attempts >= 1 {
                                return Err(SyncError::UnsupportedStateAffectingUpdate {
                                    constructor_id: *constructor_id,
                                    pts: *u_pts,
                                    pts_count: *pts_count,
                                });
                            }
                            recovery_attempts += 1;
                            warn!(
                                constructor_id,
                                pts = current_pts,
                                "Encountered unsupported channel state-affecting update. Retrying channel difference from last safe pts."
                            );
                            continue;
                        }
                    }

                    new_messages_ingested += new_messages.len();
                    for upd in &other_updates {
                        match upd {
                            NormalizedUpdate::EditedMessage { .. } => edits_applied += 1,
                            NormalizedUpdate::CommonDeletedMessages { message_ids, .. }
                            | NormalizedUpdate::ChannelDeletedMessages { message_ids, .. } => {
                                deletes_applied += message_ids.len();
                            }
                            NormalizedUpdate::NewMessage { .. } => new_messages_ingested += 1,
                            _ => {}
                        }
                    }

                    self.storage.apply_channel_difference_slice(
                        channel_id,
                        &new_messages,
                        &other_updates,
                        &auxiliary_peers,
                        pts,
                        timeout,
                    )?;

                    current_pts = pts;
                    recovery_attempts = 0;

                    if final_state {
                        debug!(
                            channel_id = channel_id.raw(),
                            pts = current_pts,
                            "Completed channel difference synchronization"
                        );
                        let now = now_unix_secs();
                        let state = PeerSyncState {
                            peer_id: channel_id,
                            pts: Some(current_pts),
                            min_message_id: None,
                            max_message_id: None,
                            has_gap: false,
                            sync_uncertain: false,
                            poll_timeout_secs: timeout,
                            last_synced_at: now,
                        };
                        self.storage.complete_channel_sync_and_queue(
                            channel_id,
                            current_pts,
                            timeout,
                            now,
                        )?;
                        return Ok(ChannelSyncSummary {
                            state,
                            new_messages_ingested,
                            edits_applied,
                            deletes_applied,
                        });
                    }
                }
                ChannelDifferenceResult::TooLong {
                    dialog_pts,
                    top_message,
                    messages,
                    auxiliary_peers,
                    timeout,
                    ..
                } => {
                    warn!(
                        channel_id = channel_id.raw(),
                        dialog_pts,
                        "Channel difference ring buffer overflow (channelDifferenceTooLong)"
                    );

                    let recovered_state = self
                        .recover_channel_difference_too_long(
                            channel_id,
                            dialog_pts,
                            top_message,
                            messages,
                            auxiliary_peers,
                            timeout,
                        )
                        .await?;
                    return Ok(ChannelSyncSummary {
                        state: recovered_state,
                        new_messages_ingested,
                        edits_applied,
                        deletes_applied,
                    });
                }
            }
        }
    }

    async fn recover_common_difference_too_long(
        &self,
        overflow_pts: i32,
    ) -> SyncResult<AccountSyncState> {
        let fresh_state = self.adapter.get_state().await?;
        let recovery_pts = fresh_state.pts;

        let common_peers = self.storage.list_archived_common_peer_ids()?;
        for peer_id in common_peers {
            let local_max = self.storage.get_peer_max_message_id(peer_id)?;
            let mut offset_id = None;
            loop {
                let page = self
                    .adapter
                    .get_history_page_filtered(peer_id, local_max, None, offset_id, 100)
                    .await?;

                if page.messages.is_empty() {
                    break;
                }

                let Some(batch_min_id) = page.messages.iter().map(|m| m.key.message_id).min()
                else {
                    break;
                };

                self.storage.ingest_history_page(
                    peer_id,
                    &page.messages,
                    &page.auxiliary_peers,
                    None,
                )?;

                if local_max.map(|m| batch_min_id <= m).unwrap_or(false)
                    || page.messages.len() < 100
                {
                    break;
                }
                offset_id = Some(batch_min_id);
            }
        }

        let mut rec_pts = fresh_state.pts;
        let mut rec_date = fresh_state.date;
        let mut rec_qts = fresh_state.qts;
        let mut final_state = fresh_state.clone();

        loop {
            let rec_diff = self
                .adapter
                .get_difference(rec_pts, rec_date, rec_qts)
                .await?;

            match rec_diff {
                CommonDifferenceResult::Empty { date, seq } => {
                    final_state.date = date;
                    final_state.seq = seq;
                    final_state.sync_uncertain = false;
                    final_state.last_synced_at = now_unix_secs();
                    break;
                }
                CommonDifferenceResult::Slice {
                    new_messages,
                    other_updates,
                    auxiliary_peers,
                    mut intermediate_state,
                } => {
                    if let Some(NormalizedUpdate::Unsupported {
                        constructor_id,
                        pts,
                        pts_count,
                        ..
                    }) = self.find_unsupported_state_affecting(&other_updates)
                    {
                        return Err(SyncError::UnsupportedStateAffectingUpdate {
                            constructor_id: *constructor_id,
                            pts: *pts,
                            pts_count: *pts_count,
                        });
                    }
                    intermediate_state.sync_uncertain = false;
                    self.storage.apply_common_difference_slice(
                        &new_messages,
                        &other_updates,
                        &auxiliary_peers,
                        &intermediate_state,
                    )?;
                    rec_pts = intermediate_state.pts;
                    rec_date = intermediate_state.date;
                    rec_qts = intermediate_state.qts;
                    final_state = intermediate_state;
                }
                CommonDifferenceResult::Difference {
                    new_messages,
                    other_updates,
                    auxiliary_peers,
                    mut state,
                } => {
                    if let Some(NormalizedUpdate::Unsupported {
                        constructor_id,
                        pts,
                        pts_count,
                        ..
                    }) = self.find_unsupported_state_affecting(&other_updates)
                    {
                        return Err(SyncError::UnsupportedStateAffectingUpdate {
                            constructor_id: *constructor_id,
                            pts: *pts,
                            pts_count: *pts_count,
                        });
                    }
                    state.sync_uncertain = false;
                    self.storage.apply_common_difference_slice(
                        &new_messages,
                        &other_updates,
                        &auxiliary_peers,
                        &state,
                    )?;
                    final_state = state;
                    break;
                }
                CommonDifferenceResult::TooLong { pts } => {
                    final_state.pts = pts;
                    break;
                }
            }
        }

        self.storage.record_sync_integrity_report(&SyncIntegrityReport {
            scope: "account_common".to_string(),
            peer_id: None,
            fully_lossless_contiguous_sync: false,
            current_history_repaired: true,
            new_messages_recovered: true,
            current_content_reconciled: true,
            historical_edits_complete: false,
            historical_deletions_complete: false,
            event_window_lost: true,
            channel_discovery_complete: true,
            gap_summary: Some(format!(
                "Common differenceTooLong (pts {overflow_pts} -> {recovery_pts}). Rescanned all archived common peers with server-side min_id; historical event window purged.",
            )),
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

        final_state.sync_uncertain = false;
        self.storage.upsert_account_sync_state(&final_state)?;
        Ok(final_state)
    }

    async fn recover_channel_difference_too_long(
        &self,
        channel_id: PeerId,
        dialog_pts: i32,
        top_message: Option<MessageId>,
        messages: Vec<MessageRecord>,
        auxiliary_peers: Vec<PeerRecord>,
        timeout: Option<i32>,
    ) -> SyncResult<PeerSyncState> {
        if !messages.is_empty() {
            self.storage
                .ingest_history_page(channel_id, &messages, &auxiliary_peers, None)?;
        }

        let local_max = self.storage.get_peer_max_message_id(channel_id)?;
        if let Some(top) = top_message
            && local_max.map(|m| m < top).unwrap_or(true)
        {
            let mut offset_id = None;
            loop {
                let page = self
                    .adapter
                    .get_history_page_filtered(channel_id, local_max, None, offset_id, 100)
                    .await?;

                if page.messages.is_empty() {
                    break;
                }

                let Some(batch_min_id) = page.messages.iter().map(|m| m.key.message_id).min()
                else {
                    break;
                };

                self.storage.ingest_history_page(
                    channel_id,
                    &page.messages,
                    &page.auxiliary_peers,
                    None,
                )?;

                if local_max.map(|m| batch_min_id <= m).unwrap_or(false)
                    || page.messages.len() < 100
                {
                    break;
                }
                offset_id = Some(batch_min_id);
            }
        }

        let mut final_pts = dialog_pts;
        let mut final_timeout = timeout;
        loop {
            let rec_res = self
                .adapter
                .get_channel_difference(channel_id, final_pts, 100)
                .await?;

            match rec_res {
                ChannelDifferenceResult::Empty {
                    pts, timeout: to, ..
                } => {
                    final_pts = pts;
                    final_timeout = to;
                    break;
                }
                ChannelDifferenceResult::Difference {
                    final_state,
                    pts,
                    timeout: to,
                    new_messages,
                    other_updates,
                    auxiliary_peers,
                } => {
                    if let Some(NormalizedUpdate::Unsupported {
                        constructor_id,
                        pts,
                        pts_count,
                        ..
                    }) = self.find_unsupported_state_affecting(&other_updates)
                    {
                        return Err(SyncError::UnsupportedStateAffectingUpdate {
                            constructor_id: *constructor_id,
                            pts: *pts,
                            pts_count: *pts_count,
                        });
                    }
                    self.storage.apply_channel_difference_slice(
                        channel_id,
                        &new_messages,
                        &other_updates,
                        &auxiliary_peers,
                        pts,
                        to,
                    )?;
                    final_pts = pts;
                    final_timeout = to;
                    if final_state {
                        break;
                    }
                }
                ChannelDifferenceResult::TooLong { dialog_pts: dp, .. } => {
                    final_pts = dp;
                    break;
                }
            }
        }

        let now = now_unix_secs();
        self.storage.record_sync_integrity_report(&SyncIntegrityReport {
            scope: "channel".to_string(),
            peer_id: Some(channel_id),
            fully_lossless_contiguous_sync: false,
            current_history_repaired: true,
            new_messages_recovered: true,
            current_content_reconciled: true,
            historical_edits_complete: false,
            historical_deletions_complete: false,
            event_window_lost: true,
            channel_discovery_complete: true,
            gap_summary: Some(format!(
                "ChannelDifferenceTooLong on channel {}: reset pts to {dialog_pts}, reconciled to final pts {final_pts}.",
                channel_id.raw()
            )),
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

        let state = PeerSyncState {
            peer_id: channel_id,
            pts: Some(final_pts),
            min_message_id: None,
            max_message_id: None,
            has_gap: false,
            sync_uncertain: false,
            poll_timeout_secs: final_timeout,
            last_synced_at: now,
        };
        self.storage
            .complete_channel_sync_and_queue(channel_id, final_pts, final_timeout, now)?;

        Ok(state)
    }

    fn find_unsupported_state_affecting<'a>(
        &self,
        updates: &'a [NormalizedUpdate],
    ) -> Option<&'a NormalizedUpdate> {
        updates.iter().find(|u| {
            matches!(
                u,
                NormalizedUpdate::Unsupported {
                    affects_sync_state: true,
                    ..
                }
            )
        })
    }

    fn tally_updates(&self, updates: &[NormalizedUpdate], summary: &mut CommonSyncSummary) {
        for u in updates {
            match u {
                NormalizedUpdate::EditedMessage { .. } => summary.edits_applied += 1,
                NormalizedUpdate::CommonDeletedMessages { message_ids, .. }
                | NormalizedUpdate::ChannelDeletedMessages { message_ids, .. } => {
                    summary.deletes_applied += message_ids.len();
                }
                NormalizedUpdate::ChannelCatchupRequired { .. } => {
                    summary.channel_catchups_queued += 1;
                }
                _ => {}
            }
        }
    }
}
