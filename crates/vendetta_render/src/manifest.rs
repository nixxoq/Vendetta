use std::{
    fs::{self, File},
    path::Path,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vendetta_storage::{ArchiveDb, StorageResult};

use crate::model::{ExportOptions, ExportSummary};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DatasetFingerprint {
    pub db_schema_version: u32,
    pub total_messages: usize,
    pub total_peers: usize,
    pub total_media: usize,
    pub source_digest: String,
}

impl DatasetFingerprint {
    pub fn compute(
        db_schema_version: u32,
        total_messages: usize,
        total_peers: usize,
        total_media: usize,
        peer_id_list: &[i64],
        content_digest: Option<&str>,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(db_schema_version.to_le_bytes());
        hasher.update((total_messages as u64).to_le_bytes());
        hasher.update((total_peers as u64).to_le_bytes());
        hasher.update((total_media as u64).to_le_bytes());

        for pid in peer_id_list {
            hasher.update(pid.to_le_bytes());
        }

        if let Some(cd) = content_digest {
            hasher.update(cd.as_bytes());
        }

        let source_digest = format!("{:x}", hasher.finalize());

        Self {
            db_schema_version,
            total_messages,
            total_peers,
            total_media,
            source_digest,
        }
    }

    pub fn compute_from_db(db: &ArchiveDb) -> StorageResult<Self> {
        db.with_conn(|conn| {
            let schema_version: u32 = conn
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| row.get(0))
                .unwrap_or(1);

            let total_messages: usize = conn
                .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get::<_, i64>(0))
                .unwrap_or(0) as usize;

            let total_peers: usize = conn
                .query_row("SELECT COUNT(*) FROM peers", [], |row| row.get::<_, i64>(0))
                .unwrap_or(0) as usize;

            let total_media: usize = conn
                .query_row("SELECT COUNT(*) FROM media_objects", [], |row| row.get::<_, i64>(0))
                .unwrap_or(0) as usize;

            let mut peer_stmt = conn.prepare("SELECT peer_id FROM peers ORDER BY peer_id ASC")?;
            let peer_id_list: Vec<i64> = peer_stmt
                .query_map([], |row| row.get(0))?
                .filter_map(Result::ok)
                .collect();

            let mut msg_stmt = conn.prepare(
                "SELECT m.peer_id, m.message_id, m.date, m.state, m.text, COALESCE(mo.sha256, '')
                 FROM messages m
                 LEFT JOIN message_media mm ON m.peer_id = mm.peer_id AND m.message_id = mm.message_id
                 LEFT JOIN media_objects mo ON mm.media_id = mo.media_id
                 ORDER BY m.peer_id ASC, m.message_id ASC",
            )?;

            let mut content_hasher = Sha256::new();
            let rows = msg_stmt.query_map([], |row| {
                let pid: i64 = row.get(0)?;
                let mid: i64 = row.get(1)?;
                let date: i64 = row.get(2)?;
                let state: String = row.get(3)?;
                let text: Option<String> = row.get(4)?;
                let media_sha: String = row.get(5)?;
                Ok((pid, mid, date, state, text, media_sha))
            })?;

            for r in rows {
                let (pid, mid, date, state, text, media_sha) = r?;
                content_hasher.update(pid.to_le_bytes());
                content_hasher.update(mid.to_le_bytes());
                content_hasher.update(date.to_le_bytes());
                content_hasher.update(state.as_bytes());
                if let Some(t) = text {
                    content_hasher.update(t.as_bytes());
                }
                if !media_sha.is_empty() {
                    content_hasher.update(media_sha.as_bytes());
                }
            }

            let content_digest_hex = format!("{:x}", content_hasher.finalize());

            Ok(Self::compute(
                schema_version,
                total_messages,
                total_peers,
                total_media,
                &peer_id_list,
                Some(&content_digest_hex),
            ))
        })
    }
}

