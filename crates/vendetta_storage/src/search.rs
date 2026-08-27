use rusqlite::ToSql;
use serde::{Deserialize, Serialize};
use vendetta_model::{MessageId, MessageKey, PeerId};

use crate::{db::ArchiveDb, error::StorageResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FtsSearchParams {
    pub query: String,
    pub peer_id: Option<PeerId>,
    pub sender_id: Option<PeerId>,
    pub min_date: Option<i64>,
    pub max_date: Option<i64>,
    pub limit: usize,
    pub offset: usize,
}

impl Default for FtsSearchParams {
    fn default() -> Self {
        Self {
            query: String::new(),
            peer_id: None,
            sender_id: None,
            min_date: None,
            max_date: None,
            limit: 50,
            offset: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FtsSearchResult {
    pub key: MessageKey,
    pub sender_id: Option<PeerId>,
    pub date: i64,
    pub text: String,
    pub snippet: Option<String>,
    pub rank: f64,
}

impl ArchiveDb {
    pub fn search_fts(&self, params: &FtsSearchParams) -> StorageResult<Vec<FtsSearchResult>> {
        if params.query.trim().is_empty() {
            return Ok(Vec::new());
        }

        self.with_conn(|conn| {
            let mut sql = String::from(
                "SELECT peer_id, message_id, sender_id, date, text, snippet(messages_fts, 0, '<b>', '</b>', '...', 32) as snip, rank
                 FROM messages_fts
                 WHERE messages_fts MATCH ?1",
            );

            let peer_raw = params.peer_id.map(|p| p.raw());
            let sender_raw = params.sender_id.map(|s| s.raw());
            let limit = params.limit.max(1) as i64;
            let offset = params.offset as i64;

            let mut bindings: Vec<&dyn ToSql> = vec![&params.query];
            let mut param_idx = 2;

            if let Some(ref p) = peer_raw {
                sql.push_str(&format!(" AND peer_id = ?{param_idx}"));
                bindings.push(p);
                param_idx += 1;
            }

            if let Some(ref s) = sender_raw {
                sql.push_str(&format!(" AND sender_id = ?{param_idx}"));
                bindings.push(s);
                param_idx += 1;
            }

            if let Some(ref min_d) = params.min_date {
                sql.push_str(&format!(" AND CAST(date AS INTEGER) >= ?{param_idx}"));
                bindings.push(min_d);
                param_idx += 1;
            }

            if let Some(ref max_d) = params.max_date {
                sql.push_str(&format!(" AND CAST(date AS INTEGER) <= ?{param_idx}"));
                bindings.push(max_d);
                param_idx += 1;
            }

            sql.push_str(" ORDER BY rank");

            sql.push_str(&format!(" LIMIT ?{param_idx}"));
            bindings.push(&limit);
            param_idx += 1;

            if params.offset > 0 {
                sql.push_str(&format!(" OFFSET ?{param_idx}"));
                bindings.push(&offset);
            }

            let mut stmt = conn.prepare(&sql)?;
            stmt.query_map(bindings.as_slice(), |row| {
                let peer_id_raw: i64 = row.get(0)?;
                let message_id_raw: i64 = row.get(1)?;
                let sender_id_raw: Option<i64> = row.get(2)?;
                let date: i64 = row.get(3)?;
                let text: String = row.get(4)?;
                let snippet: Option<String> = row.get(5)?;
                let rank: f64 = row.get(6)?;

                Ok(FtsSearchResult {
                    key: MessageKey::new(
                        PeerId::new(peer_id_raw),
                        MessageId::new(message_id_raw),
                    ),
                    sender_id: sender_id_raw.map(PeerId::new),
                    date,
                    text,
                    snippet,
                    rank,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
        })
    }
}
