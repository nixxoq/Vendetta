use std::{collections::HashMap, fs, sync::Arc};
use tokio::{fs as tokio_fs, io::AsyncWriteExt};
use tracing::{debug, warn};

use grammers_tl_types::{self as tl, Serializable};
use vendetta_storage::ArchiveDb;
use vendetta_tg_adapter::TelegramAdapter;

use crate::error::{MediaEngineError, MediaEngineResult};
use crate::storage_layout::StorageLayoutManager;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustomReactionDownloadStatus {
    Downloaded(usize),
    AlreadyExists,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomReactionSyncSummary {
    pub total_discovered: usize,
    pub downloaded: usize,
    pub already_existed: usize,
    pub unavailable: usize,
    pub failed: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct CustomReactionSyncProgress {
    pub total_discovered: usize,
    pub processed: usize,
    pub downloaded: usize,
    pub already_existed: usize,
    pub failed: usize,
}

#[derive(Debug, Clone)]
pub struct CustomEmojiFileLocation {
    pub document_id: i64,
    pub dc_id: i32,
    pub location_tl: Vec<u8>,
    pub embedded_bytes: Option<Vec<u8>>,
}

fn doc_file_location(d: &tl::types::Document, thumb_size: String) -> CustomEmojiFileLocation {
    let input_loc = tl::enums::InputFileLocation::InputDocumentFileLocation(
        tl::types::InputDocumentFileLocation {
            id: d.id,
            access_hash: d.access_hash,
            file_reference: d.file_reference.clone(),
            thumb_size,
        },
    );
    CustomEmojiFileLocation {
        document_id: d.id,
        dc_id: d.dc_id,
        location_tl: input_loc.to_bytes(),
        embedded_bytes: None,
    }
}

pub fn extract_custom_emoji_location(doc: &tl::enums::Document) -> Option<CustomEmojiFileLocation> {
    let tl::enums::Document::Document(d) = doc else {
        return None;
    };

    if d.access_hash == 0 {
        return None;
    }

    if d.mime_type.starts_with("image/") {
        return Some(doc_file_location(d, String::new()));
    }

    if let Some(ref thumbs) = d.thumbs {
        for thumb in thumbs {
            match thumb {
                tl::enums::PhotoSize::Size(s) => {
                    return Some(doc_file_location(d, s.r#type.clone()));
                }
                tl::enums::PhotoSize::Progressive(p) => {
                    return Some(doc_file_location(d, p.r#type.clone()));
                }
                tl::enums::PhotoSize::PhotoCachedSize(c) => {
                    return Some(CustomEmojiFileLocation {
                        document_id: d.id,
                        dc_id: d.dc_id,
                        location_tl: Vec::new(),
                        embedded_bytes: Some(c.bytes.clone()),
                    });
                }
                tl::enums::PhotoSize::PhotoStrippedSize(st) => {
                    return Some(CustomEmojiFileLocation {
                        document_id: d.id,
                        dc_id: d.dc_id,
                        location_tl: Vec::new(),
                        embedded_bytes: Some(st.bytes.clone()),
                    });
                }
                _ => {}
            }
        }
    }

    Some(doc_file_location(d, String::new()))
}

pub async fn download_single_custom_reaction(
    adapter: &Arc<dyn TelegramAdapter>,
    storage_layout: &StorageLayoutManager,
    loc: &CustomEmojiFileLocation,
) -> MediaEngineResult<CustomReactionDownloadStatus> {
    let final_path = storage_layout.reaction_path(loc.document_id);

    if final_path.is_file() && fs::metadata(&final_path).is_ok_and(|m| m.len() > 0) {
        return Ok(CustomReactionDownloadStatus::AlreadyExists);
    }

    if let Some(parent) = final_path.parent() {
        tokio_fs::create_dir_all(parent).await?;
    }

    if let Some(ref bytes) = loc.embedded_bytes {
        if bytes.is_empty() {
            return Ok(CustomReactionDownloadStatus::Unavailable);
        }
        let temp_path = storage_layout.temp_part_path(&format!("reaction_{}", loc.document_id));
        tokio_fs::write(&temp_path, bytes).await?;
        tokio_fs::rename(&temp_path, &final_path).await?;
        return Ok(CustomReactionDownloadStatus::Downloaded(bytes.len()));
    }

    if loc.location_tl.is_empty() {
        return Ok(CustomReactionDownloadStatus::Unavailable);
    }

    let temp_path = storage_layout.temp_part_path(&format!("reaction_{}", loc.document_id));
    let mut file = tokio_fs::File::create(&temp_path).await?;

    let mut offset: i64 = 0;
    const CHUNK_LIMIT: i32 = 131_072;
    let mut total_downloaded: usize = 0;

    loop {
        let chunk = match adapter
            .download_file_chunk(&loc.location_tl, loc.dc_id, offset, CHUNK_LIMIT)
            .await
        {
            Ok(bytes) => bytes,
            Err(err) => {
                let _ = tokio_fs::remove_file(&temp_path).await;
                return Err(MediaEngineError::Adapter(err));
            }
        };

        if chunk.is_empty() {
            break;
        }

        let len = chunk.len();
        file.write_all(&chunk).await?;

        total_downloaded += len;
        offset += len as i64;

        if len < CHUNK_LIMIT as usize {
            break;
        }
    }

    file.flush().await?;
    drop(file);

    if total_downloaded == 0 {
        let _ = tokio_fs::remove_file(&temp_path).await;
        return Ok(CustomReactionDownloadStatus::Unavailable);
    }

    tokio_fs::rename(&temp_path, &final_path).await?;
    Ok(CustomReactionDownloadStatus::Downloaded(total_downloaded))
}

pub async fn sync_all_custom_reactions<F>(
    db: &ArchiveDb,
    adapter: &Arc<dyn TelegramAdapter>,
    storage_layout: &StorageLayoutManager,
    mut progress_cb: F,
) -> MediaEngineResult<CustomReactionSyncSummary>
where
    F: FnMut(&CustomReactionSyncProgress),
{
    storage_layout.ensure_dirs()?;

    let all_doc_ids = db.list_custom_emoji_reaction_document_ids()?;

    let mut summary = CustomReactionSyncSummary {
        total_discovered: all_doc_ids.len(),
        ..Default::default()
    };

    if all_doc_ids.is_empty() {
        return Ok(summary);
    }

    let mut missing_ids = Vec::new();
    for id in all_doc_ids {
        let final_path = storage_layout.reaction_path(id);
        if final_path.is_file() && fs::metadata(&final_path).is_ok_and(|m| m.len() > 0) {
            summary.already_existed += 1;
        } else {
            missing_ids.push(id);
        }
    }

    let mut progress = CustomReactionSyncProgress {
        total_discovered: summary.total_discovered,
        processed: summary.already_existed,
        downloaded: 0,
        already_existed: summary.already_existed,
        failed: 0,
    };
    progress_cb(&progress);

    if missing_ids.is_empty() {
        return Ok(summary);
    }

    const BATCH_SIZE: usize = 100;
    for chunk_ids in missing_ids.chunks(BATCH_SIZE) {
        let docs = match adapter.get_custom_emoji_documents(chunk_ids).await {
            Ok(d) => d,
            Err(e) => {
                warn!("Failed to query custom emoji documents batch: {e}");
                summary.failed += chunk_ids.len();
                progress.failed += chunk_ids.len();
                progress.processed += chunk_ids.len();
                progress_cb(&progress);
                continue;
            }
        };

        let doc_map: HashMap<i64, _> = docs
            .into_iter()
            .filter_map(|doc| {
                if let tl::enums::Document::Document(ref d) = doc {
                    Some((d.id, doc))
                } else {
                    None
                }
            })
            .collect();

        for &id in chunk_ids {
            if let Some(doc) = doc_map.get(&id) {
                if let Some(loc) = extract_custom_emoji_location(doc) {
                    match download_single_custom_reaction(adapter, storage_layout, &loc).await {
                        Ok(CustomReactionDownloadStatus::Downloaded(bytes)) => {
                            summary.downloaded += 1;
                            summary.total_bytes += bytes as u64;
                            progress.downloaded += 1;
                        }
                        Ok(CustomReactionDownloadStatus::AlreadyExists) => {
                            summary.already_existed += 1;
                            progress.already_existed += 1;
                        }
                        Ok(CustomReactionDownloadStatus::Unavailable) => {
                            summary.unavailable += 1;
                        }
                        Ok(CustomReactionDownloadStatus::Failed) | Err(_) => {
                            summary.failed += 1;
                            progress.failed += 1;
                        }
                    }
                } else {
                    debug!("Custom emoji document {id} missing valid visual download location");
                    summary.unavailable += 1;
                }
            } else {
                debug!("Custom emoji document {id} not found on server");
                summary.unavailable += 1;
            }

            progress.processed += 1;
            progress_cb(&progress);
        }
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaction_sync_extracts_static_webp_location() {
        let doc = tl::enums::Document::Document(tl::types::Document {
            id: 5256103272296499934,
            access_hash: 123456789,
            file_reference: vec![1, 2, 3],
            date: 1700000000,
            mime_type: "image/webp".to_string(),
            size: 54321,
            thumbs: None,
            video_thumbs: None,
            dc_id: 2,
            attributes: vec![],
        });

        let loc = extract_custom_emoji_location(&doc).unwrap();
        assert_eq!(loc.document_id, 5256103272296499934);
        assert_eq!(loc.dc_id, 2);
        assert!(!loc.location_tl.is_empty());
        assert!(loc.embedded_bytes.is_none());
    }

    #[test]
    fn reaction_sync_extracts_animated_tgs_static_thumbnail() {
        let doc = tl::enums::Document::Document(tl::types::Document {
            id: 8888888888888888888,
            access_hash: 987654321,
            file_reference: vec![4, 5, 6],
            date: 1700000000,
            mime_type: "application/x-tgsticker".to_string(),
            size: 12345,
            thumbs: Some(vec![tl::enums::PhotoSize::Size(tl::types::PhotoSize {
                r#type: "m".to_string(),
                w: 100,
                h: 100,
                size: 2048,
            })]),
            video_thumbs: None,
            dc_id: 4,
            attributes: vec![],
        });

        let loc = extract_custom_emoji_location(&doc).unwrap();
        assert_eq!(loc.document_id, 8888888888888888888);
        assert_eq!(loc.dc_id, 4);
        assert!(!loc.location_tl.is_empty());
        assert!(loc.embedded_bytes.is_none());
    }

    #[test]
    fn reaction_sync_extracts_cached_stripped_thumbnail() {
        let doc = tl::enums::Document::Document(tl::types::Document {
            id: 7777777777777777777,
            access_hash: 555555555,
            file_reference: vec![7, 8, 9],
            date: 1700000000,
            mime_type: "video/webm".to_string(),
            size: 67890,
            thumbs: Some(vec![tl::enums::PhotoSize::PhotoCachedSize(
                tl::types::PhotoCachedSize {
                    r#type: "s".to_string(),
                    w: 50,
                    h: 50,
                    bytes: vec![0xFF, 0xD8, 0xFF, 0xE0],
                },
            )]),
            video_thumbs: None,
            dc_id: 1,
            attributes: vec![],
        });

        let loc = extract_custom_emoji_location(&doc).unwrap();
        assert_eq!(loc.document_id, 7777777777777777777);
        assert_eq!(loc.embedded_bytes, Some(vec![0xFF, 0xD8, 0xFF, 0xE0]));
    }
}
