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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HtmlExportManifest {
    pub format_version: u32,
    pub presentation_mode: String,
    pub media_mode: String,
    pub chunk_size: usize,
    pub source_fingerprint: DatasetFingerprint,
    pub export_config_fingerprint: String,
    pub summary: ExportSummary,
}

impl HtmlExportManifest {
    pub fn write_to_file(&self, path: &Path) -> std::io::Result<()> {
        fs::write(path, serde_json::to_string_pretty(self)?)
    }

    pub fn read_from_file(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let manifest = serde_json::from_reader(file)?;
        Ok(manifest)
    }
}
