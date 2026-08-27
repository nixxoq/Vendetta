use std::{
    fs::{self, OpenOptions},
    sync::Arc,
};
use tracing::{debug, info, warn};
use vendetta_model::{MediaDownloadStatus, MediaVerificationStatus};
use vendetta_storage::ArchiveDb;

use crate::error::MediaEngineResult;
use crate::storage_layout::StorageLayoutManager;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconciliationReport {
    pub downloading_reset_count: usize,
    pub completed_promoted_count: usize,
    pub missing_file_marked_count: usize,
    pub corrupted_file_marked_count: usize,
    pub orphan_part_cleaned_count: usize,
}

pub struct StartupReconciler {
    db: Arc<ArchiveDb>,
    layout: StorageLayoutManager,
}

impl StartupReconciler {
    pub fn new(db: Arc<ArchiveDb>, layout: StorageLayoutManager) -> Self {
        Self { db, layout }
    }

    pub fn reconcile(&self) -> MediaEngineResult<ReconciliationReport> {
        let mut report = ReconciliationReport::default();
        self.layout.ensure_dirs()?;

        let downloading_items = self
            .db
            .list_media_by_status(MediaDownloadStatus::Downloading, usize::MAX)?;

        for record in downloading_items {
            let part_path = self.layout.temp_part_path(&record.media_id);

            if let Some(sha256) = &record.sha256 {
                let rel_path = StorageLayoutManager::content_addressed_rel_path(
                    sha256,
                    record.file_name.as_deref(),
                );
                let abs_path = self.layout.resolve_path(&rel_path);
                if let Ok(metadata) = fs::metadata(&abs_path) {
                    let expected_size = record.size_bytes.unwrap_or(metadata.len() as i64);
                    if metadata.len() as i64 == expected_size {
                        debug!(
                            "Startup reconciliation (Case B): Promoting {} to Completed",
                            record.media_id
                        );
                        self.db
                            .update_media_completed(&record.media_id, sha256, &rel_path)?;
                        let _ = fs::remove_file(&part_path);
                        report.completed_promoted_count += 1;
                        continue;
                    }
                }
            }

            if let Ok(metadata) = fs::metadata(&part_path) {
                let file_len = metadata.len() as i64;
                if file_len > record.downloaded_bytes {
                    debug!(
                        "Startup reconciliation (Case A): Truncating {:?} from {} to {}",
                        part_path, file_len, record.downloaded_bytes
                    );
                    if let Ok(file) = OpenOptions::new().write(true).open(&part_path) {
                        let _ = file.set_len(record.downloaded_bytes as u64);
                    }
                }
            }

            self.db.update_media_status(
                &record.media_id,
                MediaDownloadStatus::Pending,
                None,
                None,
            )?;
            report.downloading_reset_count += 1;
        }

        let completed_items = self
            .db
            .list_media_by_status(MediaDownloadStatus::Completed, usize::MAX)?;

        for record in completed_items {
            let Some(rel_path) = &record.local_rel_path else {
                debug!(
                    "Startup reconciliation (Case C): Missing local_rel_path for {}, re-queuing",
                    record.media_id
                );
                self.db
                    .requeue_missing_media_for_recovery(&record.media_id)?;
                report.missing_file_marked_count += 1;
                continue;
            };

            let abs_path = self.layout.resolve_path(rel_path);
            let metadata = match fs::metadata(&abs_path) {
                Ok(m) => m,
                Err(_) => {
                    debug!(
                        "Startup reconciliation (Case C): Missing final file for {}, re-queuing",
                        record.media_id
                    );
                    self.db
                        .requeue_missing_media_for_recovery(&record.media_id)?;
                    report.missing_file_marked_count += 1;
                    continue;
                }
            };

            if let Some(expected_size) = record.size_bytes
                && metadata.len() as i64 != expected_size
            {
                debug!(
                    "Startup reconciliation (Case D): Size mismatch for {}",
                    record.media_id
                );
                self.db.update_media_verification_status(
                    &record.media_id,
                    MediaVerificationStatus::CorruptedSize,
                )?;
                report.corrupted_file_marked_count += 1;
            } else if record.verification_status == MediaVerificationStatus::MissingFile {
                self.db.update_media_verification_status(
                    &record.media_id,
                    MediaVerificationStatus::Verified,
                )?;
                report.completed_promoted_count += 1;
            }
        }

        let temp_dir = self.layout.temp_dir();
        if let Ok(entries) = fs::read_dir(&temp_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "part")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                    && self.db.get_media(stem)?.is_none() {
                        debug!(
                            "Startup reconciliation (Case E): Removing orphan .part file {:?}",
                            path
                        );
                        let _ = fs::remove_file(&path);
                        report.orphan_part_cleaned_count += 1;
                    }
            }
        }

        if report.missing_file_marked_count > 0 || report.corrupted_file_marked_count > 0 {
            warn!(
                "Startup reconciliation: {} missing marked (re-queued for recovery), {} corrupted marked, {} downloading reset, {} completed promoted, {} orphan parts cleaned",
                report.missing_file_marked_count,
                report.corrupted_file_marked_count,
                report.downloading_reset_count,
                report.completed_promoted_count,
                report.orphan_part_cleaned_count
            );
        } else {
            info!(
                "Startup reconciliation complete: {} downloading reset, {} completed promoted, {} missing marked, {} corrupted marked, {} orphan parts cleaned",
                report.downloading_reset_count,
                report.completed_promoted_count,
                report.missing_file_marked_count,
                report.corrupted_file_marked_count,
                report.orphan_part_cleaned_count
            );
        }

        Ok(report)
    }
}
