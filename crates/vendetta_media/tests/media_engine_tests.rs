use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    sync::Arc,
};
use tempfile::{TempDir, tempdir};

use grammers_tl_types::{self as tl, Serializable};
use sha2::{Digest, Sha256};
use vendetta_core::decode_hex;
use vendetta_media::{
    ChunkPlanner, DynamicConcurrencyController, FileReferenceRefresher, MediaEngine,
    MediaFilterPolicy, MediaPolicyEvaluator, SingleMediaDownloader, StorageLayoutManager,
};
use vendetta_model::{
    FileRangeHash, FilterDecision, FilterReason, MediaDownloadStatus, MediaKind, MediaRecord,
    MediaRole, MediaVerificationStatus, MessageId, MessageKey, MessageMediaJoin, MessageRecord,
    MessageState, PeerId, PeerRecord, PeerType,
};
use vendetta_storage::ArchiveDb;
use vendetta_tg_adapter::{FakeTelegramAdapter, TelegramAdapter, extract_media_records};

fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn make_dummy_message(
    id: i32,
    peer_user_id: i64,
    media: Option<tl::enums::MessageMedia>,
) -> tl::enums::Message {
    tl::enums::Message::Message(tl::types::Message {
        out: false,
        mentioned: false,
        media_unread: false,
        silent: false,
        post: false,
        from_scheduled: false,
        legacy: false,
        edit_hide: false,
        pinned: false,
        noforwards: false,
        invert_media: false,
        offline: false,
        video_processing_pending: false,
        paid_suggested_post_stars: false,
        paid_suggested_post_ton: false,
        id,
        from_id: Some(tl::enums::Peer::User(tl::types::PeerUser {
            user_id: peer_user_id,
        })),
        from_boosts_applied: None,
        from_rank: None,
        peer_id: tl::enums::Peer::User(tl::types::PeerUser {
            user_id: peer_user_id,
        }),
        saved_peer_id: None,
        fwd_from: None,
        via_bot_id: None,
        via_business_bot_id: None,
        guestchat_via_from: None,
        reply_to: None,
        date: 1700000000,
        message: "Test message".to_string(),
        media,
        reply_markup: None,
        entities: None,
        views: None,
        forwards: None,
        replies: None,
        edit_date: None,
        post_author: None,
        grouped_id: None,
        reactions: None,
        restriction_reason: None,
        ttl_period: None,
        quick_reply_shortcut_id: None,
        effect: None,
        factcheck: None,
        report_delivery_until_date: None,
        paid_message_stars: None,
        suggested_post: None,
        schedule_repeat_period: None,
        summary_from_language: None,
        rich_message: None,
    })
}

#[test]
fn chunk_planner_clamps_to_1mb_fragment_boundaries() {
    const CHUNK: i32 = 524_288;
    const FRAGMENT: i64 = 1024 * 1024;

    assert_eq!(ChunkPlanner::plan_next_chunk(0, CHUNK).unwrap(), 524_288);
    assert_eq!(
        ChunkPlanner::plan_next_chunk(524_288, CHUNK).unwrap(),
        524_288
    );
    assert_eq!(
        ChunkPlanner::plan_next_chunk(FRAGMENT - 4096, CHUNK).unwrap(),
        4096
    );
    assert_eq!(
        ChunkPlanner::plan_next_chunk(FRAGMENT - 1024, CHUNK).unwrap(),
        1024
    );
    assert_eq!(
        ChunkPlanner::plan_next_chunk(FRAGMENT, CHUNK).unwrap(),
        524_288
    );
    assert_eq!(
        ChunkPlanner::plan_next_chunk(2 * FRAGMENT - 4096, CHUNK).unwrap(),
        4096
    );
}

#[test]
fn chunk_planner_rejects_unaligned_and_negative_offsets() {
    const CHUNK: i32 = 524_288;
    const FRAGMENT: i64 = 1024 * 1024;

    assert!(matches!(
        ChunkPlanner::plan_next_chunk(100, CHUNK),
        Err(vendetta_media::ChunkPlannerError::UnalignedOffset(100))
    ));

    assert!(matches!(
        ChunkPlanner::plan_next_chunk(FRAGMENT - 500, CHUNK),
        Err(vendetta_media::ChunkPlannerError::UnalignedOffset(off)) if off == FRAGMENT - 500
    ));

    assert!(matches!(
        ChunkPlanner::plan_next_chunk(-1024, CHUNK),
        Err(vendetta_media::ChunkPlannerError::NegativeOffset(-1024))
    ));
}

#[test]
fn media_metadata_normalizes_and_selects_optimal_photo_size() {
    let photo = tl::types::Photo {
        has_stickers: false,
        id: 99887766,
        access_hash: 1122334455,
        file_reference: vec![1, 2, 3, 4],
        date: 1700000000,
        sizes: vec![
            tl::enums::PhotoSize::PhotoStrippedSize(tl::types::PhotoStrippedSize {
                r#type: "i".to_string(),
                bytes: vec![1, 2, 3],
            }),
            tl::enums::PhotoSize::Size(tl::types::PhotoSize {
                r#type: "m".to_string(),
                w: 320,
                h: 240,
                size: 15000,
            }),
            tl::enums::PhotoSize::Progressive(tl::types::PhotoSizeProgressive {
                r#type: "y".to_string(),
                w: 1280,
                h: 720,
                sizes: vec![10000, 30000, 85000],
            }),
        ],
        video_sizes: None,
        dc_id: 2,
    };

    let photo_media = tl::enums::MessageMedia::Photo(tl::types::MessageMediaPhoto {
        live_photo: false,
        spoiler: false,
        photo: Some(tl::enums::Photo::Photo(photo)),
        video: None,
        ttl_seconds: None,
    });

    let msg = make_dummy_message(42, 1001, Some(photo_media));

    let extracted = extract_media_records(&msg, Some(PeerId::new(1001)));
    assert_eq!(extracted.len(), 1);

    let (record, join) = &extracted[0];
    assert_eq!(record.media_id, "photo_99887766_y");
    assert_eq!(record.kind, MediaKind::Photo);
    assert_eq!(record.width, Some(1280));
    assert_eq!(record.height, Some(720));
    assert_eq!(record.size_bytes, Some(85000));
    assert_eq!(record.dc_id, 2);
    assert_eq!(record.download_status, MediaDownloadStatus::Pending);

    assert_eq!(join.key.peer_id, PeerId::new(1001));
    assert_eq!(join.key.message_id, MessageId::new(42));
    assert_eq!(join.media_id, "photo_99887766_y");
}

