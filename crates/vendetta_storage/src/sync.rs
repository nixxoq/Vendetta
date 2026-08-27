use rusqlite::params;
use vendetta_model::{
    AccountSyncState, ChannelQueueItem, ChannelQueueStatus, CommonDeletionTombstone, MessageId,
    PeerId, PeerSyncState, SyncBaseline, SyncBaselineStatus, SyncIntegrityReport, SyncStateRecord,
    UnsupportedEventRecord,
};

use crate::{db::ArchiveDb, error::StorageResult};

fn map_channel_queue_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChannelQueueItem> {
    let status_str: String = row.get(3)?;
    let status = status_str.parse().unwrap_or(ChannelQueueStatus::Pending);
    Ok(ChannelQueueItem {
        peer_id: PeerId::new(row.get(0)?),
        discovered_pts: row.get(1)?,
        current_pts: row.get(2)?,
        status,
        attempts: row.get(4)?,
        poll_timeout: row.get(5)?,
        last_error: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn build_in_clause(peers: Option<&[PeerId]>) -> (Option<String>, Vec<i64>) {
    match peers {
        Some(p) if !p.is_empty() => {
            let placeholders = p.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let ids = p.iter().map(|x| x.raw()).collect();
            (Some(placeholders), ids)
        }
        _ => (None, Vec::new()),
    }
}

impl ArchiveDb {
    pub fn upsert_sync_state(&self, state: &SyncStateRecord) -> StorageResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sync_state (
                    peer_id, pts, qts, date, seq, min_message_id, max_message_id, last_synced_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(peer_id) DO UPDATE SET
                    pts = excluded.pts,
                    qts = excluded.qts,
                    date = excluded.date,
                    seq = excluded.seq,
                    min_message_id = COALESCE(excluded.min_message_id, sync_state.min_message_id),
                    max_message_id = COALESCE(excluded.max_message_id, sync_state.max_message_id),
                    last_synced_at = excluded.last_synced_at;",
                params![
                    state.peer_id.raw(),
                    state.pts,
                    state.qts,
                    state.date,
                    state.seq,
                    state.min_message_id,
                    state.max_message_id,
                    state.last_synced_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_sync_state(&self, peer_id: PeerId) -> StorageResult<Option<SyncStateRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT peer_id, pts, qts, date, seq, min_message_id, max_message_id, last_synced_at
                 FROM sync_state WHERE peer_id = ?1",
            )?;
            let mut rows = stmt.query(params![peer_id.raw()])?;

            if let Some(row) = rows.next()? {
                Ok(Some(SyncStateRecord {
                    peer_id: PeerId::new(row.get(0)?),
                    pts: row.get(1)?,
                    qts: row.get(2)?,
                    date: row.get(3)?,
                    seq: row.get(4)?,
                    min_message_id: row.get(5)?,
                    max_message_id: row.get(6)?,
                    last_synced_at: row.get(7)?,
                }))
            } else {
                Ok(None)
            }
        })
    }

    pub fn upsert_account_sync_state(&self, state: &AccountSyncState) -> StorageResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO account_sync_state (
                    account_id, pts, qts, date, seq, sync_uncertain, last_synced_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(account_id) DO UPDATE SET
                    pts = excluded.pts,
                    qts = excluded.qts,
                    date = excluded.date,
                    seq = excluded.seq,
                    sync_uncertain = excluded.sync_uncertain,
                    last_synced_at = excluded.last_synced_at;",
                params![
                    state.account_id,
                    state.pts,
                    state.qts,
                    state.date,
                    state.seq,
                    if state.sync_uncertain { 1 } else { 0 },
                    state.last_synced_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_account_sync_state(
        &self,
        account_id: &str,
    ) -> StorageResult<Option<AccountSyncState>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT account_id, pts, qts, date, seq, sync_uncertain, last_synced_at
                 FROM account_sync_state WHERE account_id = ?1",
            )?;
            let mut rows = stmt.query(params![account_id])?;

            if let Some(row) = rows.next()? {
                let uncertain_num: i32 = row.get(5)?;
                Ok(Some(AccountSyncState {
                    account_id: row.get(0)?,
                    pts: row.get(1)?,
                    qts: row.get(2)?,
                    date: row.get(3)?,
                    seq: row.get(4)?,
                    sync_uncertain: uncertain_num != 0,
                    last_synced_at: row.get(6)?,
                }))
            } else {
                Ok(None)
            }
        })
    }

    pub fn upsert_peer_sync_state(&self, state: &PeerSyncState) -> StorageResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sync_state (
                    peer_id, pts, qts, date, seq, min_message_id, max_message_id, poll_timeout_secs, sync_uncertain, last_synced_at
                 ) VALUES (?1, ?2, NULL, NULL, NULL, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(peer_id) DO UPDATE SET
                    pts = excluded.pts,
                    min_message_id = COALESCE(excluded.min_message_id, sync_state.min_message_id),
                    max_message_id = COALESCE(excluded.max_message_id, sync_state.max_message_id),
                    poll_timeout_secs = excluded.poll_timeout_secs,
                    sync_uncertain = excluded.sync_uncertain,
                    last_synced_at = excluded.last_synced_at;",
                params![
                    state.peer_id.raw(),
                    state.pts,
                    state.min_message_id.map(|m| m.raw()),
                    state.max_message_id.map(|m| m.raw()),
                    state.poll_timeout_secs,
                    if state.sync_uncertain { 1 } else { 0 },
                    state.last_synced_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_peer_sync_state(&self, peer_id: PeerId) -> StorageResult<Option<PeerSyncState>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT peer_id, pts, min_message_id, max_message_id, poll_timeout_secs, sync_uncertain, last_synced_at
                 FROM sync_state WHERE peer_id = ?1",
            )?;
            let mut rows = stmt.query(params![peer_id.raw()])?;

            if let Some(row) = rows.next()? {
                let min_id: Option<i64> = row.get(2)?;
                let max_id: Option<i64> = row.get(3)?;
                let poll_timeout: Option<i32> = row.get(4)?;
                let uncertain: i32 = row.get(5).unwrap_or(0);
                Ok(Some(PeerSyncState {
                    peer_id: PeerId::new(row.get(0)?),
                    pts: row.get(1)?,
                    min_message_id: min_id.map(MessageId::new),
                    max_message_id: max_id.map(MessageId::new),
                    has_gap: false,
                    sync_uncertain: uncertain != 0,
                    poll_timeout_secs: poll_timeout,
                    last_synced_at: row.get(6)?,
                }))
            } else {
                Ok(None)
            }
        })
    }

    pub fn record_sync_baseline(&self, baseline: &SyncBaseline) -> StorageResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sync_baseline (
                    baseline_id, common_pts, common_qts, common_date, common_seq, status, captured_at, completed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(baseline_id) DO UPDATE SET
                    common_pts = excluded.common_pts,
                    common_qts = excluded.common_qts,
                    common_date = excluded.common_date,
                    common_seq = excluded.common_seq,
                    status = excluded.status,
                    completed_at = excluded.completed_at;",
                params![
                    baseline.baseline_id,
                    baseline.common_pts,
                    baseline.common_qts,
                    baseline.common_date,
                    baseline.common_seq,
                    baseline.status.as_ref(),
                    baseline.captured_at,
                    baseline.completed_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_sync_baseline(&self, baseline_id: &str) -> StorageResult<Option<SyncBaseline>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT baseline_id, common_pts, common_qts, common_date, common_seq, status, captured_at, completed_at
                 FROM sync_baseline WHERE baseline_id = ?1",
            )?;
            let mut rows = stmt.query(params![baseline_id])?;

            if let Some(row) = rows.next()? {
                let status_str: String = row.get(5)?;
                let status = status_str
                    .parse()
                    .unwrap_or(SyncBaselineStatus::InProgress);
                Ok(Some(SyncBaseline {
                    baseline_id: row.get(0)?,
                    common_pts: row.get(1)?,
                    common_qts: row.get(2)?,
                    common_date: row.get(3)?,
                    common_seq: row.get(4)?,
                    status,
                    captured_at: row.get(6)?,
                    completed_at: row.get(7)?,
                }))
            } else {
                Ok(None)
            }
        })
    }

    pub fn complete_sync_baseline(
        &self,
        baseline_id: &str,
        completed_at: i64,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE sync_baseline SET status = 'completed', completed_at = ?2 WHERE baseline_id = ?1",
                params![baseline_id, completed_at],
            )?;
            Ok(())
        })
    }

    pub fn reset_stale_in_progress_channels(&self) -> StorageResult<usize> {
        self.with_conn(|conn| {
            let affected = conn.execute(
                "UPDATE channel_sync_queue SET status = 'pending' WHERE status = 'in_progress'",
                [],
            )?;
            Ok(affected)
        })
    }

    pub fn enqueue_channel(&self, item: &ChannelQueueItem) -> StorageResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO channel_sync_queue (
                    peer_id, discovered_pts, current_pts, status, attempts, poll_timeout, last_error, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(peer_id) DO UPDATE SET
                    discovered_pts = MAX(channel_sync_queue.discovered_pts, excluded.discovered_pts),
                    current_pts = COALESCE(excluded.current_pts, channel_sync_queue.current_pts),
                    status = excluded.status,
                    poll_timeout = excluded.poll_timeout,
                    last_error = excluded.last_error,
                    updated_at = excluded.updated_at;",
                params![
                    item.peer_id.raw(),
                    item.discovered_pts,
                    item.current_pts,
                    item.status.as_ref(),
                    item.attempts,
                    item.poll_timeout,
                    item.last_error,
                    item.updated_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn pop_next_pending_channel(&self) -> StorageResult<Option<ChannelQueueItem>> {
        self.pop_next_pending_channel_filtered(None)
    }

    pub fn pop_next_pending_channel_filtered(
        &self,
        allowed_peers: Option<&[PeerId]>,
    ) -> StorageResult<Option<ChannelQueueItem>> {
        self.with_conn(|conn| {
            let (clause, ids) = build_in_clause(allowed_peers);
            let (query, params_slice): (String, Vec<&dyn rusqlite::ToSql>) = if let Some(placeholders) = clause {
                (
                    format!(
                        "SELECT peer_id, discovered_pts, current_pts, status, attempts, poll_timeout, last_error, updated_at
                         FROM channel_sync_queue
                         WHERE status = 'pending' AND peer_id IN ({placeholders})
                         ORDER BY updated_at ASC LIMIT 1"
                    ),
                    ids.iter().map(|v| v as &dyn rusqlite::ToSql).collect(),
                )
            } else {
                (
                    "SELECT peer_id, discovered_pts, current_pts, status, attempts, poll_timeout, last_error, updated_at
                     FROM channel_sync_queue
                     WHERE status = 'pending'
                     ORDER BY updated_at ASC LIMIT 1".to_string(),
                    Vec::new(),
                )
            };

            let mut stmt = conn.prepare(&query)?;
            let mut rows = stmt.query(params_slice.as_slice())?;

            if let Some(row) = rows.next()? {
                let peer_id = PeerId::new(row.get(0)?);
                conn.execute(
                    "UPDATE channel_sync_queue SET status = 'in_progress', attempts = attempts + 1 WHERE peer_id = ?1",
                    params![peer_id.raw()],
                )?;
                Ok(Some(ChannelQueueItem {
                    peer_id,
                    discovered_pts: row.get(1)?,
                    current_pts: row.get(2)?,
                    status: ChannelQueueStatus::InProgress,
                    attempts: row.get::<_, i32>(4)? + 1,
                    poll_timeout: row.get(5)?,
                    last_error: row.get(6)?,
                    updated_at: row.get(7)?,
                }))
            } else {
                Ok(None)
            }
        })
    }

    pub fn mark_channel_queue_completed(
        &self,
        peer_id: PeerId,
        final_pts: i32,
        poll_timeout: Option<i32>,
        now: i64,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE channel_sync_queue SET
                    status = 'completed',
                    current_pts = ?2,
                    poll_timeout = ?3,
                    last_error = NULL,
                    updated_at = ?4
                 WHERE peer_id = ?1",
                params![peer_id.raw(), final_pts, poll_timeout, now],
            )?;
            Ok(())
        })
    }

    pub fn mark_channel_queue_failed(
        &self,
        peer_id: PeerId,
        error_msg: &str,
        now: i64,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE channel_sync_queue SET
                    status = 'failed',
                    last_error = ?2,
                    updated_at = ?3
                 WHERE peer_id = ?1",
                params![peer_id.raw(), error_msg, now],
            )?;
            Ok(())
        })
    }

    pub fn list_pending_channels(&self) -> StorageResult<Vec<ChannelQueueItem>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT peer_id, discovered_pts, current_pts, status, attempts, poll_timeout, last_error, updated_at
                 FROM channel_sync_queue WHERE status = 'pending' ORDER BY updated_at ASC",
            )?;
            stmt.query_map([], map_channel_queue_row)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
    }

    pub fn count_blocked_or_failed_channels(&self) -> StorageResult<usize> {
        self.count_blocked_or_failed_channels_filtered(None)
    }

    pub fn count_blocked_or_failed_channels_filtered(
        &self,
        allowed_peers: Option<&[PeerId]>,
    ) -> StorageResult<usize> {
        self.with_conn(|conn| {
            let (clause, ids) = build_in_clause(allowed_peers);
            let (query, params_slice): (String, Vec<&dyn rusqlite::ToSql>) = if let Some(placeholders) = clause {
                (
                    format!(
                        "SELECT COUNT(*) FROM channel_sync_queue WHERE status IN ('blocked', 'failed') AND peer_id IN ({placeholders})"
                    ),
                    ids.iter().map(|v| v as &dyn rusqlite::ToSql).collect(),
                )
            } else {
                (
                    "SELECT COUNT(*) FROM channel_sync_queue WHERE status IN ('blocked', 'failed')".to_string(),
                    Vec::new(),
                )
            };

            let mut stmt = conn.prepare(&query)?;
            let count: i64 = stmt.query_row(params_slice.as_slice(), |row| row.get(0))?;
            Ok(count as usize)
        })
    }

    pub fn list_blocked_channels(&self) -> StorageResult<Vec<ChannelQueueItem>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT peer_id, discovered_pts, current_pts, status, attempts, poll_timeout, last_error, updated_at
                 FROM channel_sync_queue WHERE status = 'blocked' ORDER BY updated_at ASC",
            )?;
            stmt.query_map([], |row| {
                Ok(ChannelQueueItem {
                    peer_id: PeerId::new(row.get(0)?),
                    discovered_pts: row.get(1)?,
                    current_pts: row.get(2)?,
                    status: ChannelQueueStatus::Blocked,
                    attempts: row.get(4)?,
                    poll_timeout: row.get(5)?,
                    last_error: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn persist_account_unsupported_event_and_mark_uncertain(
        &self,
        constructor_id: u32,
        pts: Option<i32>,
        pts_count: Option<i32>,
        qts: Option<i32>,
        qts_count: Option<i32>,
        diagnostic_info: Option<&str>,
        raw_tl: &[u8],
        safe_state: &AccountSyncState,
        now: i64,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO unsupported_events (
                    peer_id, constructor_id, pts, pts_count, qts, qts_count, affects_sync_state, diagnostic_info, raw_tl, created_at
                 ) VALUES (NULL, ?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?8)",
                params![
                    constructor_id as i64,
                    pts,
                    pts_count,
                    qts,
                    qts_count,
                    diagnostic_info,
                    raw_tl,
                    now,
                ],
            )?;

            tx.execute(
                "INSERT INTO account_sync_state (
                    account_id, pts, qts, date, seq, sync_uncertain, last_synced_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
                 ON CONFLICT(account_id) DO UPDATE SET
                    pts = excluded.pts,
                    qts = excluded.qts,
                    date = excluded.date,
                    seq = excluded.seq,
                    sync_uncertain = 1,
                    last_synced_at = excluded.last_synced_at;",
                params![
                    safe_state.account_id,
                    safe_state.pts,
                    safe_state.qts,
                    safe_state.date,
                    safe_state.seq,
                    now,
                ],
            )?;

            tx.commit()?;
            Ok(())
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn persist_channel_unsupported_event_and_mark_uncertain(
        &self,
        channel_id: PeerId,
        constructor_id: u32,
        pts: Option<i32>,
        pts_count: Option<i32>,
        qts: Option<i32>,
        qts_count: Option<i32>,
        diagnostic_info: Option<&str>,
        raw_tl: &[u8],
        last_safe_pts: i32,
        poll_timeout: Option<i32>,
        now: i64,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO unsupported_events (
                    peer_id, constructor_id, pts, pts_count, qts, qts_count, affects_sync_state, diagnostic_info, raw_tl, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?9)",
                params![
                    channel_id.raw(),
                    constructor_id as i64,
                    pts,
                    pts_count,
                    qts,
                    qts_count,
                    diagnostic_info,
                    raw_tl,
                    now,
                ],
            )?;

            tx.execute(
                "INSERT INTO sync_state (
                    peer_id, pts, qts, date, seq, min_message_id, max_message_id, poll_timeout_secs, sync_uncertain, last_synced_at
                 ) VALUES (?1, ?2, NULL, NULL, NULL, NULL, NULL, ?3, 1, ?4)
                 ON CONFLICT(peer_id) DO UPDATE SET
                    pts = excluded.pts,
                    poll_timeout_secs = excluded.poll_timeout_secs,
                    sync_uncertain = 1,
                    last_synced_at = excluded.last_synced_at;",
                params![
                    channel_id.raw(),
                    last_safe_pts,
                    poll_timeout,
                    now,
                ],
            )?;

            tx.commit()?;
            Ok(())
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_unsupported_event(
        &self,
        peer_id: Option<PeerId>,
        constructor_id: u32,
        pts: Option<i32>,
        pts_count: Option<i32>,
        qts: Option<i32>,
        qts_count: Option<i32>,
        affects_sync_state: bool,
        diagnostic_info: Option<&str>,
        raw_tl: &[u8],
        now: i64,
    ) -> StorageResult<i64> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO unsupported_events (
                    peer_id, constructor_id, pts, pts_count, qts, qts_count, affects_sync_state, diagnostic_info, raw_tl, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    peer_id.map(|p| p.raw()),
                    constructor_id as i64,
                    pts,
                    pts_count,
                    qts,
                    qts_count,
                    if affects_sync_state { 1 } else { 0 },
                    diagnostic_info,
                    raw_tl,
                    now,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    pub fn list_unsupported_events(&self) -> StorageResult<Vec<UnsupportedEventRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT event_id, peer_id, constructor_id, pts, pts_count, qts, qts_count, affects_sync_state, diagnostic_info, raw_tl, created_at
                 FROM unsupported_events ORDER BY event_id ASC",
            )?;
            stmt.query_map([], |row| {
                let pid_raw: Option<i64> = row.get(1)?;
                let cid_raw: i64 = row.get(2)?;
                let affects: i64 = row.get(7)?;
                Ok(UnsupportedEventRecord {
                    event_id: row.get(0)?,
                    peer_id: pid_raw.map(PeerId::new),
                    constructor_id: cid_raw as u32,
                    pts: row.get(3)?,
                    pts_count: row.get(4)?,
                    qts: row.get(5)?,
                    qts_count: row.get(6)?,
                    affects_sync_state: affects != 0,
                    diagnostic_info: row.get(8)?,
                    raw_tl: row.get(9)?,
                    created_at: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
        })
    }

    pub fn complete_channel_sync_and_queue(
        &self,
        peer_id: PeerId,
        final_pts: i32,
        poll_timeout: Option<i32>,
        now: i64,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO sync_state (
                    peer_id, pts, qts, date, seq, min_message_id, max_message_id, sync_uncertain, poll_timeout_secs, last_synced_at
                 ) VALUES (?1, ?2, NULL, NULL, NULL, NULL, NULL, 0, ?3, ?4)
                 ON CONFLICT(peer_id) DO UPDATE SET
                    pts = excluded.pts,
                    sync_uncertain = 0,
                    poll_timeout_secs = excluded.poll_timeout_secs,
                    last_synced_at = excluded.last_synced_at;",
                params![peer_id.raw(), final_pts, poll_timeout, now],
            )?;
            tx.execute(
                "UPDATE channel_sync_queue
                 SET status = 'completed', current_pts = ?2, poll_timeout = ?3, updated_at = ?4
                 WHERE peer_id = ?1;",
                params![peer_id.raw(), final_pts, poll_timeout, now],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn record_common_deletion_tombstone(
        &self,
        tombstone: &CommonDeletionTombstone,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO common_deletion_tombstones (
                    message_id, pts, pts_count, observed_at
                 ) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(message_id) DO UPDATE SET
                    pts = coalesce(excluded.pts, common_deletion_tombstones.pts),
                    pts_count = coalesce(excluded.pts_count, common_deletion_tombstones.pts_count);",
                params![
                    tombstone.message_id.raw(),
                    tombstone.pts,
                    tombstone.pts_count,
                    tombstone.observed_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_common_deletion_tombstones(&self) -> StorageResult<Vec<CommonDeletionTombstone>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT message_id, pts, pts_count, observed_at FROM common_deletion_tombstones ORDER BY message_id ASC",
            )?;
            stmt.query_map([], |row| {
                Ok(CommonDeletionTombstone {
                    message_id: MessageId::new(row.get(0)?),
                    pts: row.get(1)?,
                    pts_count: row.get(2)?,
                    observed_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
        })
    }

    pub fn record_sync_integrity_report(&self, report: &SyncIntegrityReport) -> StorageResult<i64> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sync_integrity_reports (
                    scope, peer_id, fully_lossless_contiguous_sync, current_history_repaired,
                    new_messages_recovered, current_content_reconciled, historical_edits_complete,
                    historical_deletions_complete, event_window_lost, channel_discovery_complete,
                    gap_summary, created_at, provenance_version, deletion_reconciliation_performed,
                    deletion_reconciliation_complete, deletion_event_gap_count, deletion_tombstones_reconciled,
                    historical_message_reconciliation_performed, historical_message_reconciliation_complete,
                    historical_message_gap_count
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                params![
                    report.scope,
                    report.peer_id.map(|p| p.raw()),
                    if report.fully_lossless_contiguous_sync { 1 } else { 0 },
                    if report.current_history_repaired { 1 } else { 0 },
                    if report.new_messages_recovered { 1 } else { 0 },
                    if report.current_content_reconciled { 1 } else { 0 },
                    if report.historical_edits_complete { 1 } else { 0 },
                    if report.historical_deletions_complete { 1 } else { 0 },
                    if report.event_window_lost { 1 } else { 0 },
                    if report.channel_discovery_complete { 1 } else { 0 },
                    report.gap_summary,
                    report.created_at,
                    report.provenance_version,
                    if report.deletion_reconciliation_performed { 1 } else { 0 },
                    if report.deletion_reconciliation_complete { 1 } else { 0 },
                    report.deletion_event_gap_count,
                    report.deletion_tombstones_reconciled as i64,
                    if report.historical_message_reconciliation_performed { 1 } else { 0 },
                    if report.historical_message_reconciliation_complete { 1 } else { 0 },
                    report.historical_message_gap_count,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    pub fn get_latest_sync_integrity_report(
        &self,
        scope: &str,
    ) -> StorageResult<Option<SyncIntegrityReport>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT report_id, scope, peer_id, fully_lossless_contiguous_sync, current_history_repaired,
                        new_messages_recovered, current_content_reconciled, historical_edits_complete,
                        historical_deletions_complete, event_window_lost, channel_discovery_complete,
                        gap_summary, created_at,
                        COALESCE(provenance_version, 1),
                        COALESCE(deletion_reconciliation_performed, 0),
                        COALESCE(deletion_reconciliation_complete, 0),
                        COALESCE(deletion_event_gap_count, 0),
                        COALESCE(deletion_tombstones_reconciled, 0),
                        COALESCE(historical_message_reconciliation_performed, 0),
                        COALESCE(historical_message_reconciliation_complete, 0),
                        COALESCE(historical_message_gap_count, 0)
                 FROM sync_integrity_reports WHERE scope = ?1 ORDER BY report_id DESC LIMIT 1",
            )?;
            let mut rows = stmt.query(params![scope])?;

            if let Some(row) = rows.next()? {
                let peer_raw: Option<i64> = row.get(2)?;
                Ok(Some(SyncIntegrityReport {
                    scope: row.get(1)?,
                    peer_id: peer_raw.map(PeerId::new),
                    fully_lossless_contiguous_sync: row.get::<_, i32>(3)? != 0,
                    current_history_repaired: row.get::<_, i32>(4)? != 0,
                    new_messages_recovered: row.get::<_, i32>(5)? != 0,
                    current_content_reconciled: row.get::<_, i32>(6)? != 0,
                    historical_edits_complete: row.get::<_, i32>(7)? != 0,
                    historical_deletions_complete: row.get::<_, i32>(8)? != 0,
                    event_window_lost: row.get::<_, i32>(9)? != 0,
                    channel_discovery_complete: row.get::<_, i32>(10)? != 0,
                    gap_summary: row.get(11)?,
                    created_at: row.get(12)?,
                    provenance_version: row.get::<_, u32>(13)?,
                    deletion_reconciliation_performed: row.get::<_, i32>(14)? != 0,
                    deletion_reconciliation_complete: row.get::<_, i32>(15)? != 0,
                    deletion_event_gap_count: row.get::<_, u32>(16)?,
                    deletion_tombstones_reconciled: row.get::<_, i64>(17)? as usize,
                    historical_message_reconciliation_performed: row.get::<_, i32>(18)? != 0,
                    historical_message_reconciliation_complete: row.get::<_, i32>(19)? != 0,
                    historical_message_gap_count: row.get::<_, u32>(20)?,
                }))
            } else {
                Ok(None)
            }
        })
    }

    pub fn list_archived_common_peer_ids(&self) -> StorageResult<Vec<PeerId>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT peer_id FROM peers WHERE peer_type IN ('user', 'group') ORDER BY peer_id ASC",
            )?;
            stmt.query_map([], |row| Ok(PeerId::new(row.get(0)?)))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
    }

    pub fn list_archived_channel_peer_ids(&self) -> StorageResult<Vec<PeerId>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT peer_id FROM peers WHERE peer_type = 'channel' ORDER BY peer_id ASC",
            )?;
            stmt.query_map([], |row| Ok(PeerId::new(row.get(0)?)))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
    }
}
