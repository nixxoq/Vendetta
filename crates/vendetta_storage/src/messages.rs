use std::collections::BTreeSet;

use rusqlite::{Row, Transaction, params};
use vendetta_core::now_unix_secs;
use vendetta_model::{
    AccountSyncState, MessageId, MessageKey, MessageRecord, MessageRevisionRecord, MessageState,
    NormalizedUpdate, PeerId, PeerRecord, ReactionKey, SyncStateRecord, parse_reactions_json,
};

use crate::{db::ArchiveDb, error::StorageResult};

fn map_message_row(row: &Row<'_>) -> rusqlite::Result<MessageRecord> {
    let state_str: String = row.get(7)?;
    let state = state_str.parse().unwrap_or(MessageState::Active);

    Ok(MessageRecord {
        key: MessageKey::new(row.get::<_, i64>(0)?, row.get::<_, i64>(1)?),
        date: row.get(2)?,
        sender_id: row.get::<_, Option<i64>>(3)?.map(PeerId::new),
        text: row.get(4)?,
        entities_json: row.get(5)?,
        edit_date: row.get(6)?,
        state,
        reply_to_msg_id: row.get::<_, Option<i64>>(8)?.map(MessageId::new),
        reply_to_top_id: row.get::<_, Option<i64>>(9)?.map(MessageId::new),
        reply_to_peer_id: row.get::<_, Option<i64>>(10)?.map(PeerId::new),
        grouped_id: row.get(11)?,
        forward_json: row.get(12)?,
        reactions_json: row.get(13)?,
        views: row.get(14)?,
        forwards_count: row.get(15)?,
        raw_tl: row.get(16)?,
    })
}

fn upsert_peer_in_tx(tx: &Transaction<'_>, peer: &PeerRecord) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO peers (peer_id, peer_type, name, username, phone, raw_tl, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(peer_id) DO UPDATE SET
            name = coalesce(excluded.name, peers.name),
            username = coalesce(excluded.username, peers.username),
            phone = coalesce(excluded.phone, peers.phone),
            raw_tl = coalesce(excluded.raw_tl, peers.raw_tl),
            updated_at = excluded.updated_at;",
        params![
            peer.peer_id.raw(),
            peer.peer_type.as_ref(),
            peer.name,
            peer.username,
            peer.phone,
            peer.raw_tl,
            peer.updated_at,
        ],
    )?;
    Ok(())
}

