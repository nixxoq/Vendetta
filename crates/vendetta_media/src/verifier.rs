use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, sync::Arc};
use tracing::{debug, info, warn};
use vendetta_model::{MediaDownloadStatus, MediaRecord, MediaVerificationStatus};
use vendetta_storage::ArchiveDb;

use crate::{error::MediaEngineResult, storage_layout::StorageLayoutManager};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerificationReport {
    pub total_checked: usize,
    pub verified_count: usize,
    pub missing_count: usize,
    pub corrupted_size_count: usize,
    pub corrupted_hash_count: usize,
}

pub struct MediaVerifier {
    db: Arc<ArchiveDb>,
    layout: StorageLayoutManager,
}

impl MediaVerifier {
    pub fn new(db: Arc<ArchiveDb>, layout: StorageLayoutManager) -> Self {
        Self { db, layout }
    }

    pub fn verify_all_completed(&self) -> MediaEngineResult<VerificationReport> {
        self.verify_all_completed_with_progress(|_, _, _| {})
    }

    pub fn verify_all_completed_with_progress<F>(
        &self,
        progress: F,
    ) -> MediaEngineResult<VerificationReport>
    where
        F: Fn(usize, usize, &VerificationReport),
    {
        let mut report = VerificationReport::default();
        let completed = self
            .db
            .list_media_by_status(MediaDownloadStatus::Completed, usize::MAX)?;

        report.total_checked = completed.len();
        debug!(
            "Verifying {} completed media objects on disk",
            completed.len()
        );

        let mut buffer = vec![0u8; 65536];

        for (idx, record) in completed.into_iter().enumerate() {
            let status = self.verify_single_record(&record, &mut buffer);

            match status {
                MediaVerificationStatus::Verified => report.verified_count += 1,
                MediaVerificationStatus::MissingFile => report.missing_count += 1,
                MediaVerificationStatus::CorruptedSize => report.corrupted_size_count += 1,
                MediaVerificationStatus::CorruptedHash => report.corrupted_hash_count += 1,
                _ => {}
            }

            self.db
                .update_media_verification_status(&record.media_id, status)?;
            progress(idx + 1, report.total_checked, &report);
        }

        info!(
            "Media verification complete: {} checked ({} verified, {} missing, {} size mismatch, {} hash mismatch)",
            report.total_checked,
            report.verified_count,
            report.missing_count,
            report.corrupted_size_count,
            report.corrupted_hash_count
        );

        Ok(report)
    }

    fn verify_single_record(
        &self,
        record: &MediaRecord,
        buf: &mut [u8],
    ) -> MediaVerificationStatus {
        let Some(rel_path) = &record.local_rel_path else {
            warn!("Completed media {} has no local_rel_path", record.media_id);
            return MediaVerificationStatus::MissingFile;
        };

        let abs_path = self.layout.resolve_path(rel_path);
        let Ok(mut file) = File::open(&abs_path) else {
            warn!("Media file missing on disk: {:?}", abs_path);
            return MediaVerificationStatus::MissingFile;
        };

        let Ok(metadata) = file.metadata() else {
            return MediaVerificationStatus::MissingFile;
        };

        let file_len = metadata.len() as i64;
        if let Some(expected_size) = record.size_bytes
            && file_len != expected_size
        {
            warn!(
                "Media size mismatch for {}: expected {}, found {}",
                record.media_id, expected_size, file_len
            );
            return MediaVerificationStatus::CorruptedSize;
        }

        let mut hasher = Sha256::new();
        while let Ok(n) = file.read(buf) {
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }

        let computed_hash = format!("{:x}", hasher.finalize());
        if let Some(expected_hash) = &record.sha256
            && !computed_hash.eq_ignore_ascii_case(expected_hash)
        {
            warn!(
                "Media hash mismatch for {}: expected {}, computed {}",
                record.media_id, expected_hash, computed_hash
            );
            return MediaVerificationStatus::CorruptedHash;
        }

        MediaVerificationStatus::Verified
    }
}