pub fn compute_export_config_fingerprint(options: &ExportOptions) -> String {
    let mut hasher = Sha256::new();
    hasher.update(options.presentation_mode.as_ref().as_bytes());
    hasher.update(options.theme.as_ref().as_bytes());
    hasher.update(options.media_mode.as_ref().as_bytes());
    hasher.update((options.chunk_size as u64).to_le_bytes());
    hasher.update([options.include_service_messages as u8]);
    hasher.update([options.include_deleted_messages as u8]);
    hasher.update([options.include_edit_history as u8]);
    hasher.update([options.build_search_index as u8]);
    hasher.update([options.build_date_index as u8]);

    if let Some(targets) = &options.target_peers {
        for pid in targets {
            hasher.update(pid.raw().to_le_bytes());
        }
    }

    format!("{:x}", hasher.finalize())
}

pub fn compute_export_config_fingerprint_full(
    options: &ExportOptions,
    readable_names: bool,
    from_date: Option<i64>,
    to_date: Option<i64>,
    split_by: crate::model::SplitBy,
    date_structure: crate::model::DateStructure,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(compute_export_config_fingerprint(options).as_bytes());
    hasher.update([readable_names as u8]);
    if let Some(fd) = from_date {
        hasher.update(fd.to_le_bytes());
    }
    if let Some(td) = to_date {
        hasher.update(td.to_le_bytes());
    }
    hasher.update(split_by.as_ref().as_bytes());
    hasher.update(date_structure.as_ref().as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn compute_chat_fingerprint(
    db: &ArchiveDb,
    peer_id: i64,
    export_config_fingerprint: &str,
    from_date: Option<i64>,
    to_date: Option<i64>,
) -> StorageResult<String> {
    db.with_conn(|conn| {
        let mut hasher = Sha256::new();
        hasher.update(peer_id.to_le_bytes());
        hasher.update(export_config_fingerprint.as_bytes());
        if let Some(fd) = from_date {
            hasher.update(fd.to_le_bytes());
        }
        if let Some(td) = to_date {
            hasher.update(td.to_le_bytes());
        }

        let peer_name: Option<String> = conn
            .query_row(
                "SELECT name FROM peers WHERE peer_id = ?1",
                [peer_id],
                |row| row.get(0),
            )
            .unwrap_or(None);
        if let Some(name) = peer_name {
            hasher.update(name.as_bytes());
        }

        let query = match (from_date, to_date) {
            (Some(f), Some(t)) => format!(
                "SELECT m.message_id, m.date, m.state, m.text, COALESCE(mo.sha256, '')
                 FROM messages m
                 LEFT JOIN message_media mm ON m.peer_id = mm.peer_id AND m.message_id = mm.message_id
                 LEFT JOIN media_objects mo ON mm.media_id = mo.media_id
                 WHERE m.peer_id = {peer_id} AND m.date >= {f} AND m.date <= {t}
                 ORDER BY m.message_id ASC"
            ),
            (Some(f), None) => format!(
                "SELECT m.message_id, m.date, m.state, m.text, COALESCE(mo.sha256, '')
                 FROM messages m
                 LEFT JOIN message_media mm ON m.peer_id = mm.peer_id AND m.message_id = mm.message_id
                 LEFT JOIN media_objects mo ON mm.media_id = mo.media_id
                 WHERE m.peer_id = {peer_id} AND m.date >= {f}
                 ORDER BY m.message_id ASC"
            ),
            (None, Some(t)) => format!(
                "SELECT m.message_id, m.date, m.state, m.text, COALESCE(mo.sha256, '')
                 FROM messages m
                 LEFT JOIN message_media mm ON m.peer_id = mm.peer_id AND m.message_id = mm.message_id
                 LEFT JOIN media_objects mo ON mm.media_id = mo.media_id
                 WHERE m.peer_id = {peer_id} AND m.date <= {t}
                 ORDER BY m.message_id ASC"
            ),
            (None, None) => format!(
                "SELECT m.message_id, m.date, m.state, m.text, COALESCE(mo.sha256, '')
                 FROM messages m
                 LEFT JOIN message_media mm ON m.peer_id = mm.peer_id AND m.message_id = mm.message_id
                 LEFT JOIN media_objects mo ON mm.media_id = mo.media_id
                 WHERE m.peer_id = {peer_id}
                 ORDER BY m.message_id ASC"
            ),
        };

        let mut stmt = conn.prepare(&query)?;
        let rows = stmt.query_map([], |row| {
            let mid: i64 = row.get(0)?;
            let date: i64 = row.get(1)?;
            let state: String = row.get(2)?;
            let text: Option<String> = row.get(3)?;
            let media_sha: String = row.get(4)?;
            Ok((mid, date, state, text, media_sha))
        })?;

        for r in rows {
            let (mid, date, state, text, media_sha) = r?;
            hasher.update(mid.to_le_bytes());
            hasher.update(date.to_le_bytes());
            hasher.update(state.as_bytes());
            if let Some(t) = text {
                hasher.update(t.as_bytes());
            }
            if !media_sha.is_empty() {
                hasher.update(media_sha.as_bytes());
            }
        }

        Ok(format!("{:x}", hasher.finalize()))
    })
}

pub fn compute_day_fingerprint(
    day_msgs: &[crate::model::RenderMessage],
    config_fingerprint: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(config_fingerprint.as_bytes());
    for m in day_msgs {
        hasher.update(m.key.message_id.raw().to_le_bytes());
        hasher.update(m.date.to_le_bytes());
        for rev in &m.revisions {
            if let Some(ed) = rev.edit_date {
                hasher.update(ed.to_le_bytes());
            }
            if let Some(rt) = &rev.raw_text {
                hasher.update(rt.as_bytes());
            }
        }
        if let Some(t) = &m.raw_text {
            hasher.update(t.as_bytes());
        }
        hasher.update(format!("{:?}", m.state).as_bytes());
        for r in &m.reactions {
            hasher.update(format!("{:?}", r.reaction).as_bytes());
            hasher.update((r.count as u64).to_le_bytes());
        }
        for med in &m.media_items {
            hasher.update(med.record.media_id.as_bytes());
            if let Some(sha) = &med.record.sha256 {
                hasher.update(sha.as_bytes());
            }
        }
    }
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ManifestChatEntry {
    pub peer_id: i64,
    pub directory: String,
    #[serde(default)]
    pub title: Option<String>,
    pub fingerprint: String,
    #[serde(default)]
    pub pages: Vec<String>,
    #[serde(default)]
    pub day_fingerprints: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub media_files: Vec<String>,
    #[serde(default)]
    pub avatar_files: Vec<String>,
    #[serde(default)]
    pub reaction_files: Vec<String>,
    #[serde(default)]
    pub topic_assets: Vec<String>,
}

fn default_export_format() -> String {
    "chat-portable-v1".to_string()
}

fn default_renderer_version() -> String {
    "vendetta_render_v2".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HtmlExportManifest {
    pub format_version: u32,
    #[serde(default = "default_export_format")]
    pub export_format: String,
    #[serde(default = "default_renderer_version")]
    pub renderer_version: String,
    #[serde(default)]
    pub readable_names: bool,
    #[serde(default)]
    pub from_date: Option<i64>,
    #[serde(default)]
    pub to_date: Option<i64>,
    #[serde(default)]
    pub split_by: Option<String>,
    #[serde(default)]
    pub date_structure: Option<String>,
    pub presentation_mode: String,
    pub media_mode: String,
    pub chunk_size: usize,
    pub source_fingerprint: DatasetFingerprint,
    pub export_config_fingerprint: String,
    pub summary: ExportSummary,
    #[serde(default)]
    pub chats: Vec<ManifestChatEntry>,
}

impl HtmlExportManifest {
    pub fn write_to_file(&self, path: &Path) -> std::io::Result<()> {
        let tmp_path = path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&tmp_path, content)?;
        fs::rename(&tmp_path, path)?;
        Ok(())
    }

    pub fn read_from_file(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let manifest = serde_json::from_reader(file)?;
        Ok(manifest)
    }
}
