use std::{io::SeekFrom, sync::Arc};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    task,
};

use sha2::{Digest, Sha256};
use tracing::debug;
use vendetta_model::{FileRangeHash, MediaDownloadStatus, MediaRecord};
use vendetta_storage::ArchiveDb;
use vendetta_tg_adapter::{AdapterError, TelegramAdapter};

use crate::error::{MediaEngineError, MediaEngineResult};
use crate::storage_layout::StorageLayoutManager;

pub const FRAGMENT_SIZE: i64 = 1024 * 1024;
pub const DEFAULT_CHUNK_SIZE: i32 = 524_288;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChunkPlannerError {
    #[error("Offset {0} is unaligned (must be a multiple of 1024 bytes)")]
    UnalignedOffset(i64),
    #[error("Offset {0} cannot be negative")]
    NegativeOffset(i64),
}

pub struct ChunkPlanner;

impl ChunkPlanner {
    pub fn plan_next_chunk(
        offset: i64,
        configured_chunk_size: i32,
    ) -> Result<i32, ChunkPlannerError> {
        if offset < 0 {
            return Err(ChunkPlannerError::NegativeOffset(offset));
        }
        if offset % 1024 != 0 {
            return Err(ChunkPlannerError::UnalignedOffset(offset));
        }

        let fragment_offset = offset % FRAGMENT_SIZE;
        let remaining_in_fragment = FRAGMENT_SIZE - fragment_offset;

        let limit = (configured_chunk_size as i64).min(remaining_in_fragment);
        let aligned = (limit / 1024) * 1024;
        Ok(aligned.max(1024) as i32)
    }
}

fn verify_range_hashes(
    chunk_bytes: &[u8],
    chunk_start: i64,
    range_hashes: &[FileRangeHash],
) -> MediaEngineResult<()> {
    let chunk_len = chunk_bytes.len() as i64;
    let chunk_end = chunk_start + chunk_len;

    let mut covering: Vec<&FileRangeHash> = range_hashes
        .iter()
        .filter(|rh| rh.offset >= chunk_start && (rh.offset + rh.limit as i64) <= chunk_end)
        .collect();
    covering.sort_by_key(|rh| rh.offset);

    if covering.is_empty() {
        return Ok(());
    }

    let mut expected_cursor = chunk_start;
    for rh in &covering {
        if rh.offset != expected_cursor {
            return Ok(());
        }
        expected_cursor += rh.limit as i64;
    }

    if expected_cursor != chunk_end {
        return Ok(());
    }

    for rh in covering {
        let slice_start = (rh.offset - chunk_start) as usize;
        let slice_end = slice_start + rh.limit as usize;
        let sub_slice = &chunk_bytes[slice_start..slice_end];

        let mut chunk_hasher = Sha256::new();
        chunk_hasher.update(sub_slice);
        let actual_hash = chunk_hasher.finalize();

        if actual_hash.as_slice() != rh.hash.as_slice() {
            return Err(MediaEngineError::ChunkHashMismatch {
                offset: rh.offset,
                expected: rh
                    .hash
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>(),
                actual: format!("{actual_hash:x}"),
            });
        }
    }

    Ok(())
}

pub struct SingleMediaDownloader {
    db: Arc<ArchiveDb>,
    adapter: Arc<dyn TelegramAdapter>,
    layout: StorageLayoutManager,
}

impl SingleMediaDownloader {
    pub fn new(
        db: Arc<ArchiveDb>,
        adapter: Arc<dyn TelegramAdapter>,
        layout: StorageLayoutManager,
    ) -> Self {
        Self {
            db,
            adapter,
            layout,
        }
    }

    pub async fn download_item(&self, record: &mut MediaRecord) -> MediaEngineResult<String> {
        let location_tl = record
            .source_location_tl
            .as_ref()
            .ok_or_else(|| MediaEngineError::UnsupportedLocation(record.media_id.clone()))?;

        self.layout.ensure_dirs()?;
        let temp_path = self.layout.temp_part_path(&record.media_id);

        let committed_bytes = record.downloaded_bytes;
        if committed_bytes < 0 {
            return Err(MediaEngineError::InvalidState(format!(
                "Negative committed bytes ({committed_bytes}) for media {}",
                record.media_id
            )));
        }

        if record.download_status == MediaDownloadStatus::Completed
            && let Some(h) = &record.sha256
        {
            return Ok(h.clone());
        }

        if committed_bytes % 1024 != 0 {
            return Err(MediaEngineError::CorruptedProgress {
                media_id: record.media_id.clone(),
                downloaded_bytes: committed_bytes,
                reason:
                    "Persisted intermediate downloaded_bytes is unaligned (not divisible by 1024)"
                        .to_string(),
            });
        }

        if fs::try_exists(&temp_path).await.unwrap_or(false) {
            let metadata = fs::metadata(&temp_path).await?;
            let file_len = metadata.len() as i64;
            if file_len > committed_bytes {
                debug!(
                    "Crash recovery: truncating uncommitted bytes in {:?} from {} to {}",
                    temp_path, file_len, committed_bytes
                );
                let file = OpenOptions::new().write(true).open(&temp_path).await?;
                file.set_len(committed_bytes as u64).await?;
            } else if file_len < committed_bytes {
                return Err(MediaEngineError::CorruptedProgress {
                    media_id: record.media_id.clone(),
                    downloaded_bytes: committed_bytes,
                    reason: format!(
                        "Disk .part file length ({file_len}) is smaller than committed progress ({committed_bytes})"
                    ),
                });
            }
        }

        let mut hasher = Sha256::new();
        if committed_bytes > 0 && fs::try_exists(&temp_path).await.unwrap_or(false) {
            let mut file = File::open(&temp_path).await?;
            let mut buffer = vec![0u8; DEFAULT_CHUNK_SIZE as usize];
            let mut remaining_to_hash = committed_bytes;

            while remaining_to_hash > 0 {
                let to_read = (buffer.len() as i64).min(remaining_to_hash) as usize;
                file.read_exact(&mut buffer[..to_read]).await?;
                hasher.update(&buffer[..to_read]);
                remaining_to_hash -= to_read as i64;
            }
            debug!(
                "Seeded SHA-256 hasher with {} committed bytes for {}",
                committed_bytes, record.media_id
            );
        }

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&temp_path)
            .await?;
        file.seek(SeekFrom::Start(committed_bytes as u64)).await?;