#[test]
fn media_policy_filters_by_type_and_size() {
    let policy = MediaFilterPolicy {
        allow_videos: false,
        max_size_bytes: Some(1_000_000),
        ..Default::default()
    };

    let video_rec = MediaRecord {
        media_id: "doc_1".to_string(),
        kind: MediaKind::Video,
        mime_type: Some("video/mp4".to_string()),
        size_bytes: Some(500_000),
        file_name: Some("video.mp4".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: None,
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 0,
        updated_at: 0,
    };

    let (dec, reason) = MediaPolicyEvaluator::evaluate(&policy, &video_rec, None);
    assert_eq!(dec, FilterDecision::Skip);
    assert_eq!(reason, Some(FilterReason::TypeExcluded));

    let mut doc_rec = video_rec.clone();
    doc_rec.media_id = "doc_2".to_string();
    doc_rec.kind = MediaKind::Document;
    doc_rec.size_bytes = Some(5_000_000);

    let (dec2, reason2) = MediaPolicyEvaluator::evaluate(&policy, &doc_rec, None);
    assert_eq!(dec2, FilterDecision::Skip);
    assert_eq!(reason2, Some(FilterReason::SizeAboveMax));

    doc_rec.size_bytes = Some(500_000);
    let (dec3, reason3) = MediaPolicyEvaluator::evaluate(&policy, &doc_rec, None);
    assert_eq!(dec3, FilterDecision::Allow);
    assert_eq!(reason3, None);
}

#[test]
fn media_backfill_from_existing_messages_is_idempotent() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let doc = tl::types::Document {
        id: 778899,
        access_hash: 554433,
        file_reference: vec![9, 8, 7],
        date: 1700000000,
        mime_type: "application/pdf".to_string(),
        size: 2048,
        thumbs: None,
        video_thumbs: None,
        dc_id: 2,
        attributes: vec![tl::enums::DocumentAttribute::Filename(
            tl::types::DocumentAttributeFilename {
                file_name: "test.pdf".to_string(),
            },
        )],
    };

    let doc_media = tl::enums::MessageMedia::Document(tl::types::MessageMediaDocument {
        nopremium: false,
        spoiler: false,
        video: false,
        round: false,
        voice: false,
        video_cover: None,
        video_timestamp: None,
        document: Some(tl::enums::Document::Document(doc)),
        alt_documents: None,
        ttl_seconds: None,
    });

    let msg = make_dummy_message(10, 500, Some(doc_media));

    let msg_rec = MessageRecord {
        key: MessageKey::new(PeerId::new(500), MessageId::new(10)),
        date: 1700000000,
        sender_id: Some(PeerId::new(500)),
        text: Some("Attached file".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: None,
        views: None,
        forwards_count: None,
        raw_tl: Some(msg.to_bytes()),
    };

    db.insert_or_update_message(&msg_rec)
        .expect("insert msg failed");

    let engine = MediaEngine::new(Arc::clone(&db), adapter, temp_dir.path(), 1, 4, 4, 2);
    let policy = MediaFilterPolicy::default();

    let r1 = engine
        .plan_media_from_archive(&policy)
        .expect("pass 1 failed");
    assert_eq!(r1.messages_scanned, 1);
    assert_eq!(r1.media_discovered, 1);
    assert_eq!(r1.media_eligible, 1);

    let med = db
        .get_media("doc_778899")
        .expect("query failed")
        .expect("missing");
    assert_eq!(med.file_name.as_deref(), Some("test.pdf"));
    assert_eq!(med.download_status, MediaDownloadStatus::Pending);

    let r2 = engine
        .plan_media_from_archive(&policy)
        .expect("pass 2 failed");
    assert_eq!(r2.messages_scanned, 1);
    assert_eq!(r2.media_discovered, 1);

    let stats = engine.get_stats().expect("stats failed");
    assert_eq!(stats.total_count, 1);
    assert_eq!(stats.pending_count, 1);
}

#[tokio::test]
async fn media_downloader_resumes_partial_chunk_after_crash() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let layout = StorageLayoutManager::new(temp_dir.path());
    layout.ensure_dirs().expect("ensure dirs failed");

    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let total_bytes: Vec<u8> = (0..(1500 * 1024)).map(|i| (i % 251) as u8).collect();
    let expected_hash = sha256_hex(&total_bytes);

    let location_tl = vec![0x11, 0x22, 0x33, 0x44];
    fake_adapter.add_file(location_tl.clone(), total_bytes.clone());

    let mut record = MediaRecord {
        media_id: "doc_resumable".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(total_bytes.len() as i64),
        file_name: Some("data.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(location_tl.clone()),
        file_reference: Some(vec![1, 2, 3]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };

    db.insert_or_update_media(&record).expect("insert failed");

    let part_path = layout.temp_part_path("doc_resumable");
    {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&part_path)
            .expect("create part file failed");
        file.write_all(&total_bytes[..600_000])
            .expect("write partial failed");
    }

    record.downloaded_bytes = 524_288;
    db.update_media_progress("doc_resumable", 524_288)
        .expect("update progress failed");

    let downloader = SingleMediaDownloader::new(Arc::clone(&db), fake_adapter, layout.clone());

    let final_hash = downloader
        .download_item(&mut record)
        .await
        .expect("download failed");

    assert_eq!(final_hash, expected_hash);

    let rel_path = format!("media/{}/{expected_hash}.bin", &expected_hash[..2]);
    let abs_path = layout.resolve_path(&rel_path);
    assert!(abs_path.exists());
    assert_eq!(
        fs::metadata(&abs_path).unwrap().len() as usize,
        total_bytes.len()
    );
    assert!(!part_path.exists());
}

#[tokio::test]
async fn chunk_range_hash_verifies_contiguous_sub_ranges() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let layout = StorageLayoutManager::new(temp_dir.path());
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let payload: Vec<u8> = (0..524288).map(|i| (i % 256) as u8).collect();
    let location_tl = vec![0x99, 0x88, 0x77];
    fake_adapter.add_file(location_tl.clone(), payload.clone());

    let mut range_hashes = Vec::new();
    for i in 0..4 {
        let offset = i * 131072;
        let limit = 131072;
        let h = sha256_hex(&payload[offset as usize..(offset + limit) as usize]);
        range_hashes.push(FileRangeHash {
            offset,
            limit: limit as i32,
            hash: decode_hex(&h).unwrap(),
        });
    }
    fake_adapter.add_file_hashes(location_tl.clone(), range_hashes);

    let mut record = MediaRecord {
        media_id: "doc_hashes_multi".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(payload.len() as i64),
        file_name: Some("test.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(location_tl),
        file_reference: Some(vec![1]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&record).expect("insert failed");

    let downloader = SingleMediaDownloader::new(Arc::clone(&db), fake_adapter, layout);
    let res = downloader.download_item(&mut record).await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn chunk_range_hash_gap_falls_back_to_whole_file_verification() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let layout = StorageLayoutManager::new(temp_dir.path());
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let payload: Vec<u8> = (0..524288).map(|i| (i % 256) as u8).collect();
    let location_tl = vec![0x99, 0x88, 0x66];
    fake_adapter.add_file(location_tl.clone(), payload.clone());

    let range_hashes = vec![
        FileRangeHash {
            offset: 0,
            limit: 131072,
            hash: decode_hex(&sha256_hex(&payload[0..131072])).unwrap(),
        },
        FileRangeHash {
            offset: 262144,
            limit: 131072,
            hash: decode_hex(&sha256_hex(&payload[262144..393216])).unwrap(),
        },
    ];
    fake_adapter.add_file_hashes(location_tl.clone(), range_hashes);

    let mut record = MediaRecord {
        media_id: "doc_hashes_gap".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(payload.len() as i64),
        file_name: Some("test_gap.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(location_tl),
        file_reference: Some(vec![1]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&record).expect("insert failed");

    let downloader = SingleMediaDownloader::new(Arc::clone(&db), fake_adapter, layout);
    let res = downloader.download_item(&mut record).await;
    assert!(res.is_ok());
}

#[tokio::test]
async fn expired_file_reference_refreshes_and_resumes_download() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let payload: Vec<u8> = vec![42u8; 100_000];
    let location_tl = tl::enums::InputFileLocation::InputDocumentFileLocation(
        tl::types::InputDocumentFileLocation {
            id: 12345,
            access_hash: 67890,
            file_reference: vec![1, 1, 1],
            thumb_size: String::new(),
        },
    )
    .to_bytes();
    fake_adapter.add_file(location_tl.clone(), payload.clone());

    let refreshed_location_tl = tl::enums::InputFileLocation::InputDocumentFileLocation(
        tl::types::InputDocumentFileLocation {
            id: 12345,
            access_hash: 67890,
            file_reference: vec![9, 9, 9],
            thumb_size: String::new(),
        },
    )
    .to_bytes();
    fake_adapter.add_file(refreshed_location_tl, payload.clone());
    fake_adapter.inject_download_error("FILE_REFERENCE_EXPIRED");

    let record = MediaRecord {
        media_id: "doc_12345".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(payload.len() as i64),
        file_name: Some("data.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(location_tl),
        file_reference: Some(vec![1, 1, 1]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&record).expect("insert failed");

    let doc_tl = tl::types::Document {
        id: 12345,
        access_hash: 67890,
        file_reference: vec![9, 9, 9],
        date: 1700000000,
        mime_type: "application/octet-stream".to_string(),
        size: payload.len() as i64,
        thumbs: None,
        video_thumbs: None,
        dc_id: 2,
        attributes: vec![],
    };

    let doc_media = tl::enums::MessageMedia::Document(tl::types::MessageMediaDocument {
        nopremium: false,
        spoiler: false,
        video: false,
        round: false,
        voice: false,
        video_cover: None,
        video_timestamp: None,
        document: Some(tl::enums::Document::Document(doc_tl)),
        alt_documents: None,
        ttl_seconds: None,
    });

    let msg_tl = make_dummy_message(77, 100, Some(doc_media));

    let msg_rec = MessageRecord {
        key: MessageKey::new(PeerId::new(100), MessageId::new(77)),
        date: 1700000000,
        sender_id: Some(PeerId::new(100)),
        text: None,
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: None,
        views: None,
        forwards_count: None,
        raw_tl: Some(msg_tl.to_bytes()),
    };

    db.insert_or_update_message(&msg_rec)
        .expect("insert msg failed");
    fake_adapter.add_peer(PeerRecord {
        peer_id: PeerId::new(100),
        peer_type: PeerType::User,
        name: Some("User 100".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    });
    fake_adapter.add_message(msg_rec.clone());

    let join = MessageMediaJoin {
        key: MessageKey::new(PeerId::new(100), MessageId::new(77)),
        media_id: "doc_12345".to_string(),
        role: MediaRole::Attachment,
        position: 0,
    };
    db.link_message_media(&join).expect("link failed");

    let engine = MediaEngine::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
        temp_dir.path(),
        1,
        2,
        4,
        1,
    );

    let completed = engine.download_all_pending("worker_test").await;
    assert_eq!(completed, 1);

    let fetched = db.get_media("doc_12345").expect("get failed").unwrap();
    assert_eq!(fetched.download_status, MediaDownloadStatus::Completed);
}

#[tokio::test]
async fn content_hash_deduplication_handles_concurrent_races() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let layout = StorageLayoutManager::new(temp_dir.path());
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let identical_bytes = vec![7u8; 50_000];
    let hash = sha256_hex(&identical_bytes);

    let loc1 = vec![1, 1, 1];
    let loc2 = vec![2, 2, 2];
    fake_adapter.add_file(loc1.clone(), identical_bytes.clone());
    fake_adapter.add_file(loc2.clone(), identical_bytes.clone());

    let mut r1 = MediaRecord {
        media_id: "doc_dup1".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(identical_bytes.len() as i64),
        file_name: Some("file1.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(loc1),
        file_reference: Some(vec![1]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };

    let mut r2 = r1.clone();
    r2.media_id = "doc_dup2".to_string();
    r2.file_name = Some("file2.bin".to_string());
    r2.source_location_tl = Some(loc2);

    db.insert_or_update_media(&r1).expect("insert r1 failed");
    db.insert_or_update_media(&r2).expect("insert r2 failed");

    let downloader = SingleMediaDownloader::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
        layout.clone(),
    );

    let h1 = downloader
        .download_item(&mut r1)
        .await
        .expect("dl 1 failed");
    assert_eq!(h1, hash);

    let h2 = downloader
        .download_item(&mut r2)
        .await
        .expect("dl 2 failed");
    assert_eq!(h2, hash);

    let m1 = db.get_media("doc_dup1").expect("get failed").unwrap();
    let m2 = db.get_media("doc_dup2").expect("get failed").unwrap();

    assert_eq!(m1.download_status, MediaDownloadStatus::Completed);
    assert_eq!(m2.download_status, MediaDownloadStatus::Completed);
    assert_eq!(m1.sha256, m2.sha256);
}

#[test]
fn media_engine_reconciles_startup_cases_a_through_e() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let layout = StorageLayoutManager::new(temp_dir.path());
    layout.ensure_dirs().expect("dirs failed");

    let engine = MediaEngine::new(
        Arc::clone(&db),
        Arc::new(FakeTelegramAdapter::new()),
        temp_dir.path(),
        1,
        2,
        4,
        1,
    );

    let r_a = MediaRecord {
        media_id: "doc_case_a".to_string(),
        kind: MediaKind::Document,
        mime_type: None,
        size_bytes: Some(1000),
        file_name: None,
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Downloading,
        downloaded_bytes: 400,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: Some(1700000000),
        worker_id: Some("dead_worker".to_string()),
        last_error: None,
        filter_decision: None,
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&r_a).expect("insert a failed");

    let part_a = layout.temp_part_path("doc_case_a");
    {
        let mut f = File::create(&part_a).unwrap();
        f.write_all(&vec![0u8; 800]).unwrap();
    }

    let orphan_path = layout.temp_part_path("doc_orphan_xyz");
    {
        let mut f = File::create(&orphan_path).unwrap();
        f.write_all(b"orphan data").unwrap();
    }

    let rep = engine.reconcile_startup().expect("reconciliation failed");

    assert_eq!(rep.downloading_reset_count, 1);
    assert_eq!(rep.orphan_part_cleaned_count, 1);

    let a_post = db.get_media("doc_case_a").unwrap().unwrap();
    assert_eq!(a_post.download_status, MediaDownloadStatus::Pending);
    assert_eq!(fs::metadata(&part_a).unwrap().len(), 400);
    assert!(!orphan_path.exists());
}

#[test]
fn concurrency_controller_scales_dynamically_with_dc_cooldowns() {
    let ctrl = DynamicConcurrencyController::new(1, 8, 4, 2);
    assert_eq!(ctrl.current_concurrency(), 2);

    for _ in 0..5 {
        ctrl.record_success();
    }
    assert_eq!(ctrl.current_concurrency(), 3);

    for _ in 0..5 {
        ctrl.record_success();
    }
    assert_eq!(ctrl.current_concurrency(), 4);

    ctrl.record_backoff();
    assert_eq!(ctrl.current_concurrency(), 2);

    assert!(!ctrl.is_dc_in_cooldown(2));
    ctrl.set_dc_cooldown(2, 60);
    assert!(ctrl.is_dc_in_cooldown(2));
    assert!(!ctrl.is_dc_in_cooldown(4));
}

#[tokio::test]
async fn flood_wait_applies_exponential_cooldown_and_resumes() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let payload: Vec<u8> = vec![1, 2, 3, 4, 5];
    let location_tl = vec![0x33, 0x44];
    fake_adapter.add_file(location_tl.clone(), payload.clone());
    fake_adapter.inject_download_error("FLOOD_PREMIUM_WAIT_2");

    let record = MediaRecord {
        media_id: "doc_flood".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(payload.len() as i64),
        file_name: Some("flood.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(location_tl),
        file_reference: Some(vec![1]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&record).expect("insert failed");

    let engine = MediaEngine::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
        temp_dir.path(),
        1,
        2,
        4,
        1,
    );

    let downloaded = engine.download_all_pending("worker_flood").await;
    assert_eq!(downloaded, 0);

    let med = db.get_media("doc_flood").unwrap().unwrap();
    assert_eq!(med.download_status, MediaDownloadStatus::RetryWait);
    assert!(med.next_retry_at.is_some());
    assert_eq!(med.retry_count, 1);
}

#[tokio::test]
async fn dc_migration_switches_dc_and_resumes_download() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let payload: Vec<u8> = vec![99u8; 1000];
    let location_tl = vec![0x55, 0x66];
    fake_adapter.add_file(location_tl.clone(), payload.clone());
    fake_adapter.inject_download_error("FILE_MIGRATE_4");

    let record = MediaRecord {
        media_id: "doc_migrate".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(payload.len() as i64),
        file_name: Some("migrated.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(location_tl),
        file_reference: Some(vec![1]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&record).expect("insert failed");

    let engine = MediaEngine::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
        temp_dir.path(),
        1,
        2,
        4,
        1,
    );

    let downloaded = engine.download_all_pending("worker_migrate").await;
    assert_eq!(downloaded, 1);

    let med = db.get_media("doc_migrate").unwrap().unwrap();
    assert_eq!(med.download_status, MediaDownloadStatus::Completed);
    assert_eq!(med.dc_id, 4);
}

#[test]
fn deleted_message_preserves_archived_media_link_and_binary() {
    let db = ArchiveDb::open_in_memory().expect("db failed");

    let msg = MessageRecord {
        key: MessageKey::new(PeerId::new(200), MessageId::new(5)),
        date: 1700000000,
        sender_id: Some(PeerId::new(200)),
        text: Some("Important attachment".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: None,
        views: None,
        forwards_count: None,
        raw_tl: None,
    };
    db.insert_or_update_message(&msg)
        .expect("insert msg failed");

    let media = MediaRecord {
        media_id: "doc_preserved".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/pdf".to_string()),
        size_bytes: Some(5000),
        file_name: Some("preserved.pdf".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some("media/ab/abcdef123.pdf".to_string()),
        sha256: Some("abcdef123".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 5000,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Verified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&media)
        .expect("insert media failed");

    let join = MessageMediaJoin {
        key: MessageKey::new(PeerId::new(200), MessageId::new(5)),
        media_id: "doc_preserved".to_string(),
        role: MediaRole::Attachment,
        position: 0,
    };
    db.link_message_media(&join).expect("link failed");

    db.mark_message_deleted(MessageKey::new(PeerId::new(200), MessageId::new(5)))
        .expect("mark deleted failed");

    let m = db
        .get_message(MessageKey::new(PeerId::new(200), MessageId::new(5)))
        .unwrap()
        .unwrap();
    assert_eq!(m.state, MessageState::Deleted);

    let med = db.get_media("doc_preserved").unwrap().unwrap();
    assert_eq!(med.download_status, MediaDownloadStatus::Completed);
    let refs = db
        .get_referencing_messages_for_media("doc_preserved")
        .unwrap();
    assert_eq!(refs.len(), 1);
}

#[test]
fn media_verifier_detects_missing_and_corrupted_files() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let layout = StorageLayoutManager::new(temp_dir.path());
    layout.ensure_dirs().expect("dirs failed");

    let content = b"Valid content 12345";
    let hash = sha256_hex(content);
    let rel_valid = format!("media/{}/valid.bin", &hash[..2]);
    let abs_valid = layout.resolve_path(&rel_valid);
    if let Some(p) = abs_valid.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(&abs_valid, content).unwrap();

    let r_valid = MediaRecord {
        media_id: "doc_valid".to_string(),
        kind: MediaKind::Document,
        mime_type: None,
        size_bytes: Some(content.len() as i64),
        file_name: Some("valid.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(rel_valid),
        sha256: Some(hash.clone()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: content.len() as i64,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&r_valid).unwrap();

    let r_missing = MediaRecord {
        media_id: "doc_missing".to_string(),
        kind: MediaKind::Document,
        mime_type: None,
        size_bytes: Some(100),
        file_name: Some("missing.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some("media/00/nonexistent.bin".to_string()),
        sha256: Some("0011223344".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 100,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&r_missing).unwrap();

    let verifier = vendetta_media::MediaVerifier::new(Arc::clone(&db), layout);
    let report = verifier.verify_all_completed().unwrap();

    assert_eq!(report.total_checked, 2);
    assert_eq!(report.verified_count, 1);
    assert_eq!(report.missing_count, 1);

    let v_post = db.get_media("doc_valid").unwrap().unwrap();
    assert_eq!(
        v_post.verification_status,
        MediaVerificationStatus::Verified
    );

    let m_post = db.get_media("doc_missing").unwrap().unwrap();
    assert_eq!(
        m_post.verification_status,
        MediaVerificationStatus::MissingFile
    );
}

#[tokio::test]
async fn media_downloader_handles_unexpected_cdn_redirect() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let layout = StorageLayoutManager::new(temp_dir.path());
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    fake_adapter.inject_download_error("CDN_REDIRECT");

    let mut record = MediaRecord {
        media_id: "doc_cdn".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(5000),
        file_name: Some("cdn.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(vec![1, 2, 3]),
        file_reference: Some(vec![1]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&record).unwrap();

    let downloader = SingleMediaDownloader::new(Arc::clone(&db), fake_adapter, layout);

    let res = downloader.download_item(&mut record).await;
    assert!(res.is_err());
    match res.unwrap_err() {
        vendetta_media::MediaEngineError::Adapter(
            vendetta_tg_adapter::AdapterError::CdnRedirectUnsupported { dc_id, .. },
        ) => {
            assert_eq!(dc_id, 2);
        }
        other => panic!("Expected CdnRedirectUnsupported, got {other:?}"),
    }
}

#[tokio::test]
async fn large_media_streaming_detects_size_mismatch() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let layout = StorageLayoutManager::new(temp_dir.path());
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let data: Vec<u8> = vec![123u8; 2 * 1024 * 1024];
    let location_tl = vec![0x77, 0x88];
    fake_adapter.add_file(location_tl.clone(), data.clone());

    let mut record = MediaRecord {
        media_id: "doc_large".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(3 * 1024 * 1024),
        file_name: Some("large.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(location_tl),
        file_reference: Some(vec![1]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&record).unwrap();

    let downloader = SingleMediaDownloader::new(Arc::clone(&db), fake_adapter, layout);

    let res = downloader.download_item(&mut record).await;
    assert!(res.is_err());
    match res.unwrap_err() {
        vendetta_media::MediaEngineError::FinalSizeMismatch {
            expected, actual, ..
        } => {
            assert_eq!(expected, 3 * 1024 * 1024);
            assert_eq!(actual, 2 * 1024 * 1024);
        }
        other => panic!("Expected FinalSizeMismatch, got {other:?}"),
    }
}

#[test]
fn media_extraction_extracts_webpage_and_paid_media() {
    let photo = tl::types::Photo {
        has_stickers: false,
        id: 111222,
        access_hash: 333444,
        file_reference: vec![1, 2, 3],
        date: 1700000000,
        sizes: vec![tl::enums::PhotoSize::Size(tl::types::PhotoSize {
            r#type: "x".to_string(),
            w: 800,
            h: 600,
            size: 45000,
        })],
        video_sizes: None,
        dc_id: 2,
    };

    let doc = tl::types::Document {
        id: 555666,
        access_hash: 777888,
        file_reference: vec![4, 5, 6],
        date: 1700000000,
        mime_type: "video/mp4".to_string(),
        size: 1_200_000,
        thumbs: None,
        video_thumbs: None,
        dc_id: 2,
        attributes: vec![tl::enums::DocumentAttribute::Video(
            tl::types::DocumentAttributeVideo {
                round_message: false,
                supports_streaming: true,
                nosound: false,
                duration: 10.0,
                w: 1280,
                h: 720,
                preload_prefix_size: None,
                video_start_ts: None,
                video_codec: None,
            },
        )],
    };

    let webpage = tl::enums::WebPage::Page(tl::types::WebPage {
        has_large_media: true,
        video_cover_photo: false,
        id: 999,
        url: "https://example.com/article".to_string(),
        display_url: "example.com".to_string(),
        hash: 0,
        r#type: Some("article".to_string()),
        site_name: Some("Example".to_string()),
        title: Some("Example News".to_string()),
        description: Some("News preview".to_string()),
        photo: Some(tl::enums::Photo::Photo(photo)),
        embed_url: None,
        embed_type: None,
        embed_width: None,
        embed_height: None,
        duration: None,
        author: None,
        document: None,
        cached_page: None,
        attributes: None,
    });

    let wp_media = tl::enums::MessageMedia::WebPage(tl::types::MessageMediaWebPage {
        force_large_media: false,
        force_small_media: false,
        manual: false,
        safe: true,
        webpage,
    });

    let msg1 = make_dummy_message(101, 2001, Some(wp_media));
    let extracted1 = extract_media_records(&msg1, Some(PeerId::new(2001)));
    assert_eq!(extracted1.len(), 1);
    assert_eq!(extracted1[0].0.media_id, "photo_111222_x");

    let paid_media = tl::enums::MessageMedia::PaidMedia(tl::types::MessageMediaPaidMedia {
        stars_amount: 10,
        extended_media: vec![tl::enums::MessageExtendedMedia::Media(Box::new(
            tl::types::MessageExtendedMedia {
                media: tl::enums::MessageMedia::Document(tl::types::MessageMediaDocument {
                    nopremium: false,
                    spoiler: false,
                    video: true,
                    round: false,
                    voice: false,
                    video_cover: None,
                    video_timestamp: None,
                    document: Some(tl::enums::Document::Document(doc)),
                    alt_documents: None,
                    ttl_seconds: None,
                }),
            },
        ))],
    });

    let msg2 = make_dummy_message(102, 2001, Some(paid_media));
    let extracted2 = extract_media_records(&msg2, Some(PeerId::new(2001)));
    assert_eq!(extracted2.len(), 1);
    assert_eq!(extracted2[0].0.media_id, "doc_555666");
    assert_eq!(extracted2[0].0.kind, MediaKind::Video);
}

#[tokio::test]
async fn content_hash_dedup_replaces_corrupted_existing_file() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let layout = StorageLayoutManager::new(temp_dir.path());
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let payload = vec![42u8; 100_000];
    let expected_hash = sha256_hex(&payload);
    let location_tl = vec![0x11, 0x22];
    fake_adapter.add_file(location_tl.clone(), payload.clone());

    let rel_path =
        StorageLayoutManager::content_addressed_rel_path(&expected_hash, Some("file.bin"));
    let dest_path = layout.resolve_path(&rel_path);
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&dest_path, b"CORRUPTED_BYTES").unwrap();

    let mut record = MediaRecord {
        media_id: "doc_corrupt_dest".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(payload.len() as i64),
        file_name: Some("file.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(location_tl),
        file_reference: Some(vec![1]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&record).unwrap();

    let downloader = SingleMediaDownloader::new(Arc::clone(&db), fake_adapter, layout);

    let h = downloader
        .download_item(&mut record)
        .await
        .expect("download failed");
    assert_eq!(h, expected_hash);

    let dest_content = fs::read(&dest_path).unwrap();
    assert_eq!(dest_content, payload);
}

#[test]
fn media_backfill_preserves_completed_media_state() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let engine = MediaEngine::new(Arc::clone(&db), adapter, temp_dir.path(), 1, 4, 4, 2);

    let doc = tl::types::Document {
        id: 777,
        access_hash: 888,
        file_reference: vec![1, 2, 3],
        date: 1700000000,
        mime_type: "application/pdf".to_string(),
        size: 50_000,
        thumbs: None,
        video_thumbs: None,
        dc_id: 2,
        attributes: vec![],
    };
    let doc_media = tl::enums::MessageMedia::Document(tl::types::MessageMediaDocument {
        nopremium: false,
        spoiler: false,
        video: false,
        round: false,
        voice: false,
        video_cover: None,
        video_timestamp: None,
        document: Some(tl::enums::Document::Document(doc)),
        alt_documents: None,
        ttl_seconds: None,
    });
    let msg = make_dummy_message(10, 500, Some(doc_media));

    let msg_rec = MessageRecord {
        key: MessageKey::new(PeerId::new(500), MessageId::new(10)),
        date: 1700000000,
        sender_id: Some(PeerId::new(500)),
        text: None,
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: None,
        views: None,
        forwards_count: None,
        raw_tl: Some(msg.to_bytes()),
    };
    db.insert_or_update_message(&msg_rec).unwrap();

    let policy1 = MediaFilterPolicy::default();
    engine.plan_media_from_archive(&policy1).unwrap();

    db.update_media_completed("doc_777", "VALID_HASH_777", "media/va/valid.pdf")
        .unwrap();

    let before_second = db.get_media("doc_777").unwrap().unwrap();
    assert_eq!(
        before_second.download_status,
        MediaDownloadStatus::Completed
    );

    let policy2 = MediaFilterPolicy {
        allow_documents: false,
        ..Default::default()
    };
    engine.plan_media_from_archive(&policy2).unwrap();

    let after_second = db.get_media("doc_777").unwrap().unwrap();
    assert_eq!(
        after_second.download_status,
        MediaDownloadStatus::Completed,
        "Completed media must NEVER be reset by subsequent backfill"
    );
    assert_eq!(after_second.sha256.as_deref(), Some("VALID_HASH_777"));
}

#[tokio::test]
async fn file_hashes_fetch_propagates_flood_wait_error() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let payload = vec![0xAB; 2048];
    let loc_tl = vec![1, 2, 3, 4];
    fake_adapter.add_file(loc_tl.clone(), payload);
    fake_adapter.inject_file_hash_error("FLOOD_WAIT_15");

    let record = MediaRecord {
        media_id: "doc_hash_flood".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(2048),
        file_name: Some("test.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(loc_tl),
        file_reference: Some(vec![9, 9, 9]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 1024,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&record).expect("insert failed");

    let engine = MediaEngine::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
        temp_dir.path(),
        1,
        2,
        4,
        1,
    );

    let summary = engine.download_batch("worker_hash_flood").await;
    assert_eq!(summary.completed_count, 0);
    assert_eq!(summary.retry_wait_count, 1);

    let med = db.get_media("doc_hash_flood").unwrap().unwrap();
    assert_eq!(med.download_status, MediaDownloadStatus::RetryWait);
    assert_eq!(med.retry_count, 1);
    assert!(med.next_retry_at.is_some());
}

#[tokio::test]
async fn file_hashes_fetch_propagates_expired_reference_error() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let payload = vec![0xCD; 2048];
    let loc_tl = vec![5, 6, 7, 8];
    fake_adapter.add_file(loc_tl.clone(), payload);
    fake_adapter.inject_file_hash_error("FILE_REFERENCE_EXPIRED");

    let mut record = MediaRecord {
        media_id: "doc_hash_expired".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(2048),
        file_name: Some("test.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(loc_tl),
        file_reference: Some(vec![1, 2, 3]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 1024,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };

    let layout = StorageLayoutManager::new(temp_dir.path());
    let downloader = SingleMediaDownloader::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
        layout,
    );

    let err = downloader.download_item(&mut record).await.unwrap_err();
    match err {
        vendetta_media::MediaEngineError::Adapter(
            vendetta_tg_adapter::AdapterError::FileReferenceExpired,
        ) => {}
        other => panic!("Expected Adapter(FileReferenceExpired), got: {:?}", other),
    }
}

#[tokio::test]
async fn file_hashes_fetch_falls_back_on_unsupported_location() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let payload = vec![0xEE; 2048];
    let loc_tl = vec![9, 10, 11, 12];
    fake_adapter.add_file(loc_tl.clone(), payload.clone());
    fake_adapter.inject_file_hash_error("LOCATION_INVALID");

    let mut record = MediaRecord {
        media_id: "doc_hash_fallback".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(2048),
        file_name: Some("test.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(loc_tl),
        file_reference: Some(vec![1, 2, 3]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 1024,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };

    let layout = StorageLayoutManager::new(temp_dir.path());
    let downloader = SingleMediaDownloader::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
        layout,
    );

    let res = downloader.download_item(&mut record).await.unwrap();
    assert_eq!(res, sha256_hex(&payload));
}

#[tokio::test]
async fn multi_chunk_file_hash_verifies_per_chunk_window() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let total_size = 1_572_864;
    let mut payload = Vec::with_capacity(total_size);
    for i in 0..total_size {
        payload.push((i % 251) as u8);
    }

    let loc_tl = vec![13, 14, 15, 16];
    fake_adapter.add_file(loc_tl.clone(), payload.clone());

    let h0 = Sha256::digest(&payload[0..524_288]).to_vec();
    let h1 = Sha256::digest(&payload[524_288..1_048_576]).to_vec();
    let h2 = Sha256::digest(&payload[1_048_576..1_572_864]).to_vec();

    fake_adapter.add_file_hashes(
        loc_tl.clone(),
        vec![
            FileRangeHash {
                offset: 0,
                limit: 524_288,
                hash: h0,
            },
            FileRangeHash {
                offset: 524_288,
                limit: 524_288,
                hash: h1,
            },
            FileRangeHash {
                offset: 1_048_576,
                limit: 524_288,
                hash: h2,
            },
        ],
    );

    let mut record = MediaRecord {
        media_id: "doc_multi_window".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(total_size as i64),
        file_name: Some("multi.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(loc_tl),
        file_reference: Some(vec![1, 2, 3]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524_288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };

    let layout = StorageLayoutManager::new(temp_dir.path());
    let downloader = SingleMediaDownloader::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
        layout,
    );

    let res = downloader.download_item(&mut record).await.unwrap();
    assert_eq!(res, sha256_hex(&payload));
}

#[tokio::test]
async fn unaligned_persisted_progress_returns_corruption_error() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let mut record = MediaRecord {
        media_id: "doc_unaligned_progress".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(10_000),
        file_name: Some("test.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(vec![1, 2, 3]),
        file_reference: Some(vec![1, 2, 3]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Downloading,
        downloaded_bytes: 1_000,
        chunk_size: 1024,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };

    let layout = StorageLayoutManager::new(temp_dir.path());
    let downloader = SingleMediaDownloader::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
        layout,
    );

    let err = downloader.download_item(&mut record).await.unwrap_err();
    match err {
        vendetta_media::MediaEngineError::CorruptedProgress {
            downloaded_bytes, ..
        } => {
            assert_eq!(downloaded_bytes, 1000);
        }
        other => panic!("Expected CorruptedProgress, got: {:?}", other),
    }
}

#[tokio::test]
async fn disk_part_smaller_than_progress_returns_error() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let layout = StorageLayoutManager::new(temp_dir.path());
    layout.ensure_dirs().unwrap();

    let temp_path = layout.temp_part_path("doc_disk_truncated");
    fs::write(&temp_path, vec![0u8; 500]).unwrap();

    let mut record = MediaRecord {
        media_id: "doc_disk_truncated".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(10_000),
        file_name: Some("test.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(vec![1, 2, 3]),
        file_reference: Some(vec![1, 2, 3]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Downloading,
        downloaded_bytes: 2048,
        chunk_size: 1024,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };

    let downloader = SingleMediaDownloader::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
        layout,
    );

    let err = downloader.download_item(&mut record).await.unwrap_err();
    match err {
        vendetta_media::MediaEngineError::CorruptedProgress {
            downloaded_bytes, ..
        } => {
            assert_eq!(downloaded_bytes, 2048);
        }
        other => panic!("Expected CorruptedProgress, got: {:?}", other),
    }
}

#[tokio::test]
async fn short_chunk_size_mismatch_returns_error() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let loc_tl = vec![17, 18, 19, 20];
    fake_adapter.add_file(loc_tl.clone(), vec![0xAA; 500]);

    let mut record = MediaRecord {
        media_id: "doc_short_mismatch".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(1_000_000),
        file_name: Some("test.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(loc_tl),
        file_reference: Some(vec![1, 2, 3]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 524_288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };

    let layout = StorageLayoutManager::new(temp_dir.path());
    let downloader = SingleMediaDownloader::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
        layout,
    );

    let err = downloader.download_item(&mut record).await.unwrap_err();
    match err {
        vendetta_media::MediaEngineError::FinalSizeMismatch {
            expected, actual, ..
        } => {
            assert_eq!(expected, 1_000_000);
            assert_eq!(actual, 500);
        }
        other => panic!("Expected FinalSizeMismatch, got: {:?}", other),
    }
}

#[test]
fn requeue_skipped_media_reevaluates_policy_per_record() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let photo_rec = MediaRecord {
        media_id: "photo_skip_1".to_string(),
        kind: MediaKind::Photo,
        mime_type: Some("image/jpeg".to_string()),
        size_bytes: Some(5000),
        file_name: Some("photo.jpg".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(vec![1, 2]),
        file_reference: Some(vec![1, 2]),
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
        last_error: None,
        filter_decision: Some(FilterDecision::Skip),
        filter_reason: Some(FilterReason::TypeExcluded),
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };

    let video_rec = MediaRecord {
        media_id: "video_skip_2".to_string(),
        kind: MediaKind::Video,
        mime_type: Some("video/mp4".to_string()),
        size_bytes: Some(50000),
        file_name: Some("video.mp4".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(vec![3, 4]),
        file_reference: Some(vec![3, 4]),
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
        last_error: None,
        filter_decision: Some(FilterDecision::Skip),
        filter_reason: Some(FilterReason::TypeExcluded),
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };

    db.insert_or_update_media(&photo_rec).unwrap();
    db.insert_or_update_media(&video_rec).unwrap();

    let engine = MediaEngine::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
        temp_dir.path(),
        1,
        2,
        4,
        1,
    );

    let new_policy = MediaFilterPolicy {
        allow_photos: true,
        allow_videos: false,
        policy_version: 2,
        ..Default::default()
    };

    let newly_allowed = engine.requeue_skipped(&new_policy).unwrap();
    assert_eq!(newly_allowed, 1);

    let photo_after = db.get_media("photo_skip_1").unwrap().unwrap();
    assert_eq!(photo_after.download_status, MediaDownloadStatus::Pending);
    assert_eq!(photo_after.filter_decision, Some(FilterDecision::Allow));
    assert_eq!(photo_after.policy_version, 2);

    let video_after = db.get_media("video_skip_2").unwrap().unwrap();
    assert_eq!(video_after.download_status, MediaDownloadStatus::Skipped);
    assert_eq!(video_after.filter_decision, Some(FilterDecision::Skip));
    assert_eq!(video_after.policy_version, 2);
}

#[test]
fn media_stats_covers_all_status_enums() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));

    for (idx, &status) in MediaDownloadStatus::ALL.iter().enumerate() {
        let rec = MediaRecord {
            media_id: format!("doc_status_{idx}"),
            kind: MediaKind::Document,
            mime_type: Some("application/pdf".to_string()),
            size_bytes: Some(1000),
            file_name: Some("doc.pdf".to_string()),
            size_type: None,
            width: None,
            height: None,
            dc_id: 2,
            source_location_tl: Some(vec![1]),
            file_reference: Some(vec![1]),
            local_rel_path: None,
            sha256: None,
            download_status: status,
            downloaded_bytes: 500,
            chunk_size: 524288,
            retry_count: 0,
            max_retries: 5,
            next_retry_at: None,
            claimed_at: None,
            worker_id: None,
            last_error: None,
            filter_decision: Some(FilterDecision::Allow),
            filter_reason: None,
            policy_version: 1,
            verification_status: MediaVerificationStatus::Unverified,
            created_at: 1700000000,
            updated_at: 1700000000,
        };
        db.insert_or_update_media(&rec).unwrap();
    }

    let stats = db.get_media_stats().unwrap();
    assert_eq!(stats.total_count, MediaDownloadStatus::ALL.len() as i64);
    assert_eq!(stats.pending_count, 1);
    assert_eq!(stats.resolving_count, 1);
    assert_eq!(stats.downloading_count, 1);
    assert_eq!(stats.paused_count, 1);
    assert_eq!(stats.retry_wait_count, 1);
    assert_eq!(stats.completed_count, 1);
    assert_eq!(stats.verification_failed_count, 1);
    assert_eq!(stats.needs_reauth_count, 1);
    assert_eq!(stats.needs_file_ref_refresh_count, 1);
    assert_eq!(stats.needs_dc_migration_count, 1);
    assert_eq!(stats.permanently_failed_count, 1);
    assert_eq!(stats.skipped_count, 1);
}

#[tokio::test]
async fn final_short_chunk_crash_resumes_from_aligned_checkpoint() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let temp_dir = TempDir::new().expect("tempdir failed");
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let payload: Vec<u8> = (0..1500).map(|i| (i % 250) as u8).collect();
    let loc_tl = vec![50, 51, 52, 53];
    fake_adapter.add_file(loc_tl.clone(), payload.clone());

    let mut record = MediaRecord {
        media_id: "doc_short_crash".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/octet-stream".to_string()),
        size_bytes: Some(1500),
        file_name: Some("short.bin".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(loc_tl.clone()),
        file_reference: Some(vec![1, 2, 3]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 1024,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&record).unwrap();

    let layout = StorageLayoutManager::new(temp_dir.path());
    layout.ensure_dirs().unwrap();

    db.update_media_progress("doc_short_crash", 1024).unwrap();
    let temp_path = layout.temp_part_path("doc_short_crash");
    fs::write(&temp_path, &payload).unwrap();

    let med_before = db.get_media("doc_short_crash").unwrap().unwrap();
    assert_eq!(med_before.downloaded_bytes, 1024);
    assert_eq!(med_before.downloaded_bytes % 1024, 0);

    let downloader = SingleMediaDownloader::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
        layout.clone(),
    );

    record.downloaded_bytes = 1024;
    let res = downloader.download_item(&mut record).await.unwrap();
    assert_eq!(res, sha256_hex(&payload));

    let med_after = db.get_media("doc_short_crash").unwrap().unwrap();
    assert_eq!(med_after.download_status, MediaDownloadStatus::Completed);
    let final_dest = layout.resolve_path(med_after.local_rel_path.as_ref().unwrap());
    let final_bytes = fs::read(&final_dest).unwrap();
    assert_eq!(final_bytes.len(), 1500);
    assert_eq!(final_bytes, payload);
}

#[tokio::test]
async fn refresh_retry_preserves_worker_claim() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));
    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let payload = vec![0x55; 2048];
    let loc_tl = vec![60, 61, 62, 63];
    fake_adapter.add_file(loc_tl.clone(), payload);

    let photo = tl::types::Photo {
        has_stickers: false,
        id: 777,
        access_hash: 888,
        file_reference: vec![9, 9, 9],
        date: 1700000000,
        sizes: vec![tl::enums::PhotoSize::Size(tl::types::PhotoSize {
            r#type: "x".to_string(),
            w: 800,
            h: 600,
            size: 2048,
        })],
        video_sizes: None,
        dc_id: 2,
    };
    let msg = make_dummy_message(
        10,
        100,
        Some(tl::enums::MessageMedia::Photo(
            tl::types::MessageMediaPhoto {
                spoiler: false,
                live_photo: false,
                video: None,
                photo: Some(tl::enums::Photo::Photo(photo)),
                ttl_seconds: None,
            },
        )),
    );
    let msg_rec = MessageRecord {
        key: MessageKey::new(PeerId::new(100), MessageId::new(10)),
        date: 1700000000,
        sender_id: Some(PeerId::new(100)),
        text: None,
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: None,
        views: None,
        forwards_count: None,
        raw_tl: Some(msg.to_bytes()),
    };
    db.insert_or_update_message(&msg_rec).unwrap();
    fake_adapter.add_peer(PeerRecord {
        peer_id: PeerId::new(100),
        peer_type: PeerType::User,
        name: Some("User 100".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    });
    fake_adapter.add_message(msg_rec.clone());

    let record = MediaRecord {
        media_id: "photo_777_x".to_string(),
        kind: MediaKind::Photo,
        mime_type: Some("image/jpeg".to_string()),
        size_bytes: Some(2048),
        file_name: Some("photo.jpg".to_string()),
        size_type: Some("x".to_string()),
        width: Some(800),
        height: Some(600),
        dc_id: 2,
        source_location_tl: Some(loc_tl),
        file_reference: Some(vec![1, 1, 1]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 1024,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&record).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: MessageKey::new(PeerId::new(100), MessageId::new(10)),
        media_id: "photo_777_x".to_string(),
        role: MediaRole::Attachment,
        position: 0,
    })
    .unwrap();

    let mut claimed_by_a = db.claim_next_pending_media("worker_a").unwrap().unwrap();
    assert_eq!(claimed_by_a.worker_id.as_deref(), Some("worker_a"));
    assert_eq!(
        claimed_by_a.download_status,
        MediaDownloadStatus::Downloading
    );

    let refresher = FileReferenceRefresher::new(
        Arc::clone(&db),
        Arc::clone(&fake_adapter) as Arc<dyn TelegramAdapter>,
    );
    refresher
        .refresh_file_reference_while_claimed(&mut claimed_by_a, "worker_a")
        .await
        .expect("refresh failed");

    let claimed_by_b = db.claim_next_pending_media("worker_b").unwrap();
    assert!(
        claimed_by_b.is_none(),
        "Worker B must not claim an item currently owned and being refreshed by Worker A"
    );

    let med = db.get_media("photo_777_x").unwrap().unwrap();
    assert_eq!(med.worker_id.as_deref(), Some("worker_a"));
    assert_eq!(med.download_status, MediaDownloadStatus::Downloading);
    assert_eq!(med.file_reference.as_deref(), Some(&[9, 9, 9][..]));
}

#[test]
fn migrate_retry_preserves_worker_claim() {
    let db = Arc::new(ArchiveDb::open_in_memory().expect("db open failed"));

    let record = MediaRecord {
        media_id: "doc_dc_migrate_claim".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/pdf".to_string()),
        size_bytes: Some(4096),
        file_name: Some("doc.pdf".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(vec![1, 2, 3]),
        file_reference: Some(vec![4, 5, 6]),
        local_rel_path: None,
        sha256: None,
        download_status: MediaDownloadStatus::Pending,
        downloaded_bytes: 0,
        chunk_size: 1024,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: Some(FilterDecision::Allow),
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&record).unwrap();

    let claimed_by_a = db.claim_next_pending_media("worker_a").unwrap().unwrap();
    assert_eq!(claimed_by_a.worker_id.as_deref(), Some("worker_a"));

    db.update_media_dc_while_claimed(&claimed_by_a.media_id, 4, "worker_a")
        .unwrap();

    let claimed_by_b = db.claim_next_pending_media("worker_b").unwrap();
    assert!(
        claimed_by_b.is_none(),
        "Worker B must not claim an item currently owned and migrating DC by Worker A"
    );

    let med = db.get_media("doc_dc_migrate_claim").unwrap().unwrap();
    assert_eq!(med.worker_id.as_deref(), Some("worker_a"));
    assert_eq!(med.download_status, MediaDownloadStatus::Downloading);
    assert_eq!(med.dc_id, 4);
}

#[test]
fn concurrent_same_hash_finalization_replaces_corrupt_destination() {
    let temp_dir = TempDir::new().expect("tempdir failed");
    let layout = StorageLayoutManager::new(temp_dir.path());
    layout.ensure_dirs().unwrap();

    let valid_data = b"SHARED_CANONICAL_PAYLOAD_XYZ";
    let valid_hash = sha256_hex(valid_data);
    let valid_size = valid_data.len() as i64;

    let temp_path_a = layout.temp_part_path("item_a");
    let temp_path_b = layout.temp_part_path("item_b");
    fs::write(&temp_path_a, valid_data).unwrap();
    fs::write(&temp_path_b, valid_data).unwrap();

    let rel_path =
        StorageLayoutManager::content_addressed_rel_path(&valid_hash, Some("shared.bin"));
    let final_dest = layout.resolve_path(&rel_path);

    if let Some(parent) = final_dest.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&final_dest, b"CORRUPT_PAYLOAD").unwrap();

    layout
        .finalize_temp_file(&temp_path_a, &final_dest, &valid_hash, valid_size)
        .expect("A finalize failed");

    layout
        .finalize_temp_file(&temp_path_b, &final_dest, &valid_hash, valid_size)
        .expect("B finalize failed");

    let final_bytes = fs::read(&final_dest).unwrap();
    assert_eq!(final_bytes, valid_data);
    assert!(!temp_path_a.exists());
    assert!(!temp_path_b.exists());
}

#[tokio::test]
async fn custom_reaction_media_syncs_idempotently() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = ArchiveDb::open(&db_path).unwrap();

    let storage_layout = StorageLayoutManager::new(dir.path());
    storage_layout.ensure_dirs().unwrap();

    let fake_adapter = Arc::new(FakeTelegramAdapter::new());

    let peer = PeerRecord {
        peer_id: PeerId::new(99001),
        peer_type: PeerType::Channel,
        name: Some("Reactions Chat".to_string()),
        username: None,
        phone: None,
        updated_at: 1700000000,
        raw_tl: None,
    };
    db.upsert_peer(&peer).unwrap();

    let msg1 = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(1)),
        date: 1700000001,
        sender_id: Some(peer.peer_id),
        text: Some("First msg".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: Some(r#"{
            "Reactions": {
                "results": [
                    { "Count": { "reaction": { "CustomEmoji": { "document_id": 5256103272296499934 } }, "count": 1, "chosen_order": null } }
                ],
                "recent_reactions": []
            }
        }"#.to_string()),
        views: None,
        forwards_count: None,
        raw_tl: None,
    };

    let msg2 = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(2)),
        date: 1700000002,
        sender_id: Some(peer.peer_id),
        text: Some("Second msg with two custom reactions".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: Some(r#"{
            "Reactions": {
                "results": [
                    { "Count": { "reaction": { "CustomEmoji": { "document_id": 5256103272296499934 } }, "count": 2, "chosen_order": null } },
                    { "Count": { "reaction": { "CustomEmoji": { "document_id": 8888888888888888888 } }, "count": 1, "chosen_order": null } }
                ],
                "recent_reactions": []
            }
        }"#.to_string()),
        views: None,
        forwards_count: None,
        raw_tl: None,
    };

    db.insert_messages_batch(&[msg1, msg2]).unwrap();

    let discovered_ids = db.list_custom_emoji_reaction_document_ids().unwrap();
    assert_eq!(discovered_ids.len(), 2);
    assert!(discovered_ids.contains(&5256103272296499934));
    assert!(discovered_ids.contains(&8888888888888888888));

    let doc1 = tl::enums::Document::Document(tl::types::Document {
        id: 5256103272296499934,
        access_hash: 111222,
        file_reference: vec![1, 2, 3],
        date: 1700000000,
        mime_type: "image/webp".to_string(),
        size: 16,
        thumbs: None,
        video_thumbs: None,
        dc_id: 2,
        attributes: vec![],
    });
    fake_adapter.set_custom_emoji_document(doc1);

    let doc1_loc = tl::enums::InputFileLocation::InputDocumentFileLocation(
        tl::types::InputDocumentFileLocation {
            id: 5256103272296499934,
            access_hash: 111222,
            file_reference: vec![1, 2, 3],
            thumb_size: String::new(),
        },
    )
    .to_bytes();
    let doc1_bytes = b"WEBP_STATIC_PAYLOAD".to_vec();
    fake_adapter.add_file(doc1_loc, doc1_bytes.clone());

    let doc2 = tl::enums::Document::Document(tl::types::Document {
        id: 8888888888888888888,
        access_hash: 333444,
        file_reference: vec![4, 5, 6],
        date: 1700000000,
        mime_type: "application/x-tgsticker".to_string(),
        size: 500,
        thumbs: Some(vec![tl::enums::PhotoSize::Size(tl::types::PhotoSize {
            r#type: "m".to_string(),
            w: 100,
            h: 100,
            size: 24,
        })]),
        video_thumbs: None,
        dc_id: 4,
        attributes: vec![],
    });
    fake_adapter.set_custom_emoji_document(doc2);

    let doc2_thumb_loc = tl::enums::InputFileLocation::InputDocumentFileLocation(
        tl::types::InputDocumentFileLocation {
            id: 8888888888888888888,
            access_hash: 333444,
            file_reference: vec![4, 5, 6],
            thumb_size: "m".to_string(),
        },
    )
    .to_bytes();
    let doc2_thumb_bytes = b"THUMB_STATIC_PAYLOAD".to_vec();
    fake_adapter.add_file(doc2_thumb_loc, doc2_thumb_bytes.clone());

    let mut progress_events = Vec::new();
    let adapter_trait: Arc<dyn TelegramAdapter> = fake_adapter.clone();
    let summary1 =
        vendetta_media::sync_all_custom_reactions(&db, &adapter_trait, &storage_layout, |p| {
            progress_events.push(p.clone())
        })
        .await
        .unwrap();

    assert_eq!(summary1.total_discovered, 2);
    assert_eq!(summary1.downloaded, 2);
    assert_eq!(summary1.already_existed, 0);
    assert_eq!(summary1.unavailable, 0);
    assert_eq!(summary1.failed, 0);
    assert_eq!(
        summary1.total_bytes,
        (doc1_bytes.len() + doc2_thumb_bytes.len()) as u64
    );

    let file1_path = storage_layout.reaction_path(5256103272296499934);
    let file2_path = storage_layout.reaction_path(8888888888888888888);
    assert!(file1_path.is_file());
    assert!(file2_path.is_file());
    assert_eq!(fs::read(&file1_path).unwrap(), doc1_bytes);
    assert_eq!(fs::read(&file2_path).unwrap(), doc2_thumb_bytes);

    let summary2 =
        vendetta_media::sync_all_custom_reactions(&db, &adapter_trait, &storage_layout, |_p| {})
            .await
            .unwrap();

    assert_eq!(summary2.total_discovered, 2);
    assert_eq!(summary2.downloaded, 0);
    assert_eq!(summary2.already_existed, 2);
    assert_eq!(summary2.unavailable, 0);
    assert_eq!(summary2.failed, 0);
}

#[tokio::test]
async fn test_missing_media_requeued_for_recovery() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = Arc::new(ArchiveDb::open(&db_path).unwrap());
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let layout = StorageLayoutManager::new(dir.path());

    let valid_data = b"RECOVERABLE_PAYLOAD".to_vec();
    let valid_hash = format!("{:x}", Sha256::digest(&valid_data));
    let rel_path_photo = format!("media/01/{}.jpg", valid_hash);
    let rel_path_video = format!("media/02/{}.mp4", valid_hash);
    let rel_path_doc = format!("media/03/{}.pdf", valid_hash);

    let records = vec![
        MediaRecord {
            media_id: "photo_1001".to_string(),
            kind: MediaKind::Photo,
            mime_type: Some("image/jpeg".to_string()),
            size_bytes: Some(valid_data.len() as i64),
            file_name: Some("photo.jpg".to_string()),
            size_type: None,
            width: None,
            height: None,
            dc_id: 2,
            source_location_tl: Some(vec![1, 2, 3]),
            file_reference: Some(vec![4, 5, 6]),
            local_rel_path: Some(rel_path_photo.clone()),
            sha256: Some(valid_hash.clone()),
            download_status: MediaDownloadStatus::Completed,
            downloaded_bytes: valid_data.len() as i64,
            chunk_size: 524288,
            retry_count: 0,
            max_retries: 5,
            next_retry_at: None,
            claimed_at: None,
            worker_id: None,
            last_error: None,
            filter_decision: None,
            filter_reason: None,
            policy_version: 1,
            verification_status: MediaVerificationStatus::Verified,
            created_at: 1000,
            updated_at: 1000,
        },
        MediaRecord {
            media_id: "video_2002".to_string(),
            kind: MediaKind::Video,
            mime_type: Some("video/mp4".to_string()),
            size_bytes: Some(valid_data.len() as i64),
            file_name: Some("video.mp4".to_string()),
            size_type: None,
            width: None,
            height: None,
            dc_id: 2,
            source_location_tl: Some(vec![7, 8, 9]),
            file_reference: Some(vec![4, 5, 6]),
            local_rel_path: Some(rel_path_video.clone()),
            sha256: Some(valid_hash.clone()),
            download_status: MediaDownloadStatus::Completed,
            downloaded_bytes: valid_data.len() as i64,
            chunk_size: 524288,
            retry_count: 0,
            max_retries: 5,
            next_retry_at: None,
            claimed_at: None,
            worker_id: None,
            last_error: None,
            filter_decision: None,
            filter_reason: None,
            policy_version: 1,
            verification_status: MediaVerificationStatus::Verified,
            created_at: 1000,
            updated_at: 1000,
        },
        MediaRecord {
            media_id: "doc_3003".to_string(),
            kind: MediaKind::Document,
            mime_type: Some("application/pdf".to_string()),
            size_bytes: Some(valid_data.len() as i64),
            file_name: Some("doc.pdf".to_string()),
            size_type: None,
            width: None,
            height: None,
            dc_id: 2,
            source_location_tl: Some(vec![10, 11, 12]),
            file_reference: Some(vec![4, 5, 6]),
            local_rel_path: Some(rel_path_doc.clone()),
            sha256: Some(valid_hash.clone()),
            download_status: MediaDownloadStatus::Completed,
            downloaded_bytes: valid_data.len() as i64,
            chunk_size: 524288,
            retry_count: 0,
            max_retries: 5,
            next_retry_at: None,
            claimed_at: None,
            worker_id: None,
            last_error: None,
            filter_decision: None,
            filter_reason: None,
            policy_version: 1,
            verification_status: MediaVerificationStatus::Verified,
            created_at: 1000,
            updated_at: 1000,
        },
    ];

    for r in &records {
        db.insert_or_update_media(r).unwrap();
    }

    let abs_photo = layout.resolve_canonical_path(&rel_path_photo);
    fs::create_dir_all(abs_photo.parent().unwrap()).unwrap();
    fs::write(&abs_photo, &valid_data).unwrap();

    let engine = MediaEngine::new(Arc::clone(&db), adapter.clone(), dir.path(), 1, 2, 2, 1);

    let rep = engine.reconcile_startup().expect("reconciliation failed");
    assert_eq!(rep.missing_file_marked_count, 2);

    let p = db.get_media("photo_1001").unwrap().unwrap();
    let v = db.get_media("video_2002").unwrap().unwrap();
    let d = db.get_media("doc_3003").unwrap().unwrap();

    assert_eq!(p.download_status, MediaDownloadStatus::Completed);
    assert_eq!(v.download_status, MediaDownloadStatus::Pending);
    assert_eq!(v.verification_status, MediaVerificationStatus::MissingFile);
    assert_eq!(d.download_status, MediaDownloadStatus::Pending);
    assert_eq!(d.verification_status, MediaVerificationStatus::MissingFile);

    let q_stats = db.get_queue_stats().unwrap();
    assert_eq!(q_stats.eligible_count, 2);
    assert_eq!(q_stats.expected_bytes, (valid_data.len() * 2) as u64);
    assert!(q_stats.all_sizes_known);

    adapter.add_file(vec![7, 8, 9], valid_data.clone());
    adapter.add_file(vec![10, 11, 12], valid_data.clone());

    let summary = engine.download_all_pending("test_recovery").await;
    assert_eq!(summary, 2);

    let v_after = db.get_media("video_2002").unwrap().unwrap();
    let d_after = db.get_media("doc_3003").unwrap().unwrap();
    assert_eq!(v_after.download_status, MediaDownloadStatus::Completed);
    assert_eq!(d_after.download_status, MediaDownloadStatus::Completed);
}

#[tokio::test]
async fn test_dual_path_storage_resolution_legacy_fallback() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = Arc::new(ArchiveDb::open(&db_path).unwrap());
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let media_dir = dir.path().join("media");
    let layout = StorageLayoutManager::new(&media_dir);

    let test_data = b"LEGACY_NESTED_PAYLOAD".to_vec();
    let test_hash = format!("{:x}", Sha256::digest(&test_data));
    let rel_path = format!("media/45/{}.jpg", test_hash);

    let legacy_file = media_dir
        .join("media")
        .join("45")
        .join(format!("{}.jpg", test_hash));
    fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();
    fs::write(&legacy_file, &test_data).unwrap();

    let record = MediaRecord {
        media_id: "legacy_media_45".to_string(),
        kind: MediaKind::Photo,
        mime_type: Some("image/jpeg".to_string()),
        size_bytes: Some(test_data.len() as i64),
        file_name: Some("photo.jpg".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(vec![1, 2, 3]),
        file_reference: Some(vec![4, 5, 6]),
        local_rel_path: Some(rel_path.clone()),
        sha256: Some(test_hash.clone()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: test_data.len() as i64,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: None,
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1000,
        updated_at: 1000,
    };
    db.insert_or_update_media(&record).unwrap();

    let resolved = layout.resolve_path(&rel_path);
    assert_eq!(resolved, legacy_file);
    assert!(resolved.exists());

    let engine = MediaEngine::new(Arc::clone(&db), adapter, &media_dir, 1, 2, 2, 1);
    let rep = engine.reconcile_startup().expect("reconcile failed");
    assert_eq!(rep.missing_file_marked_count, 0);

    let v_rep = engine.verify_media().expect("verify failed");
    assert_eq!(v_rep.total_checked, 1);
    assert_eq!(v_rep.verified_count, 1);
    assert_eq!(v_rep.missing_count, 0);
}

#[tokio::test]
async fn test_idempotent_repeated_reconciliation() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = Arc::new(ArchiveDb::open(&db_path).unwrap());
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let layout = StorageLayoutManager::new(dir.path());

    let valid_data = b"STABLE_PAYLOAD".to_vec();
    let valid_hash = format!("{:x}", Sha256::digest(&valid_data));
    let rel_path = format!("media/99/{}.jpg", valid_hash);
    let abs_path = layout.resolve_canonical_path(&rel_path);
    fs::create_dir_all(abs_path.parent().unwrap()).unwrap();
    fs::write(&abs_path, &valid_data).unwrap();

    let record = MediaRecord {
        media_id: "stable_media_99".to_string(),
        kind: MediaKind::Photo,
        mime_type: Some("image/jpeg".to_string()),
        size_bytes: Some(valid_data.len() as i64),
        file_name: Some("photo.jpg".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: Some(vec![1, 2, 3]),
        file_reference: Some(vec![4, 5, 6]),
        local_rel_path: Some(rel_path.clone()),
        sha256: Some(valid_hash.clone()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: valid_data.len() as i64,
        chunk_size: 524288,
        retry_count: 0,
        max_retries: 5,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: None,
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Verified,
        created_at: 1000,
        updated_at: 1000,
    };
    db.insert_or_update_media(&record).unwrap();

    let engine = MediaEngine::new(Arc::clone(&db), adapter, dir.path(), 1, 2, 2, 1);

    for i in 1..=5 {
        let rep = engine.reconcile_startup().expect("reconcile failed");
        assert_eq!(rep.missing_file_marked_count, 0, "Failed at iteration {i}");
        assert_eq!(
            rep.corrupted_file_marked_count, 0,
            "Failed at iteration {i}"
        );
    }
}
