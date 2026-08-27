use std::collections::HashMap;

use rusqlite::{Row, params};
use vendetta_core::now_unix_secs;
use vendetta_model::{
    FilterDecision, FilterReason, MediaDownloadStatus, MediaFilterPolicy, MediaKind,
    MediaQueueStats, MediaRecord, MediaRole, MediaStats, MediaVerificationStatus, MessageId,
    MessageMediaJoin, PeerId,
};

use crate::{db::ArchiveDb, error::StorageResult};

fn map_media_row(row: &Row) -> rusqlite::Result<MediaRecord> {
    let kind_str: String = row.get("kind")?;
    let status_str: String = row.get("download_status")?;
    let ver_str: String = row.get("verification_status")?;
    let dec_str: Option<String> = row.get("filter_decision")?;
    let reason_str: Option<String> = row.get("filter_reason")?;

    Ok(MediaRecord {
        media_id: row.get("media_id")?,
        kind: kind_str.parse().unwrap_or(MediaKind::Other),
        mime_type: row.get("mime_type")?,
        size_bytes: row.get("size_bytes")?,
        file_name: row.get("file_name")?,
        size_type: row.get("size_type")?,
        width: row.get("width")?,
        height: row.get("height")?,
        dc_id: row.get("dc_id")?,
        source_location_tl: row.get("source_location_tl")?,
        file_reference: row.get("file_reference")?,
        local_rel_path: row.get("local_rel_path")?,
        sha256: row.get("sha256")?,
        download_status: status_str.parse().unwrap_or(MediaDownloadStatus::Pending),
        downloaded_bytes: row.get("downloaded_bytes")?,
        chunk_size: row.get("chunk_size")?,
        retry_count: row.get("retry_count")?,
        max_retries: row.get("max_retries")?,
        next_retry_at: row.get("next_retry_at")?,
        claimed_at: row.get("claimed_at")?,
        worker_id: row.get("worker_id")?,
        last_error: row.get("last_error")?,
        filter_decision: dec_str.as_deref().and_then(|s| s.parse().ok()),
        filter_reason: reason_str.as_deref().and_then(|s| s.parse().ok()),
        policy_version: row.get("policy_version")?,
        verification_status: ver_str
            .parse()
            .unwrap_or(MediaVerificationStatus::Unverified),
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

impl ArchiveDb {
    pub fn insert_or_update_media(&self, record: &MediaRecord) -> StorageResult<()> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            conn.execute(
                "INSERT INTO media_objects (
                    media_id, kind, mime_type, size_bytes, file_name, size_type,
                    width, height, dc_id, source_location_tl, file_reference,
                    local_rel_path, sha256, download_status, downloaded_bytes,
                    chunk_size, retry_count, max_retries, next_retry_at,
                    claimed_at, worker_id, last_error, filter_decision, filter_reason,
                    policy_version, verification_status, created_at, updated_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, ?8, ?9, ?10, ?11,
                    ?12, ?13, ?14, ?15,
                    ?16, ?17, ?18, ?19,
                    ?20, ?21, ?22, ?23, ?24,
                    ?25, ?26, ?27, ?28
                ) ON CONFLICT(media_id) DO UPDATE SET
                    kind = excluded.kind,
                    mime_type = COALESCE(excluded.mime_type, media_objects.mime_type),
                    size_bytes = COALESCE(excluded.size_bytes, media_objects.size_bytes),
                    file_name = COALESCE(excluded.file_name, media_objects.file_name),
                    size_type = COALESCE(excluded.size_type, media_objects.size_type),
                    width = COALESCE(excluded.width, media_objects.width),
                    height = COALESCE(excluded.height, media_objects.height),
                    dc_id = CASE WHEN excluded.dc_id != 0 THEN excluded.dc_id ELSE media_objects.dc_id END,
                    source_location_tl = COALESCE(excluded.source_location_tl, media_objects.source_location_tl),
                    file_reference = COALESCE(excluded.file_reference, media_objects.file_reference),
                    download_status = CASE
                        WHEN media_objects.download_status IN ('completed', 'downloading') THEN media_objects.download_status
                        ELSE excluded.download_status
                    END,
                    filter_decision = CASE
                        WHEN media_objects.download_status = 'completed' THEN media_objects.filter_decision
                        ELSE excluded.filter_decision
                    END,
                    filter_reason = CASE
                        WHEN media_objects.download_status = 'completed' THEN media_objects.filter_reason
                        ELSE excluded.filter_reason
                    END,
                    policy_version = CASE
                        WHEN media_objects.download_status = 'completed' THEN media_objects.policy_version
                        ELSE excluded.policy_version
                    END,
                    verification_status = CASE
                        WHEN media_objects.download_status = 'completed' THEN media_objects.verification_status
                        ELSE excluded.verification_status
                    END,
                    local_rel_path = COALESCE(media_objects.local_rel_path, excluded.local_rel_path),
                    sha256 = COALESCE(media_objects.sha256, excluded.sha256),
                    updated_at = ?28;",
                params![
                    record.media_id,
                    record.kind.as_ref(),
                    record.mime_type,
                    record.size_bytes,
                    record.file_name,
                    record.size_type,
                    record.width,
                    record.height,
                    record.dc_id,
                    record.source_location_tl,
                    record.file_reference,
                    record.local_rel_path,
                    record.sha256,
                    record.download_status.as_ref(),
                    record.downloaded_bytes,
                    record.chunk_size,
                    record.retry_count,
                    record.max_retries,
                    record.next_retry_at,
                    record.claimed_at,
                    record.worker_id,
                    record.last_error,
                    record.filter_decision.as_ref().map(|d| d.as_ref()),
                    record.filter_reason.as_ref().map(|r| r.as_ref()),
                    record.policy_version,
                    record.verification_status.as_ref(),
                    if record.created_at == 0 { now } else { record.created_at },
                    now,
                ],
            )?;
            Ok(())
        })
    }

    pub fn link_message_media(&self, join: &MessageMediaJoin) -> StorageResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO message_media (peer_id, message_id, media_id, role, position)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(peer_id, message_id, media_id, role) DO UPDATE SET position = excluded.position;",
                params![
                    join.key.peer_id.raw(),
                    join.key.message_id.raw(),
                    join.media_id,
                    join.role.as_ref(),
                    join.position,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_media(&self, media_id: &str) -> StorageResult<Option<MediaRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT * FROM media_objects WHERE media_id = ?1")?;
            let mut rows = stmt.query(params![media_id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(map_media_row(row)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn get_media_by_hash(&self, sha256: &str) -> StorageResult<Option<MediaRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT * FROM media_objects WHERE sha256 = ?1 AND download_status = 'completed' LIMIT 1",
            )?;
            let mut rows = stmt.query(params![sha256])?;
            if let Some(row) = rows.next()? {
                Ok(Some(map_media_row(row)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn claim_next_pending_media(&self, worker_id: &str) -> StorageResult<Option<MediaRecord>> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            let tx = conn.unchecked_transaction()?;

            let mut stmt = tx.prepare(
                "SELECT media_id FROM media_objects
                 WHERE (download_status = 'pending')
                    OR (download_status = 'retry_wait' AND (next_retry_at IS NULL OR next_retry_at <= ?1))
                 ORDER BY
                    CASE WHEN download_status = 'pending' THEN 0 ELSE 1 END ASC,
                    created_at ASC
                 LIMIT 1",
            )?;

            let media_id: Option<String> = stmt.query_row(params![now], |row| row.get(0)).ok();
            drop(stmt);

            let Some(id) = media_id else {
                tx.commit()?;
                return Ok(None);
            };

            tx.execute(
                "UPDATE media_objects
                 SET download_status = 'downloading', claimed_at = ?1, worker_id = ?2, updated_at = ?1
                 WHERE media_id = ?3",
                params![now, worker_id, id],
            )?;

            let mut fetch_stmt = tx.prepare("SELECT * FROM media_objects WHERE media_id = ?1")?;
            let record = fetch_stmt.query_row(params![id], map_media_row)?;
            drop(fetch_stmt);

            tx.commit()?;
            Ok(Some(record))
        })
    }

    pub fn update_media_progress(
        &self,
        media_id: &str,
        downloaded_bytes: i64,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            conn.execute(
                "UPDATE media_objects
                 SET downloaded_bytes = ?1, updated_at = ?2
                 WHERE media_id = ?3",
                params![downloaded_bytes, now, media_id],
            )?;
            Ok(())
        })
    }

    pub fn update_media_completed(
        &self,
        media_id: &str,
        sha256: &str,
        local_rel_path: &str,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            conn.execute(
                "UPDATE media_objects
                 SET download_status = 'completed',
                     verification_status = 'verified',
                     sha256 = ?1,
                     local_rel_path = ?2,
                     claimed_at = NULL,
                     worker_id = NULL,
                     last_error = NULL,
                     updated_at = ?3
                 WHERE media_id = ?4",
                params![sha256, local_rel_path, now, media_id],
            )?;
            Ok(())
        })
    }

    pub fn update_media_retry_wait(
        &self,
        media_id: &str,
        next_retry_at: i64,
        error: &str,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            conn.execute(
                "UPDATE media_objects
                 SET download_status = 'retry_wait',
                     retry_count = retry_count + 1,
                     next_retry_at = ?1,
                     last_error = ?2,
                     claimed_at = NULL,
                     worker_id = NULL,
                     updated_at = ?3
                 WHERE media_id = ?4",
                params![next_retry_at, error, now, media_id],
            )?;
            Ok(())
        })
    }

    pub fn update_media_status(
        &self,
        media_id: &str,
        status: MediaDownloadStatus,
        error: Option<&str>,
        next_retry_at: Option<i64>,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            conn.execute(
                "UPDATE media_objects
                 SET download_status = ?1,
                     retry_count = CASE WHEN ?1 = 'retry_wait' THEN retry_count + 1 ELSE retry_count END,
                     last_error = ?2,
                     next_retry_at = ?3,
                     claimed_at = NULL,
                     worker_id = NULL,
                     updated_at = ?4
                 WHERE media_id = ?5",
                params![status.as_ref(), error, next_retry_at, now, media_id],
            )?;
            Ok(())
        })
    }

    pub fn update_media_file_reference(
        &self,
        media_id: &str,
        file_reference: &[u8],
        source_location_tl: Option<&[u8]>,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            conn.execute(
                "UPDATE media_objects
                 SET file_reference = ?1,
                     source_location_tl = COALESCE(?2, source_location_tl),
                     download_status = 'pending',
                     last_error = NULL,
                     claimed_at = NULL,
                     worker_id = NULL,
                     updated_at = ?3
                 WHERE media_id = ?4",
                params![file_reference, source_location_tl, now, media_id],
            )?;
            Ok(())
        })
    }

    pub fn update_media_file_reference_while_claimed(
        &self,
        media_id: &str,
        file_reference: &[u8],
        source_location_tl: Option<&[u8]>,
        worker_id: &str,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            conn.execute(
                "UPDATE media_objects
                 SET file_reference = ?1,
                     source_location_tl = COALESCE(?2, source_location_tl),
                     download_status = 'downloading',
                     last_error = NULL,
                     claimed_at = ?3,
                     worker_id = ?4,
                     updated_at = ?3
                 WHERE media_id = ?5",
                params![file_reference, source_location_tl, now, worker_id, media_id],
            )?;
            Ok(())
        })
    }

    pub fn update_media_dc(&self, media_id: &str, new_dc_id: i32) -> StorageResult<()> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            conn.execute(
                "UPDATE media_objects
                 SET dc_id = ?1,
                     download_status = 'pending',
                     claimed_at = NULL,
                     worker_id = NULL,
                     updated_at = ?2
                 WHERE media_id = ?3",
                params![new_dc_id, now, media_id],
            )?;
            Ok(())
        })
    }

    pub fn update_media_dc_while_claimed(
        &self,
        media_id: &str,
        new_dc_id: i32,
        worker_id: &str,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            conn.execute(
                "UPDATE media_objects
                 SET dc_id = ?1,
                     download_status = 'downloading',
                     last_error = NULL,
                     claimed_at = ?2,
                     worker_id = ?3,
                     updated_at = ?2
                 WHERE media_id = ?4",
                params![new_dc_id, now, worker_id, media_id],
            )?;
            Ok(())
        })
    }

    pub fn update_media_verification_status(
        &self,
        media_id: &str,
        verification_status: MediaVerificationStatus,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            conn.execute(
                "UPDATE media_objects
                 SET verification_status = ?1, updated_at = ?2
                 WHERE media_id = ?3",
                params![verification_status.as_ref(), now, media_id],
            )?;
            Ok(())
        })
    }

    pub fn list_media_by_status(
        &self,
        status: MediaDownloadStatus,
        limit: usize,
    ) -> StorageResult<Vec<MediaRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT * FROM media_objects WHERE download_status = ?1 ORDER BY created_at ASC LIMIT ?2",
            )?;
            stmt.query_map(params![status.as_ref(), limit as i64], map_media_row)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
    }

    pub fn get_referencing_messages_for_media(
        &self,
        media_id: &str,
    ) -> StorageResult<Vec<(PeerId, MessageId)>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT peer_id, message_id FROM message_media WHERE media_id = ?1")?;
            stmt.query_map(params![media_id], |row| {
                let pid: i64 = row.get(0)?;
                let mid: i64 = row.get(1)?;
                Ok((PeerId::new(pid), MessageId::new(mid)))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
        })
    }

    pub fn get_message_media_with_roles(
        &self,
        peer_id: PeerId,
        message_id: MessageId,
    ) -> StorageResult<Vec<(MediaRecord, MediaRole, usize)>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT m.media_id, m.kind, m.mime_type, m.size_bytes, m.file_name,
                        m.size_type, m.width, m.height, m.dc_id, m.source_location_tl,
                        m.file_reference, m.local_rel_path, m.sha256, m.download_status,
                        m.downloaded_bytes, m.chunk_size, m.retry_count, m.max_retries,
                        m.next_retry_at, m.claimed_at, m.worker_id, m.last_error,
                        m.filter_decision, m.filter_reason, m.policy_version,
                        m.verification_status, m.created_at, m.updated_at,
                        j.role, j.position
                 FROM media_objects m
                 JOIN message_media j ON m.media_id = j.media_id
                 WHERE j.peer_id = ?1 AND j.message_id = ?2
                 ORDER BY j.position ASC",
            )?;
            stmt.query_map(params![peer_id.raw(), message_id.raw()], |row| {
                let rec = map_media_row(row)?;
                let role_str: String = row.get(28)?;
                let role = role_str.parse().unwrap_or(MediaRole::Attachment);
                let position: i64 = row.get(29)?;
                Ok((rec, role, position as usize))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
        })
    }

    pub fn get_media_for_message(
        &self,
        peer_id: PeerId,
        message_id: MessageId,
    ) -> StorageResult<Vec<MediaRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT m.* FROM media_objects m
                 JOIN message_media j ON m.media_id = j.media_id
                 WHERE j.peer_id = ?1 AND j.message_id = ?2
                 ORDER BY j.position ASC",
            )?;
            stmt.query_map(params![peer_id.raw(), message_id.raw()], map_media_row)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
    }

    pub fn get_media_stats(&self) -> StorageResult<MediaStats> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT
                    COUNT(*),
                    COUNT(CASE WHEN download_status = 'pending' THEN 1 END),
                    COUNT(CASE WHEN download_status = 'resolving' THEN 1 END),
                    COUNT(CASE WHEN download_status = 'downloading' THEN 1 END),
                    COUNT(CASE WHEN download_status = 'paused' THEN 1 END),
                    COUNT(CASE WHEN download_status = 'retry_wait' THEN 1 END),
                    COUNT(CASE WHEN download_status = 'completed' THEN 1 END),
                    COUNT(CASE WHEN download_status = 'verification_failed' THEN 1 END),
                    COUNT(CASE WHEN download_status = 'needs_reauth' THEN 1 END),
                    COUNT(CASE WHEN download_status = 'needs_file_reference_refresh' THEN 1 END),
                    COUNT(CASE WHEN download_status = 'needs_dc_migration' THEN 1 END),
                    COUNT(CASE WHEN download_status = 'permanently_failed' THEN 1 END),
                    COUNT(CASE WHEN download_status = 'skipped' THEN 1 END),
                    COUNT(CASE WHEN download_status IN ('permanently_failed', 'verification_failed', 'needs_reauth') THEN 1 END),
                    COUNT(CASE WHEN verification_status = 'unverified' THEN 1 END),
                    COUNT(CASE WHEN verification_status = 'verified' THEN 1 END),
                    COUNT(CASE WHEN verification_status = 'corrupted_hash' THEN 1 END),
                    COUNT(CASE WHEN verification_status = 'corrupted_size' THEN 1 END),
                    COUNT(CASE WHEN verification_status = 'missing_file' THEN 1 END),
                    COUNT(CASE WHEN verification_status IN ('corrupted_size', 'corrupted_hash') THEN 1 END),
                    COALESCE(SUM(size_bytes), 0),
                    COALESCE(SUM(downloaded_bytes), 0)
                 FROM media_objects",
            )?;

            let stats = stmt.query_row([], |row| {
                Ok(MediaStats {
                    total_count: row.get(0)?,
                    pending_count: row.get(1)?,
                    resolving_count: row.get(2)?,
                    downloading_count: row.get(3)?,
                    paused_count: row.get(4)?,
                    retry_wait_count: row.get(5)?,
                    completed_count: row.get(6)?,
                    verification_failed_count: row.get(7)?,
                    needs_reauth_count: row.get(8)?,
                    needs_file_ref_refresh_count: row.get(9)?,
                    needs_dc_migration_count: row.get(10)?,
                    permanently_failed_count: row.get(11)?,
                    skipped_count: row.get(12)?,
                    failed_count: row.get(13)?,
                    unverified_count: row.get(14)?,
                    verified_count: row.get(15)?,
                    corrupted_hash_count: row.get(16)?,
                    corrupted_size_count: row.get(17)?,
                    missing_file_count: row.get(18)?,
                    corrupted_count: row.get(19)?,
                    total_size_bytes: row.get(20)?,
                    downloaded_size_bytes: row.get(21)?,
                })
            })?;

            Ok(stats)
        })
    }

    pub fn get_queue_stats(&self) -> StorageResult<MediaQueueStats> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT
                    COUNT(*),
                    COALESCE(SUM(CASE WHEN size_bytes IS NOT NULL AND size_bytes > 0 THEN size_bytes ELSE 0 END), 0),
                    COUNT(CASE WHEN size_bytes IS NULL OR size_bytes <= 0 THEN 1 END)
                 FROM media_objects
                 WHERE download_status IN ('pending', 'retry_wait')",
            )?;

            let stats = stmt.query_row([], |row| {
                let eligible_count: i64 = row.get(0)?;
                let expected_bytes: i64 = row.get(1)?;
                let unknown_sizes_count: i64 = row.get(2)?;

                Ok(MediaQueueStats {
                    eligible_count: eligible_count as usize,
                    expected_bytes: expected_bytes.max(0) as u64,
                    all_sizes_known: unknown_sizes_count == 0,
                })
            })?;

            Ok(stats)
        })
    }

    pub fn requeue_missing_media_for_recovery(&self, media_id: &str) -> StorageResult<()> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            conn.execute(
                "UPDATE media_objects
                 SET download_status = 'pending',
                     verification_status = 'missing_file',
                     downloaded_bytes = 0,
                     claimed_at = NULL,
                     worker_id = NULL,
                     updated_at = ?1
                 WHERE media_id = ?2",
                params![now, media_id],
            )?;
            Ok(())
        })
    }

    pub fn get_skipped_media(&self) -> StorageResult<Vec<MediaRecord>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT * FROM media_objects WHERE download_status = 'skipped'")?;
            stmt.query_map([], map_media_row)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
    }

    pub fn update_media_filter_status(
        &self,
        media_id: &str,
        status: MediaDownloadStatus,
        decision: FilterDecision,
        reason: Option<FilterReason>,
        policy_version: i32,
    ) -> StorageResult<()> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            conn.execute(
                "UPDATE media_objects
                 SET download_status = ?1,
                     filter_decision = ?2,
                     filter_reason = ?3,
                     policy_version = ?4,
                     updated_at = ?5
                 WHERE media_id = ?6",
                params![
                    status.as_ref(),
                    decision.as_ref(),
                    reason.as_ref().map(|r| r.as_ref()),
                    policy_version,
                    now,
                    media_id
                ],
            )?;
            Ok(())
        })
    }

    pub fn requeue_skipped_media(&self, policy: &MediaFilterPolicy) -> StorageResult<usize> {
        self.with_conn(|conn| {
            let now = now_unix_secs();
            let rows = conn.execute(
                "UPDATE media_objects
                 SET download_status = 'pending', filter_decision = 'allow', filter_reason = NULL,
                     policy_version = ?1, updated_at = ?2
                 WHERE download_status = 'skipped'",
                params![policy.policy_version, now],
            )?;
            Ok(rows)
        })
    }

    pub fn get_peer_media_kind_counts(
        &self,
        peer_id: PeerId,
    ) -> StorageResult<HashMap<String, usize>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT m.kind, COUNT(DISTINCT m.media_id)
                 FROM message_media mm
                 JOIN media_objects m ON mm.media_id = m.media_id
                 WHERE mm.peer_id = ?1
                 GROUP BY m.kind",
            )?;
            stmt.query_map(params![peer_id.raw()], |row| {
                let kind: String = row.get(0)?;
                let count: i64 = row.get(1)?;
                Ok((kind, count as usize))
            })?
            .collect::<Result<HashMap<_, _>, _>>()
            .map_err(Into::into)
        })
    }
}