        let mut current_offset = committed_bytes;
        let chunk_size = if record.chunk_size > 0 {
            record.chunk_size
        } else {
            DEFAULT_CHUNK_SIZE
        };

        loop {
            if let Some(expected_size) = record.size_bytes
                && current_offset >= expected_size
            {
                break;
            }

            let planned_limit = ChunkPlanner::plan_next_chunk(current_offset, chunk_size)?;

            debug!(
                "Downloading chunk for {} at offset {}, limit {}",
                record.media_id, current_offset, planned_limit
            );

            let range_hashes_result = self
                .adapter
                .get_file_hashes(location_tl, record.dc_id, current_offset)
                .await;

            let range_hashes: Vec<FileRangeHash> = match range_hashes_result {
                Ok(hashes) => hashes,
                Err(err) => {
                    if let AdapterError::Invocation(msg) = &err
                        && (msg.contains("LOCATION_INVALID")
                            || msg.contains("FILE_ID_INVALID")
                            || msg.contains("METHOD_NOT_SUPPORTED"))
                    {
                        Vec::new()
                    } else {
                        return Err(MediaEngineError::from(err));
                    }
                }
            };

            let chunk_bytes = self
                .adapter
                .download_file_chunk(location_tl, record.dc_id, current_offset, planned_limit)
                .await?;

            if chunk_bytes.is_empty() {
                if let Some(expected_size) = record.size_bytes
                    && current_offset < expected_size
                {
                    return Err(MediaEngineError::FinalSizeMismatch {
                        media_id: record.media_id.clone(),
                        expected: expected_size,
                        actual: current_offset,
                    });
                }
                debug!(
                    "Received 0 bytes (EOF) for {} at offset {}",
                    record.media_id, current_offset
                );
                break;
            }

            verify_range_hashes(&chunk_bytes, current_offset, &range_hashes)?;
            debug!(
                "Verified contiguous server range hashes for {} at offset {}",
                record.media_id, current_offset
            );

            file.write_all(&chunk_bytes).await?;
            file.flush().await?;
            hasher.update(&chunk_bytes);

            current_offset += chunk_bytes.len() as i64;

            let is_short_chunk = chunk_bytes.len() < planned_limit as usize;
            if is_short_chunk {
                if let Some(expected_size) = record.size_bytes
                    && current_offset < expected_size
                {
                    return Err(MediaEngineError::FinalSizeMismatch {
                        media_id: record.media_id.clone(),
                        expected: expected_size,
                        actual: current_offset,
                    });
                }
                debug!(
                    "Received short chunk ({} < {}) indicating EOF for {}",
                    chunk_bytes.len(),
                    planned_limit,
                    record.media_id
                );
                break;
            }

            self.db
                .update_media_progress(&record.media_id, current_offset)?;
            record.downloaded_bytes = current_offset;
        }

        if let Some(expected_size) = record.size_bytes
            && current_offset != expected_size
        {
            return Err(MediaEngineError::FinalSizeMismatch {
                media_id: record.media_id.clone(),
                expected: expected_size,
                actual: current_offset,
            });
        }

        let final_hash = format!("{:x}", hasher.finalize());
        let rel_path = StorageLayoutManager::content_addressed_rel_path(
            &final_hash,
            record.file_name.as_deref(),
        );
        let final_dest = self.layout.resolve_path(&rel_path);

        let layout = self.layout.clone();
        let temp_path_clone = temp_path.clone();
        let final_dest_clone = final_dest.clone();
        let final_hash_clone = final_hash.clone();

        task::spawn_blocking(move || {
            layout.finalize_temp_file(
                &temp_path_clone,
                &final_dest_clone,
                &final_hash_clone,
                current_offset,
            )
        })
        .await
        .map_err(|e| MediaEngineError::Internal(e.to_string()))??;

        self.db
            .update_media_completed(&record.media_id, &final_hash, &rel_path)?;

        debug!(
            "Successfully finalized media {} -> {} ({} bytes)",
            record.media_id, rel_path, current_offset
        );

        Ok(final_hash)
    }
}
