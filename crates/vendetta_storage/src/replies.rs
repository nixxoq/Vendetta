use rusqlite::{Row, params};
use vendetta_model::{MessageKey, MessageReplyRecord, ReplyResolutionStatus};

use crate::{db::ArchiveDb, error::StorageResult};

fn map_reply_row(row: &Row<'_>) -> rusqlite::Result<MessageReplyRecord> {
    let status_str: String = row.get(5)?;
    let resolution_status = status_str.parse().unwrap_or(ReplyResolutionStatus::Missing);

    Ok(MessageReplyRecord {
        source: MessageKey::new(row.get::<_, i64>(0)?, row.get::<_, i64>(1)?),
        target: MessageKey::new(row.get::<_, i64>(2)?, row.get::<_, i64>(3)?),
        top_message_id: row.get(4)?,
        resolution_status,
    })
}

impl ArchiveDb {
    pub fn upsert_reply(&self, reply: &MessageReplyRecord) -> StorageResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO message_replies (
                    source_peer_id, source_message_id, target_peer_id, target_message_id,
                    top_message_id, resolution_status
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(source_peer_id, source_message_id) DO UPDATE SET
                    target_peer_id = excluded.target_peer_id,
                    target_message_id = excluded.target_message_id,
                    top_message_id = excluded.top_message_id,
                    resolution_status = excluded.resolution_status;",
                params![
                    reply.source.peer_id.raw(),
                    reply.source.message_id.raw(),
                    reply.target.peer_id.raw(),
                    reply.target.message_id.raw(),
                    reply.top_message_id,
                    reply.resolution_status.as_ref(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_reply(&self, source: MessageKey) -> StorageResult<Option<MessageReplyRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT source_peer_id, source_message_id, target_peer_id, target_message_id,
                        top_message_id, resolution_status
                 FROM message_replies WHERE source_peer_id = ?1 AND source_message_id = ?2",
            )?;
            let mut rows = stmt.query(params![source.peer_id.raw(), source.message_id.raw()])?;

            if let Some(row) = rows.next()? {
                Ok(Some(map_reply_row(row)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn list_replies_to(&self, target: MessageKey) -> StorageResult<Vec<MessageReplyRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT source_peer_id, source_message_id, target_peer_id, target_message_id,
                        top_message_id, resolution_status
                 FROM message_replies WHERE target_peer_id = ?1 AND target_message_id = ?2",
            )?;
            stmt.query_map(
                params![target.peer_id.raw(), target.message_id.raw()],
                map_reply_row,
            )?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
        })
    }
}
