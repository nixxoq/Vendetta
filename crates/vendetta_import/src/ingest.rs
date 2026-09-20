use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

use sha2::{Digest, Sha256};
use tracing::info;
use vendetta_core::encode_hex;
use vendetta_media::StorageLayoutManager;
use vendetta_model::{
    FilterReason, MediaDownloadStatus, MediaRecord, MediaRole, MediaVerificationStatus, MessageId,
    MessageKey, MessageMediaJoin, MessageReplyRecord, PeerId, PeerRecord, PeerType,
    ReplyResolutionStatus,
};
use vendetta_storage::ArchiveDb;

use crate::{
    error::ImportResult,
    model::{ImportChat, ImportMediaItem},
};

#[derive(Debug, Clone, Default)]
pub struct IngestSummary {
    pub chats_count: usize,
    pub messages_count: usize,
    pub media_copied_count: usize,
    pub media_skipped_count: usize,
}

pub fn ingest_chats_into_db(
    db: &ArchiveDb,
    media_dir: &Path,
    chats: &[ImportChat],
) -> ImportResult<IngestSummary> {
    let mut summary = IngestSummary::default();
    let layout = StorageLayoutManager::new(media_dir);
    layout.ensure_dirs()?;

    for chat in chats {
        summary.chats_count += 1;

        // 1. Ingest chat peer
        db.upsert_peer(&chat.to_peer_record())?;

        // 2. Ingest senders
        let mut seen_senders = HashSet::new();
        for msg in &chat.messages {
            if let Some(s_id) = msg.sender_id
                && seen_senders.insert(s_id)
            {
                let sender_record = PeerRecord {
                    peer_id: s_id,
                    peer_type: PeerType::User,
                    name: msg.sender_name.clone(),
                    username: None,
                    phone: None,
                    raw_tl: None,
                    updated_at: msg.date,
                };
                db.upsert_peer(&sender_record)?;
            }
        }

        // 3. Ingest messages in batches
        let existing_ids: HashSet<_> = chat.messages.iter().map(|m| m.message_id).collect();
        let records: Vec<_> = chat
            .messages
            .iter()
            .map(|m| m.to_message_record(chat.peer_id))
            .collect();

        db.insert_messages_batch(&records)?;
        summary.messages_count += records.len();

        // 4. Ingest media & links
        for msg in &chat.messages {
            for (pos, media_item) in msg.media.iter().enumerate() {
                if media_item.is_skipped {
                    let media_id = format!(
                        "tdesktop_skipped_{}_{}_{}",
                        chat.peer_id.raw(),
                        msg.message_id.raw(),
                        pos
                    );
                    let rec = skipped_media_record(
                        media_id,
                        media_item,
                        msg.date,
                        "Not included in export: exceeds maximum size or missing on disk",
                        media_item.skip_reason.unwrap_or(FilterReason::SizeAboveMax),
                    );
                    link_media_record(
                        db,
                        chat.peer_id,
                        msg.message_id,
                        &rec,
                        media_item.role,
                        pos,
                    )?;
                    summary.media_skipped_count += 1;
                    continue;
                }

                if let Some(ref src_path) = media_item.source_path {
                    if src_path.is_file() {
                        let (sha256, size) = compute_file_sha256_and_size(src_path)?;
                        let rel_path = StorageLayoutManager::content_addressed_rel_path(
                            &sha256,
                            media_item.file_name.as_deref(),
                        );
                        let dest_path = layout.resolve_canonical_path(&rel_path);

                        if let Some(parent) = dest_path.parent() {
                            fs::create_dir_all(parent)?;
                        }

                        if !dest_path.is_file() {
                            fs::copy(src_path, &dest_path)?;
                            summary.media_copied_count += 1;
                        }

                        let media_id = format!("tdesktop_{sha256}");
                        let rec = MediaRecord {
                            media_id,
                            kind: media_item.kind,
                            mime_type: media_item.mime_type.clone(),
                            size_bytes: Some(size),
                            file_name: media_item.file_name.clone(),
                            size_type: None,
                            width: None,
                            height: None,
                            dc_id: 0,
                            source_location_tl: None,
                            file_reference: None,
                            local_rel_path: Some(rel_path),
                            sha256: Some(sha256),
                            download_status: MediaDownloadStatus::Completed,
                            downloaded_bytes: size,
                            chunk_size: 524288,
                            retry_count: 0,
                            max_retries: 5,
                            next_retry_at: None,
                            claimed_at: None,
                            worker_id: None,
                            last_error: None,
                            filter_decision: Some(vendetta_model::FilterDecision::Allow),
                            filter_reason: None,
                            policy_version: 1,
                            verification_status: MediaVerificationStatus::Verified,
                            created_at: msg.date,
                            updated_at: msg.date,
                        };
                        link_media_record(
                            db,
                            chat.peer_id,
                            msg.message_id,
                            &rec,
                            media_item.role,
                            pos,
                        )?;
                    } else {
                        // File path referenced in export does not exist on disk
                        let media_id = format!(
                            "tdesktop_missing_{}_{}_{}",
                            chat.peer_id.raw(),
                            msg.message_id.raw(),
                            pos
                        );
                        let rec = skipped_media_record(
                            media_id,
                            media_item,
                            msg.date,
                            "File referenced in export is missing on disk",
                            FilterReason::Manual,
                        );
                        link_media_record(
                            db,
                            chat.peer_id,
                            msg.message_id,
                            &rec,
                            media_item.role,
                            pos,
                        )?;
                        summary.media_skipped_count += 1;
                    }
                }
            }
        }

        // 5. Ingest replies
        for msg in &chat.messages {
            if let Some(target_id) = msg.reply_to_message_id {
                let status = if existing_ids.contains(&target_id) {
                    ReplyResolutionStatus::Resolved
                } else {
                    ReplyResolutionStatus::Missing
                };

                let reply_record = MessageReplyRecord {
                    source: MessageKey::new(chat.peer_id, msg.message_id),
                    target: MessageKey::new(chat.peer_id, target_id),
                    top_message_id: None,
                    resolution_status: status,
                };
                db.upsert_reply(&reply_record)?;
            }
        }
    }

    info!(
        "Ingestion completed: {} chats, {} messages, {} media copied, {} media skipped",
        summary.chats_count,
        summary.messages_count,
        summary.media_copied_count,
        summary.media_skipped_count
    );

    Ok(summary)
}

