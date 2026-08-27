use rusqlite::{Row, params};
use vendetta_model::{PeerId, PeerRecord, PeerType};

use crate::{db::ArchiveDb, error::StorageResult};

fn map_peer_row(row: &Row<'_>) -> rusqlite::Result<PeerRecord> {
    let p_type_str: String = row.get(1)?;
    let peer_type = p_type_str.parse().unwrap_or(PeerType::User);

    Ok(PeerRecord {
        peer_id: PeerId::new(row.get(0)?),
        peer_type,
        name: row.get(2)?,
        username: row.get(3)?,
        phone: row.get(4)?,
        raw_tl: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

impl ArchiveDb {
    pub fn upsert_peer(&self, peer: &PeerRecord) -> StorageResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO peers (peer_id, peer_type, name, username, phone, raw_tl, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(peer_id) DO UPDATE SET
                    peer_type = excluded.peer_type,
                    name = excluded.name,
                    username = excluded.username,
                    phone = excluded.phone,
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
        })
    }

    pub fn get_peer(&self, peer_id: PeerId) -> StorageResult<Option<PeerRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT peer_id, peer_type, name, username, phone, raw_tl, updated_at
                 FROM peers WHERE peer_id = ?1",
            )?;
            let mut rows = stmt.query(params![peer_id.raw()])?;

            if let Some(row) = rows.next()? {
                Ok(Some(map_peer_row(row)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn list_peers(&self) -> StorageResult<Vec<PeerRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT peer_id, peer_type, name, username, phone, raw_tl, updated_at
                 FROM peers ORDER BY updated_at DESC",
            )?;
            stmt.query_map([], map_peer_row)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
    }

    pub fn list_dialog_peers_with_messages(&self) -> StorageResult<Vec<PeerRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT p.peer_id, p.peer_type, p.name, p.username, p.phone, p.raw_tl, p.updated_at,
                        COALESCE((SELECT MAX(m.date) FROM messages m WHERE m.peer_id = p.peer_id), p.updated_at) AS last_activity
                 FROM peers p
                 WHERE p.peer_id IN (SELECT DISTINCT peer_id FROM messages)
                 ORDER BY last_activity DESC, p.updated_at DESC",
            )?;
            stmt.query_map([], map_peer_row)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
    }
}