impl ArchiveDb {
    pub fn insert_messages_batch(&self, messages: &[MessageRecord]) -> StorageResult<usize> {
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
            let mut count = 0;
            for msg in messages {
                Self::process_message_in_tx(&tx, msg)?;
                count += 1;
            }
            tx.commit()?;
            Ok(count)
        })
    }

    pub fn ingest_history_page(
        &self,
        _peer_id: PeerId,
        messages: &[MessageRecord],
        auxiliary_peers: &[PeerRecord],
        new_sync_state: Option<&SyncStateRecord>,
    ) -> StorageResult<usize> {
        self.with_conn(|conn| {
            let tx = conn.transaction()?;

            for peer in auxiliary_peers {
                upsert_peer_in_tx(&tx, peer)?;
            }

            let mut count = 0;
            for msg in messages {
                Self::process_message_in_tx(&tx, msg)?;
                count += 1;
            }

            if let Some(state) = new_sync_state {
                tx.execute(
                    "INSERT INTO sync_state (
                        peer_id, pts, qts, date, seq, min_message_id, max_message_id, last_synced_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(peer_id) DO UPDATE SET
                        pts = coalesce(excluded.pts, sync_state.pts),
                        qts = coalesce(excluded.qts, sync_state.qts),
                        date = coalesce(excluded.date, sync_state.date),
                        seq = coalesce(excluded.seq, sync_state.seq),
                        min_message_id = coalesce(excluded.min_message_id, sync_state.min_message_id),
                        max_message_id = coalesce(excluded.max_message_id, sync_state.max_message_id),
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
            }
            tx.commit()?;
            Ok(count)
        })
    }

    pub fn count_messages_by_peer(&self, peer_id: PeerId) -> StorageResult<usize> {
        self.with_conn(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM messages WHERE peer_id = ?1",
                params![peer_id.raw()],
                |row| row.get(0),
            )?;
            Ok(count as usize)
        })
    }

    pub fn insert_or_update_message(&self, message: &MessageRecord) -> StorageResult<()> {
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
            Self::process_message_in_tx(&tx, message)?;
            tx.commit()?;
            Ok(())
        })
    }

    fn is_message_content_modified(prev: &MessageRecord, curr: &MessageRecord) -> bool {
        prev.text != curr.text
            || prev.entities_json != curr.entities_json
            || (prev.edit_date != curr.edit_date && curr.edit_date.is_some())
            || prev.reply_to_msg_id != curr.reply_to_msg_id
            || prev.reply_to_top_id != curr.reply_to_top_id
            || prev.reply_to_peer_id != curr.reply_to_peer_id
            || prev.forward_json != curr.forward_json
            || prev.reactions_json != curr.reactions_json
            || (prev.raw_tl != curr.raw_tl && curr.raw_tl.is_some())
    }

    pub fn process_message_in_tx(tx: &Transaction<'_>, msg: &MessageRecord) -> StorageResult<()> {
        let existing = Self::get_message_in_tx(tx, msg.key)?;

        if let Some(prev) = existing {
            if Self::is_message_content_modified(&prev, msg) {
                let now = now_unix_secs();

                tx.execute(
                    "INSERT INTO message_revisions (peer_id, message_id, captured_at, edit_date, text, entities_json, raw_tl)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        prev.key.peer_id.raw(),
                        prev.key.message_id.raw(),
                        now,
                        prev.edit_date,
                        prev.text,
                        prev.entities_json,
                        prev.raw_tl,
                    ],
                )?;
            }

            tx.execute(
                "UPDATE messages SET
                    date = ?3,
                    sender_id = ?4,
                    text = ?5,
                    entities_json = ?6,
                    edit_date = ?7,
                    state = ?8,
                    reply_to_msg_id = ?9,
                    reply_to_top_id = ?10,
                    reply_to_peer_id = ?11,
                    grouped_id = ?12,
                    forward_json = ?13,
                    reactions_json = ?14,
                    views = ?15,
                    forwards_count = ?16,
                    raw_tl = coalesce(?17, raw_tl)
                 WHERE peer_id = ?1 AND message_id = ?2",
                params![
                    msg.key.peer_id.raw(),
                    msg.key.message_id.raw(),
                    msg.date,
                    msg.sender_id.map(|s| s.raw()),
                    msg.text,
                    msg.entities_json,
                    msg.edit_date,
                    msg.state.as_ref(),
                    msg.reply_to_msg_id.map(|m| m.raw()),
                    msg.reply_to_top_id.map(|m| m.raw()),
                    msg.reply_to_peer_id.map(|p| p.raw()),
                    msg.grouped_id,
                    msg.forward_json,
                    msg.reactions_json,
                    msg.views,
                    msg.forwards_count,
                    msg.raw_tl,
                ],
            )?;
        } else {
            tx.execute(
                "INSERT INTO messages (
                    peer_id, message_id, date, sender_id, text, entities_json,
                    edit_date, state, reply_to_msg_id, reply_to_top_id, reply_to_peer_id,
                    grouped_id, forward_json, reactions_json, views, forwards_count, raw_tl
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17
                 )",
                params![
                    msg.key.peer_id.raw(),
                    msg.key.message_id.raw(),
                    msg.date,
                    msg.sender_id.map(|s| s.raw()),
                    msg.text,
                    msg.entities_json,
                    msg.edit_date,
                    msg.state.as_ref(),
                    msg.reply_to_msg_id.map(|m| m.raw()),
                    msg.reply_to_top_id.map(|m| m.raw()),
                    msg.reply_to_peer_id.map(|p| p.raw()),
                    msg.grouped_id,
                    msg.forward_json,
                    msg.reactions_json,
                    msg.views,
                    msg.forwards_count,
                    msg.raw_tl,
                ],
            )?;
        }

        Ok(())
    }

    fn get_message_in_tx(
        tx: &Transaction<'_>,
        key: MessageKey,
    ) -> StorageResult<Option<MessageRecord>> {
        let mut stmt = tx.prepare(
            "SELECT peer_id, message_id, date, sender_id, text, entities_json,
                    edit_date, state, reply_to_msg_id, reply_to_top_id, reply_to_peer_id,
                    grouped_id, forward_json, reactions_json, views, forwards_count, raw_tl
             FROM messages WHERE peer_id = ?1 AND message_id = ?2",
        )?;
        let mut rows = stmt.query(params![key.peer_id.raw(), key.message_id.raw()])?;

        if let Some(row) = rows.next()? {
            Ok(Some(map_message_row(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn get_message(&self, key: MessageKey) -> StorageResult<Option<MessageRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT peer_id, message_id, date, sender_id, text, entities_json,
                        edit_date, state, reply_to_msg_id, reply_to_top_id, reply_to_peer_id,
                        grouped_id, forward_json, reactions_json, views, forwards_count, raw_tl
                 FROM messages WHERE peer_id = ?1 AND message_id = ?2",
            )?;
            let mut rows = stmt.query(params![key.peer_id.raw(), key.message_id.raw()])?;

            if let Some(row) = rows.next()? {
                Ok(Some(map_message_row(row)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn list_message_dates_by_peer(&self, peer_id: PeerId) -> StorageResult<Vec<i64>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT date FROM messages WHERE peer_id = ?1 ORDER BY message_id ASC")?;
            stmt.query_map(params![peer_id.raw()], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
    }

    pub fn list_messages_by_peer(
        &self,
        peer_id: PeerId,
        limit: usize,
        offset: usize,
    ) -> StorageResult<Vec<MessageRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT peer_id, message_id, date, sender_id, text, entities_json,
                        edit_date, state, reply_to_msg_id, reply_to_top_id, reply_to_peer_id,
                        grouped_id, forward_json, reactions_json, views, forwards_count, raw_tl
                 FROM messages WHERE peer_id = ?1 ORDER BY message_id ASC LIMIT ?2 OFFSET ?3",
            )?;
            stmt.query_map(
                params![peer_id.raw(), limit as i64, offset as i64],
                map_message_row,
            )?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
        })
    }

    pub fn get_last_message_date_by_peer(&self, peer_id: PeerId) -> StorageResult<Option<i64>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT date FROM messages WHERE peer_id = ?1 ORDER BY date DESC, message_id DESC LIMIT 1",
            )?;
            let mut rows = stmt.query(params![peer_id.raw()])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row.get(0)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn list_message_revisions(
        &self,
        key: MessageKey,
    ) -> StorageResult<Vec<MessageRevisionRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT revision_id, peer_id, message_id, captured_at, edit_date, text, entities_json, raw_tl
                 FROM message_revisions WHERE peer_id = ?1 AND message_id = ?2 ORDER BY revision_id ASC",
            )?;
            stmt.query_map(params![key.peer_id.raw(), key.message_id.raw()], |row| {
                Ok(MessageRevisionRecord {
                    revision_id: Some(row.get(0)?),
                    key: MessageKey::new(row.get::<_, i64>(1)?, row.get::<_, i64>(2)?),
                    captured_at: row.get(3)?,
                    edit_date: row.get(4)?,
                    text: row.get(5)?,
                    entities_json: row.get(6)?,
                    raw_tl: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
        })
    }

    pub fn mark_message_deleted(&self, key: MessageKey) -> StorageResult<bool> {
        self.with_conn(|conn| {
            let affected = conn.execute(
                "UPDATE messages SET state = 'deleted' WHERE peer_id = ?1 AND message_id = ?2",
                params![key.peer_id.raw(), key.message_id.raw()],
            )?;
            Ok(affected > 0)
        })
    }

    pub fn mark_message_deleted_in_tx(
        tx: &Transaction,
        peer_id: PeerId,
        message_id: MessageId,
    ) -> StorageResult<()> {
        if peer_id.raw() == 0 {
            return Ok(());
        }
        let affected = tx.execute(
            "UPDATE messages SET state = 'deleted' WHERE peer_id = ?1 AND message_id = ?2",
            params![peer_id.raw(), message_id.raw()],
        )?;
        if affected == 0 {
            let now = now_unix_secs();
            tx.execute(
                "INSERT INTO messages (
                    peer_id, message_id, date, sender_id, text, entities_json, edit_date, state,
                    reply_to_msg_id, reply_to_top_id, reply_to_peer_id, grouped_id, forward_json,
                    reactions_json, views, forwards_count, raw_tl
                 ) VALUES (?1, ?2, ?3, NULL, NULL, NULL, NULL, 'deleted', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)
                 ON CONFLICT(peer_id, message_id) DO UPDATE SET state = 'deleted';",
                params![peer_id.raw(), message_id.raw(), now],
            )?;
        }
        Ok(())
    }

    pub fn delete_common_messages_in_tx(
        tx: &Transaction,
        message_ids: &[MessageId],
        pts: Option<i32>,
        pts_count: i32,
        now: i64,
    ) -> StorageResult<usize> {
        let mut count = 0;
        for &mid in message_ids {
            let mut stmt = tx.prepare(
                "SELECT m.peer_id
                 FROM messages m
                 INNER JOIN peers p ON m.peer_id = p.peer_id
                 WHERE m.message_id = ?1
                   AND p.peer_type IN ('user', 'group')",
            )?;
            let found_peers: Vec<i64> = stmt
                .query_map(params![mid.raw()], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();

            if found_peers.len() == 1 {
                let pid = found_peers[0];
                tx.execute(
                    "UPDATE messages SET state = 'deleted' WHERE peer_id = ?1 AND message_id = ?2",
                    params![pid, mid.raw()],
                )?;
                count += 1;
            } else if found_peers.len() > 1 {
                tx.execute(
                    "INSERT INTO unsupported_events (
                        peer_id, constructor_id, pts, pts_count, qts, qts_count, affects_sync_state, diagnostic_info, raw_tl, created_at
                     ) VALUES (NULL, 0xde1e7e, ?1, ?2, NULL, NULL, 0, ?3, X'', ?4)",
                    params![
                        pts,
                        pts_count,
                        format!("Ambiguous common delete for message_id {}: matches multiple peers {:?}", mid.raw(), found_peers),
                        now,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO common_deletion_tombstones (
                        message_id, pts, pts_count, observed_at
                     ) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(message_id) DO UPDATE SET
                        pts = coalesce(excluded.pts, common_deletion_tombstones.pts),
                        pts_count = coalesce(excluded.pts_count, common_deletion_tombstones.pts_count);",
                    params![mid.raw(), pts, pts_count, now],
                )?;
            } else {
                tx.execute(
                    "INSERT INTO common_deletion_tombstones (
                        message_id, pts, pts_count, observed_at
                     ) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(message_id) DO UPDATE SET
                        pts = coalesce(excluded.pts, common_deletion_tombstones.pts),
                        pts_count = coalesce(excluded.pts_count, common_deletion_tombstones.pts_count);",
                    params![mid.raw(), pts, pts_count, now],
                )?;
            }
        }
        Ok(count)
    }

    pub fn delete_channel_messages_in_tx(
        tx: &Transaction,
        channel_id: PeerId,
        message_ids: &[MessageId],
        now: i64,
    ) -> StorageResult<usize> {
        let mut count = 0;
        for &mid in message_ids {
            let affected = tx.execute(
                "UPDATE messages SET state = 'deleted' WHERE peer_id = ?1 AND message_id = ?2",
                params![channel_id.raw(), mid.raw()],
            )?;
            if affected > 0 {
                count += affected;
            } else if channel_id.raw() != 0 {
                tx.execute(
                    "INSERT INTO messages (
                        peer_id, message_id, date, sender_id, text, entities_json, edit_date, state,
                        reply_to_msg_id, reply_to_top_id, reply_to_peer_id, grouped_id, forward_json,
                        reactions_json, views, forwards_count, raw_tl
                     ) VALUES (?1, ?2, ?3, NULL, NULL, NULL, NULL, 'deleted', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)
                     ON CONFLICT(peer_id, message_id) DO UPDATE SET state = 'deleted';",
                    params![channel_id.raw(), mid.raw(), now],
                )?;
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn apply_common_difference_slice(
        &self,
        new_messages: &[MessageRecord],
        other_updates: &[NormalizedUpdate],
        auxiliary_peers: &[PeerRecord],
        new_state: &AccountSyncState,
    ) -> StorageResult<usize> {
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
            let now = now_unix_secs();

            for peer in auxiliary_peers {
                upsert_peer_in_tx(&tx, peer)?;
            }

            let mut count = 0;
            for msg in new_messages {
                Self::process_message_in_tx(&tx, msg)?;
                count += 1;
            }

            for update in other_updates {
                match update {
                    NormalizedUpdate::NewMessage { message, .. }
                    | NormalizedUpdate::EditedMessage { message, .. } => {
                        Self::process_message_in_tx(&tx, message)?;
                        count += 1;
                    }
                    NormalizedUpdate::CommonDeletedMessages {
                        message_ids,
                        pts,
                        pts_count,
                    } => {
                        Self::delete_common_messages_in_tx(&tx, message_ids, *pts, *pts_count, now)?;
                    }
                    NormalizedUpdate::ChannelDeletedMessages {
                        channel_id,
                        message_ids,
                        ..
                    } => {
                        Self::delete_channel_messages_in_tx(&tx, *channel_id, message_ids, now)?;
                    }
                    NormalizedUpdate::PeerDiscovered { peer } => {
                        upsert_peer_in_tx(&tx, peer)?;
                    }
                    NormalizedUpdate::ChannelCatchupRequired { channel_id, pts } => {
                        tx.execute(
                            "INSERT INTO channel_sync_queue (
                                peer_id, discovered_pts, current_pts, status, attempts, poll_timeout, last_error, updated_at
                             ) VALUES (?1, ?2, NULL, 'pending', 0, NULL, NULL, ?3)
                             ON CONFLICT(peer_id) DO UPDATE SET
                                discovered_pts = MAX(channel_sync_queue.discovered_pts, excluded.discovered_pts),
                                status = CASE WHEN channel_sync_queue.status = 'completed' THEN 'pending' ELSE channel_sync_queue.status END,
                                updated_at = excluded.updated_at;",
                            params![channel_id.raw(), pts.unwrap_or(0), now],
                        )?;
                    }
                    NormalizedUpdate::ServiceAction {
                        peer_id,
                        message_id,
                        actor_id,
                        date,
                        action_text,
                        raw_tl,
                        ..
                    } => {
                        let service_msg = MessageRecord {
                            key: MessageKey::new(peer_id.raw(), message_id.raw()),
                            date: *date,
                            sender_id: *actor_id,
                            text: Some(action_text.clone()),
                            entities_json: None,
                            edit_date: None,
                            state: MessageState::Active,
                            reply_to_msg_id: None,
                            reply_to_top_id: None,
                            reply_to_peer_id: None,
                            grouped_id: None,
                            forward_json: None,
                            reactions_json: None,
                            views: None,
                            forwards_count: None,
                            raw_tl: Some(raw_tl.clone()),
                        };
                        Self::process_message_in_tx(&tx, &service_msg)?;
                        count += 1;
                    }
                    NormalizedUpdate::Unsupported {
                        constructor_id,
                        pts,
                        pts_count,
                        qts,
                        qts_count,
                        affects_sync_state,
                        diagnostic_info,
                        raw_tl,
                        ..
                    } => {
                        tx.execute(
                            "INSERT INTO unsupported_events (
                                peer_id, constructor_id, pts, pts_count, qts, qts_count, affects_sync_state, diagnostic_info, raw_tl, created_at
                             ) VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                            params![
                                *constructor_id as i64,
                                pts,
                                pts_count,
                                qts,
                                qts_count,
                                if *affects_sync_state { 1 } else { 0 },
                                diagnostic_info.as_deref(),
                                raw_tl,
                                now,
                            ],
                        )?;
                    }
                }
            }

            tx.execute(
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
                    new_state.account_id,
                    new_state.pts,
                    new_state.qts,
                    new_state.date,
                    new_state.seq,
                    if new_state.sync_uncertain { 1 } else { 0 },
                    new_state.last_synced_at,
                ],
            )?;

            tx.commit()?;
            Ok(count)
        })
    }

    pub fn apply_channel_difference_slice(
        &self,
        channel_id: PeerId,
        new_messages: &[MessageRecord],
        other_updates: &[NormalizedUpdate],
        auxiliary_peers: &[PeerRecord],
        new_pts: i32,
        _poll_timeout: Option<i32>,
    ) -> StorageResult<usize> {
        self.with_conn(|conn| {
            let tx = conn.transaction()?;
            let now = now_unix_secs();

            for peer in auxiliary_peers {
                upsert_peer_in_tx(&tx, peer)?;
            }

            let mut count = 0;
            for msg in new_messages {
                Self::process_message_in_tx(&tx, msg)?;
                count += 1;
            }

            for update in other_updates {
                match update {
                    NormalizedUpdate::NewMessage { message, .. }
                    | NormalizedUpdate::EditedMessage { message, .. } => {
                        Self::process_message_in_tx(&tx, message)?;
                        count += 1;
                    }
                    NormalizedUpdate::ChannelDeletedMessages {
                        channel_id: cid,
                        message_ids,
                        ..
                    } => {
                        Self::delete_channel_messages_in_tx(&tx, *cid, message_ids, now)?;
                    }
                    NormalizedUpdate::CommonDeletedMessages {
                        message_ids,
                        pts,
                        pts_count,
                    } => {
                        Self::delete_common_messages_in_tx(&tx, message_ids, *pts, *pts_count, now)?;
                    }
                    NormalizedUpdate::ServiceAction {
                        peer_id,
                        message_id,
                        actor_id,
                        date,
                        action_text,
                        raw_tl,
                        ..
                    } => {
                        let service_msg = MessageRecord {
                            key: MessageKey::new(peer_id.raw(), message_id.raw()),
                            date: *date,
                            sender_id: *actor_id,
                            text: Some(action_text.clone()),
                            entities_json: None,
                            edit_date: None,
                            state: MessageState::Active,
                            reply_to_msg_id: None,
                            reply_to_top_id: None,
                            reply_to_peer_id: None,
                            grouped_id: None,
                            forward_json: None,
                            reactions_json: None,
                            views: None,
                            forwards_count: None,
                            raw_tl: Some(raw_tl.clone()),
                        };
                        Self::process_message_in_tx(&tx, &service_msg)?;
                        count += 1;
                    }
                    NormalizedUpdate::Unsupported {
                        constructor_id,
                        pts,
                        pts_count,
                        qts,
                        qts_count,
                        affects_sync_state,
                        diagnostic_info,
                        raw_tl,
                        ..
                    } => {
                        tx.execute(
                            "INSERT INTO unsupported_events (
                                peer_id, constructor_id, pts, pts_count, qts, qts_count, affects_sync_state, diagnostic_info, raw_tl, created_at
                             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                            params![
                                channel_id.raw(),
                                *constructor_id as i64,
                                pts,
                                pts_count,
                                qts,
                                qts_count,
                                if *affects_sync_state { 1 } else { 0 },
                                diagnostic_info.as_deref(),
                                raw_tl,
                                now,
                            ],
                        )?;
                    }
                    _ => {}
                }
            }

            tx.execute(
                "INSERT INTO sync_state (
                    peer_id, pts, qts, date, seq, min_message_id, max_message_id, sync_uncertain, last_synced_at
                 ) VALUES (?1, ?2, NULL, NULL, NULL, NULL, NULL, 0, ?3)
                 ON CONFLICT(peer_id) DO UPDATE SET
                    pts = excluded.pts,
                    sync_uncertain = 0,
                    last_synced_at = excluded.last_synced_at;",
                params![channel_id.raw(), new_pts, now],
            )?;

            tx.commit()?;
            Ok(count)
        })
    }

    pub fn get_peer_max_message_id(&self, peer_id: PeerId) -> StorageResult<Option<MessageId>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT MAX(message_id) FROM messages WHERE peer_id = ?1;")?;
            let max_id: Option<i64> = stmt.query_row(params![peer_id.raw()], |row| row.get(0))?;
            Ok(max_id.map(MessageId::new))
        })
    }

    pub fn get_peer_min_message_id(&self, peer_id: PeerId) -> StorageResult<Option<MessageId>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT MIN(message_id) FROM messages WHERE peer_id = ?1;")?;
            let min_id: Option<i64> = stmt.query_row(params![peer_id.raw()], |row| row.get(0))?;
            Ok(min_id.map(MessageId::new))
        })
    }

    pub fn list_custom_emoji_reaction_document_ids(&self) -> StorageResult<Vec<i64>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT reactions_json FROM messages WHERE reactions_json IS NOT NULL AND reactions_json LIKE '%CustomEmoji%';"
            )?;
            let mut rows = stmt.query([])?;
            let mut ids = BTreeSet::new();

            while let Some(row) = rows.next()? {
                let json_str: Option<String> = row.get(0)?;
                if let Some(ref s) = json_str
                    && let Some(reactions_data) = parse_reactions_json(s)
                {
                    for res in reactions_data.results {
                        if let ReactionKey::CustomEmoji { document_id } = res.reaction {
                            ids.insert(document_id);
                        }
                    }
                    for reactor in reactions_data.recent_reactors {
                        if let ReactionKey::CustomEmoji { document_id } = reactor.reaction {
                            ids.insert(document_id);
                        }
                    }
                }
            }

            Ok(ids.into_iter().collect())
        })
    }
}