fn skipped_media_record(
    media_id: String,
    media_item: &ImportMediaItem,
    created_at: i64,
    last_error: &'static str,
    skip_reason: FilterReason,
) -> MediaRecord {
    MediaRecord {
        media_id,
        kind: media_item.kind,
        mime_type: media_item.mime_type.clone(),
        size_bytes: media_item.size_bytes,
        file_name: media_item.file_name.clone(),
        size_type: None,
        width: None,
        height: None,
        dc_id: 0,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Skipped,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: Some(last_error.to_string()),
        filter_decision: Some(vendetta_model::FilterDecision::Skip),
        filter_reason: Some(skip_reason),
        policy_version: 1,
        verification_status: MediaVerificationStatus::MissingFile,
        created_at,
        updated_at: created_at,
    }
}

fn link_media_record(
    db: &ArchiveDb,
    chat_peer_id: PeerId,
    msg_id: MessageId,
    rec: &MediaRecord,
    role: MediaRole,
    position: usize,
) -> ImportResult<()> {
    db.insert_or_update_media(rec)?;
    let join = MessageMediaJoin {
        key: MessageKey::new(chat_peer_id, msg_id),
        media_id: rec.media_id.clone(),
        role,
        position: position as i32,
    };
    db.link_message_media(&join)?;
    Ok(())
}

fn compute_file_sha256_and_size(path: &Path) -> io::Result<(String, i64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    let mut total_bytes = 0i64;

    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total_bytes += n as i64;
    }

    Ok((encode_hex(&hasher.finalize()), total_bytes))
}
