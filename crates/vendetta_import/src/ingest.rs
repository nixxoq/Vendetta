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

struct MediaIngestContext<'a> {
    db: &'a ArchiveDb,
    layout: &'a StorageLayoutManager,
    chat_peer_id: PeerId,
    message_id: MessageId,
    message_date: i64,
}

pub fn ingest_chats(
    db: &ArchiveDb,
    media_dir: &Path,
    chats: &[ImportChat],
) -> ImportResult<IngestSummary> {
    let layout = StorageLayoutManager::new(media_dir);
    layout.ensure_dirs()?;

    let mut summary = IngestSummary::default();

    for chat in chats {
        ingest_chat(db, &layout, chat, &mut summary)?;
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

fn ingest_chat(
    db: &ArchiveDb,
    layout: &StorageLayoutManager,
    chat: &ImportChat,
    summary: &mut IngestSummary,
) -> ImportResult<()> {
    db.upsert_peer(&chat.to_peer_record())?;

    ingest_senders(db, chat)?;
    ingest_messages(db, chat, summary)?;
    ingest_media(db, layout, chat, summary)?;
    ingest_replies(db, chat)?;

    summary.chats_count += 1;

    Ok(())
}

fn ingest_senders(db: &ArchiveDb, chat: &ImportChat) -> ImportResult<()> {
    let mut seen_senders = HashSet::new();

    for msg in &chat.messages {
        let Some(sender_id) = msg.sender_id else {
            continue;
        };

        if seen_senders.insert(sender_id) {
            db.upsert_peer(&PeerRecord {
                peer_id: sender_id,
                peer_type: PeerType::User,
                name: msg.sender_name.clone(),
                username: None,
                phone: None,
                raw_tl: None,
                updated_at: msg.date,
            })?;
        }
    }

    Ok(())
}

fn ingest_messages(
    db: &ArchiveDb,
    chat: &ImportChat,
    summary: &mut IngestSummary,
) -> ImportResult<()> {
    let records = chat
        .messages
        .iter()
        .map(|message| message.to_message_record(chat.peer_id))
        .collect::<Vec<_>>();

    db.insert_messages_batch(&records)?;
    summary.messages_count += records.len();

    Ok(())
}

fn ingest_media(
    db: &ArchiveDb,
    layout: &StorageLayoutManager,
    chat: &ImportChat,
    summary: &mut IngestSummary,
) -> ImportResult<()> {
    for message in &chat.messages {
        let ctx = MediaIngestContext {
            db,
            layout,
            chat_peer_id: chat.peer_id,
            message_id: message.message_id,
            message_date: message.date,
        };

        for (position, media_item) in message.media.iter().enumerate() {
            ingest_media_item(&ctx, position, media_item, summary)?;
        }
    }

    Ok(())
}

fn ingest_media_item(
    ctx: &MediaIngestContext<'_>,
    position: usize,
    media_item: &ImportMediaItem,
    summary: &mut IngestSummary,
) -> ImportResult<()> {
    if media_item.is_skipped {
        let media_id = format!(
            "tdesktop_skipped_{}_{}_{}",
            ctx.chat_peer_id.raw(),
            ctx.message_id.raw(),
            position
        );

        let record = skipped_media_record(
            media_id,
            media_item,
            ctx.message_date,
            "Not included in export: exceeds maximum size or missing on disk",
            media_item.skip_reason.unwrap_or(FilterReason::SizeAboveMax),
        );

        link_media_record(ctx, &record, media_item.role, position)?;
        summary.media_skipped_count += 1;

        return Ok(());
    }

    let Some(source_path) = media_item.source_path.as_deref() else {
        return Ok(());
    };

    if !source_path.is_file() {
        let media_id = format!(
            "tdesktop_missing_{}_{}_{}",
            ctx.chat_peer_id.raw(),
            ctx.message_id.raw(),
            position
        );

        let record = skipped_media_record(
            media_id,
            media_item,
            ctx.message_date,
            "File referenced in export is missing on disk",
            FilterReason::Manual,
        );

        link_media_record(ctx, &record, media_item.role, position)?;
        summary.media_skipped_count += 1;

        return Ok(());
    }

    let (sha256, size) = compute_file(source_path)?;
    let rel_path =
        StorageLayoutManager::content_addressed_rel_path(&sha256, media_item.file_name.as_deref());
    let dest_path = ctx.layout.resolve_canonical_path(&rel_path);

    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)?;
    }

    if !dest_path.is_file() {
        fs::copy(source_path, &dest_path)?;
        summary.media_copied_count += 1;
    }

    let record = MediaRecord {
        media_id: format!("tdesktop_{sha256}"),
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
        created_at: ctx.message_date,
        updated_at: ctx.message_date,
    };

    link_media_record(ctx, &record, media_item.role, position)
}

fn ingest_replies(db: &ArchiveDb, chat: &ImportChat) -> ImportResult<()> {
    let existing_ids = chat
        .messages
        .iter()
        .map(|message| message.message_id)
        .collect::<HashSet<_>>();

    for message in &chat.messages {
        let Some(target_id) = message.reply_to_message_id else {
            continue;
        };

        let reply_record = MessageReplyRecord {
            source: MessageKey::new(chat.peer_id, message.message_id),
            target: MessageKey::new(chat.peer_id, target_id),
            top_message_id: None,
            resolution_status: if existing_ids.contains(&target_id) {
                ReplyResolutionStatus::Resolved
            } else {
                ReplyResolutionStatus::Missing
            },
        };

        db.upsert_reply(&reply_record)?;
    }

    Ok(())
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
        last_error: Some(last_error.to_owned()),
        filter_decision: Some(vendetta_model::FilterDecision::Skip),
        filter_reason: Some(skip_reason),
        policy_version: 1,
        verification_status: MediaVerificationStatus::MissingFile,
        created_at,
        updated_at: created_at,
    }
}

fn link_media_record(
    ctx: &MediaIngestContext<'_>,
    record: &MediaRecord,
    role: MediaRole,
    position: usize,
) -> ImportResult<()> {
    ctx.db.insert_or_update_media(record)?;

    ctx.db.link_message_media(&MessageMediaJoin {
        key: MessageKey::new(ctx.chat_peer_id, ctx.message_id),
        media_id: record.media_id.clone(),
        role,
        position: position as i32,
    })?;

    Ok(())
}

fn compute_file(path: &Path) -> io::Result<(String, i64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65_536];
    let mut total_bytes = 0i64;

    loop {
        let bytes_read = file.read(&mut buffer)?;

        if bytes_read == 0 {
            break;
        }

        hasher.update(&buffer[..bytes_read]);
        total_bytes += bytes_read as i64;
    }

    Ok((encode_hex(&hasher.finalize()), total_bytes))
}
