use std::{
    collections::{HashSet, VecDeque},
    fs,
};
use tempfile::{TempDir, tempdir};

use grammers_tl_types::{self as tl, Serializable};
use vendetta_model::{
    FilterDecision, MediaDownloadStatus, MediaKind, MediaRecord, MediaRole,
    MediaVerificationStatus, MessageId, MessageKey, MessageMediaJoin, MessageRecord, MessageState,
    PeerId, PeerRecord, PeerType,
};
use vendetta_render::{
    HtmlArchiveExporter,
    assets::css::THEME_CSS,
    entity::render_formatted_text,
    manifest::{DatasetFingerprint, HtmlExportManifest},
    media::validate_and_clean_media_rel_path,
    message::{GroupingContext, render_message_bubble},
    model::{ExportOptions, MediaMode, PresentationMode, RenderMediaItem, RenderMessage},
    reply::ReplyLocationMap,
    search::{
        SearchIndexer,
        ranking::{
            BoundedSearchResult, BoundedTopResults, score_search_query, tokenize_search_text,
        },
    },
    url_builder::ArchiveUrlBuilder,
    verifier::HtmlArchiveVerifier,
};
use vendetta_storage::ArchiveDb;

fn create_test_db() -> (ArchiveDb, TempDir) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_archive.db");
    let db = ArchiveDb::open(&db_path).unwrap();
    (db, dir)
}

#[test]
fn revision_specific_entity_rendering_supports_multiversion() {
    let rev1_text = "Hello world";
    let rev1_entities = vec![tl::enums::MessageEntity::Bold(
        tl::types::MessageEntityBold {
            offset: 0,
            length: 5,
        },
    )];
    let rev1_json = serde_json::to_string(&rev1_entities).unwrap();
    let rev1_html = render_formatted_text(rev1_text, Some(&rev1_json));
    assert_eq!(rev1_html, "<strong>Hello</strong> world");

    let rev2_text = "Visit https://telegram.org now";
    let rev2_entities = vec![tl::enums::MessageEntity::TextUrl(
        tl::types::MessageEntityTextUrl {
            offset: 6,
            length: 20,
            url: "https://telegram.org".to_string(),
        },
    )];
    let rev2_json = serde_json::to_string(&rev2_entities).unwrap();
    let rev2_html = render_formatted_text(rev2_text, Some(&rev2_json));
    assert!(rev2_html.contains("<a href=\"https://telegram.org\""));

    let plain_html = render_formatted_text("Plain <text> & symbols", None);
    assert_eq!(plain_html, "Plain &lt;text&gt; &amp; symbols");
}

#[test]
fn unicode_cyrillic_ukrainian_search_tokenizes_correctly() {
    let text = "Привет мир! Доброго вечора, Україно! Ελληνικά 123";
    let tokens = tokenize_search_text(text);

    assert!(tokens.contains(&"привет".to_string()));
    assert!(tokens.contains(&"мир".to_string()));
    assert!(tokens.contains(&"доброго".to_string()));
    assert!(tokens.contains(&"вечора".to_string()));
    assert!(tokens.contains(&"україно".to_string()));
    assert!(tokens.contains(&"ελληνικά".to_string()));
    assert!(tokens.contains(&"123".to_string()));

    let score = score_search_query("Україно", text, &tokens);
    assert_eq!(score, Some(100));

    let score_multi = score_search_query("привет вечора", text, &tokens);
    assert_eq!(score_multi, Some(50));
}

#[test]
fn media_relative_paths_validate_against_traversal() {
    assert!(validate_and_clean_media_rel_path("../../outside.jpg").is_err());
    assert!(validate_and_clean_media_rel_path("/etc/passwd").is_err());
    assert!(validate_and_clean_media_rel_path("C:\\Windows\\system32").is_err());
    assert!(validate_and_clean_media_rel_path("").is_err());
    assert!(validate_and_clean_media_rel_path("media/../secret.jpg").is_err());

    let clean1 = validate_and_clean_media_rel_path("media/00/photo.jpg").unwrap();
    assert_eq!(clean1.to_str().unwrap(), "00/photo.jpg");

    let clean2 = validate_and_clean_media_rel_path("sub/dir/photo.png").unwrap();
    assert_eq!(clean2.to_str().unwrap(), "sub/dir/photo.png");
}

#[test]
fn service_message_renders_from_raw_tl() {
    let (db, _db_dir) = create_test_db();
    let export_temp = tempdir().unwrap();
    let export_dir = export_temp.path().join("service_msg_export");

    let peer = PeerRecord {
        peer_id: PeerId::new(1001),
        peer_type: PeerType::Group,
        name: Some("Service Group".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let service_tl = tl::enums::Message::Service(tl::types::MessageService {
        out: false,
        mentioned: false,
        media_unread: false,
        silent: false,
        post: false,
        legacy: false,
        id: 1,
        from_id: Some(tl::enums::Peer::User(tl::types::PeerUser { user_id: 100 })),
        peer_id: tl::enums::Peer::Chat(tl::types::PeerChat { chat_id: 1001 }),
        reply_to: None,
        date: 1700000000,
        action: tl::enums::MessageAction::PinMessage,
        ttl_period: None,
        reactions: None,
        reactions_are_possible: false,
        saved_peer_id: None,
    });

    let msg = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(1)),
        date: 1700000000,
        sender_id: Some(PeerId::new(100)),
        text: Some("Pinned a message".to_string()),
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
        raw_tl: Some(service_tl.to_bytes()),
    };
    db.insert_messages_batch(&[msg]).unwrap();

    let options = ExportOptions {
        output_dir: export_dir.clone(),
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.messages_count, 1);

    let page_content = fs::read_to_string(export_dir.join("chats/p_1001/page_00001.html")).unwrap();
    assert!(page_content.contains("system-event"));
    assert!(page_content.contains("Pinned a message"));
}

#[test]
fn archive_optimized_renders_accessible_media_links() {
    let photo_rec = MediaRecord {
        media_id: "photo123".to_string(),
        kind: MediaKind::Photo,
        mime_type: Some("image/jpeg".to_string()),
        size_bytes: Some(1024 * 500),
        file_name: Some("test_photo.jpg".to_string()),
        size_type: Some("x".to_string()),
        width: Some(800),
        height: Some(600),
        dc_id: 1,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some("media/00/photo123.jpg".to_string()),
        sha256: Some("hash123".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 1024 * 500,
        chunk_size: 1024 * 128,
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

    let render_msg = RenderMessage {
        key: MessageKey::new(PeerId::new(100), MessageId::new(1)),
        date: 1700000000,
        sender_id: Some(PeerId::new(10)),
        sender_name: Some("Alice".to_string()),
        is_outgoing: false,
        state: MessageState::Active,
        formatted_html: Some("Hello photo".to_string()),
        raw_text: Some("Hello photo".to_string()),
        reply_preview: None,
        forward_info: None,
        media_items: vec![RenderMediaItem {
            record: photo_rec,
            relative_url: Some("../../media/00/photo123.jpg".to_string()),
            is_available: true,
            unavailable_reason: None,
        }],
        revisions: vec![],
        grouped_id: None,
        is_service: false,
        service_description: None,
        views: None,
        forwards_count: None,
        author_signature: None,
        reply_to_top_id: None,
        reactions: vec![],
        is_channel_post: false,
        comments_count: None,
        has_comments: false,
    };

    let dense_html = render_message_bubble(
        &render_msg,
        &GroupingContext::default(),
        PresentationMode::ArchiveOptimized,
        &HashSet::new(),
    );

    assert!(dense_html.contains("<a href=\"../../media/00/photo123.jpg\""));
    assert!(dense_html.contains("test_photo.jpg"));
    assert!(dense_html.contains("dense-media-link"));
}

#[test]
fn last_message_date_reflects_newest_message() {
    let (db, _db_dir) = create_test_db();
    let peer = PeerRecord {
        peer_id: PeerId::new(1001),
        peer_type: PeerType::Group,
        name: Some("Recent Chat".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let m1 = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(1)),
        date: 1000,
        sender_id: None,
        text: Some("Old message".to_string()),
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
    let m2 = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(2)),
        date: 5000,
        sender_id: None,
        text: Some("New message".to_string()),
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
    db.insert_messages_batch(&[m1, m2]).unwrap();

    let last_date = db.get_last_message_date_by_peer(peer.peer_id).unwrap();
    assert_eq!(last_date, Some(5000));
}

#[test]
fn dataset_fingerprint_changes_on_content_change() {
    let fp1 = DatasetFingerprint::compute(1, 10, 2, 5, &[100, 200], Some("digest_v1"));
    let fp2 = DatasetFingerprint::compute(1, 10, 2, 5, &[100, 200], Some("digest_v1"));
    let fp3 = DatasetFingerprint::compute(1, 10, 2, 5, &[100, 200], Some("digest_v2"));

    assert_eq!(fp1, fp2);
    assert_ne!(fp1.source_digest, fp3.source_digest);
}

#[test]
fn verifier_detects_duplicate_anchors() {
    let temp = tempdir().unwrap();
    let export_dir = temp.path().join("dup_anchor_export");
    fs::create_dir_all(export_dir.join("assets/icons")).unwrap();
    fs::create_dir_all(export_dir.join("chats/p_100")).unwrap();

    let page_html = r#"<!DOCTYPE html>
<html>
<body>
  <div id="m-p_100-1">Message 1</div>
  <div id="m-p_100-1">Duplicate Message 1</div>
</body>
</html>"#;
    fs::write(export_dir.join("chats/p_100/page_00001.html"), page_html).unwrap();
    fs::write(export_dir.join("index.html"), "<html><body></body></html>").unwrap();

    let verifier = HtmlArchiveVerifier::new(&export_dir);
    let err = verifier.verify().unwrap_err();
    assert!(err.to_string().contains("Duplicate anchor"));
}

#[test]
fn include_filters_restrict_exported_peers() {
    let (db, _db_dir) = create_test_db();
    let export_temp = tempdir().unwrap();
    let export_dir = export_temp.path().join("filter_test_export");

    let peer = PeerRecord {
        peer_id: PeerId::new(1001),
        peer_type: PeerType::Group,
        name: Some("Filter Group".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let active_msg = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(1)),
        date: 1000,
        sender_id: None,
        text: Some("Active msg".to_string()),
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
    let deleted_msg = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(2)),
        date: 2000,
        sender_id: None,
        text: Some("Deleted msg".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Deleted,
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
    db.insert_messages_batch(&[active_msg, deleted_msg])
        .unwrap();

    let options = ExportOptions {
        output_dir: export_dir.clone(),
        include_deleted_messages: false,
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.messages_count, 1);
}

#[test]
fn full_export_and_verification_run_end_to_end() {
    let (db, _db_dir) = create_test_db();
    let export_temp = tempdir().unwrap();
    let export_dir = export_temp.path().join("full_e2e_export");

    let peer = PeerRecord {
        peer_id: PeerId::new(500),
        peer_type: PeerType::User,
        name: Some("Bob".to_string()),
        username: Some("bob_tg".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let mut msgs = Vec::new();
    for i in 1..=50 {
        msgs.push(MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new(i)),
            date: 1700000000 + i * 60,
            sender_id: Some(peer.peer_id),
            text: Some(format!("Message number {i}")),
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: if i > 1 {
                Some(MessageId::new(i - 1))
            } else {
                None
            },
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: None,
            forward_json: None,
            reactions_json: None,
            views: Some(10),
            forwards_count: Some(2),
            raw_tl: None,
        });
    }
    db.insert_messages_batch(&msgs).unwrap();

    let options = ExportOptions {
        output_dir: export_dir.clone(),
        chunk_size: 20,
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();

    assert_eq!(summary.dialogs_count, 1);
    assert_eq!(summary.messages_count, 50);
    assert_eq!(summary.chunks_count, 3);

    let verifier = HtmlArchiveVerifier::new(&export_dir);
    let report = verifier.verify().unwrap();
    assert!(report.is_success());
    assert_eq!(report.total_pages_checked, 4);
}

#[test]
fn existing_export_directory_without_replace_flag_errors_safely() {
    let (db, _db_dir) = create_test_db();
    let export_temp = tempdir().unwrap();
    let export_dir = export_temp.path().join("existing_export");
    fs::create_dir_all(&export_dir).unwrap();

    let options = ExportOptions {
        output_dir: export_dir.clone(),
        replace: false,
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let err = exporter.export().unwrap_err();
    assert!(err.to_string().contains("already exists"));

    let options_replace = ExportOptions {
        output_dir: export_dir.clone(),
        replace: true,
        ..Default::default()
    };

    let exporter_replace = HtmlArchiveExporter::new(&db, options_replace);
    assert!(exporter_replace.export().is_ok());
}

#[test]
fn album_grouped_messages_render_on_single_page() {
    let (db, _db_dir) = create_test_db();
    let export_temp = tempdir().unwrap();
    let export_dir = export_temp.path().join("album_3msg_export");
    let media_src = tempdir().unwrap();

    let peer = PeerRecord {
        peer_id: PeerId::new(700),
        peer_type: PeerType::User,
        name: Some("Album Creator".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let mut msgs = Vec::new();
    for i in 1..=3 {
        msgs.push(MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new(i)),
            date: 1700000000 + i * 2,
            sender_id: Some(peer.peer_id),
            text: if i == 1 {
                Some("Here are the photos from our trip".to_string())
            } else {
                None
            },
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: None,
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: Some(999),
            forward_json: None,
            reactions_json: None,
            views: None,
            forwards_count: None,
            raw_tl: None,
        });
    }
    db.insert_messages_batch(&msgs).unwrap();

    fs::create_dir_all(media_src.path().join("media/photos")).unwrap();
    for i in 1..=3 {
        let rel = format!("photos/photo_{i}.jpg");
        fs::write(media_src.path().join("media").join(&rel), b"dummy photo").unwrap();

        let media = MediaRecord {
            media_id: format!("media_{i}"),
            kind: MediaKind::Photo,
            mime_type: Some("image/jpeg".to_string()),
            size_bytes: Some(1024 * i),
            file_name: Some(format!("photo_{i}.jpg")),
            size_type: None,
            width: Some(800),
            height: Some(600),
            dc_id: 2,
            source_location_tl: None,
            file_reference: None,
            local_rel_path: Some(rel),
            sha256: None,
            download_status: MediaDownloadStatus::Completed,
            downloaded_bytes: 1024 * i,
            chunk_size: 1024,
            retry_count: 0,
            max_retries: 3,
            next_retry_at: None,
            claimed_at: None,
            worker_id: None,
            last_error: None,
            filter_decision: None,
            filter_reason: None,
            policy_version: 1,
            verification_status: MediaVerificationStatus::Unverified,
            created_at: 1700000000,
            updated_at: 1700000000,
        };
        db.insert_or_update_media(&media).unwrap();
        db.link_message_media(&MessageMediaJoin {
            key: MessageKey::new(peer.peer_id, MessageId::new(i)),
            media_id: media.media_id.clone(),
            role: MediaRole::Attachment,
            position: 0,
        })
        .unwrap();
    }

    let options = ExportOptions {
        output_dir: export_dir.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        media_src_dir: Some(media_src.path().to_path_buf()),
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.messages_count, 3);

    let chat_page = export_dir.join("chats/p_700/page_00001.html");
    let html = fs::read_to_string(chat_page).unwrap();

    assert!(html.contains("data-grouped-id=\"999\""));
    assert!(html.contains("album-count-3"));
    assert!(html.contains("photo_1.jpg"));
    assert!(html.contains("photo_2.jpg"));
    assert!(html.contains("photo_3.jpg"));
    assert!(html.contains("Here are the photos from our trip"));
    assert!(html.contains("id=\"m-p_700-1\""));
    assert!(html.contains("id=\"m-p_700-2\""));
    assert!(html.contains("id=\"m-p_700-3\""));
}

#[test]
fn album_grouped_messages_handle_page_boundary_crossing() {
    let (db, _db_dir) = create_test_db();
    let export_temp = tempdir().unwrap();
    let export_dir = export_temp.path().join("album_split_export");
    let media_src = tempdir().unwrap();

    let peer = PeerRecord {
        peer_id: PeerId::new(701),
        peer_type: PeerType::User,
        name: Some("Split Album".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let msgs = vec![
        MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new(1)),
            date: 1700000001,
            sender_id: Some(peer.peer_id),
            text: Some("Album photo 1".to_string()),
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: None,
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: Some(888),
            forward_json: None,
            reactions_json: None,
            views: None,
            forwards_count: None,
            raw_tl: None,
        },
        MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new(2)),
            date: 1700000002,
            sender_id: Some(peer.peer_id),
            text: None,
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: None,
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: Some(888),
            forward_json: None,
            reactions_json: None,
            views: None,
            forwards_count: None,
            raw_tl: None,
        },
        MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new(3)),
            date: 1700000003,
            sender_id: Some(peer.peer_id),
            text: None,
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: None,
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: Some(888),
            forward_json: None,
            reactions_json: None,
            views: None,
            forwards_count: None,
            raw_tl: None,
        },
        MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new(4)),
            date: 1700000004,
            sender_id: Some(peer.peer_id),
            text: Some("Album photo 4 caption".to_string()),
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: None,
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: Some(888),
            forward_json: None,
            reactions_json: None,
            views: None,
            forwards_count: None,
            raw_tl: None,
        },
    ];
    db.insert_messages_batch(&msgs).unwrap();

    fs::create_dir_all(media_src.path().join("media/photos")).unwrap();
    for i in 1..=4 {
        let rel = format!("photos/photo_{i}.jpg");
        fs::write(media_src.path().join("media").join(&rel), b"dummy photo").unwrap();

        let media = MediaRecord {
            media_id: format!("media_{i}"),
            kind: MediaKind::Photo,
            mime_type: Some("image/jpeg".to_string()),
            size_bytes: Some(2048),
            file_name: Some(format!("photo_{i}.jpg")),
            size_type: None,
            width: Some(800),
            height: Some(600),
            dc_id: 2,
            source_location_tl: None,
            file_reference: None,
            local_rel_path: Some(rel),
            sha256: None,
            download_status: MediaDownloadStatus::Completed,
            downloaded_bytes: 2048,
            chunk_size: 1024,
            retry_count: 0,
            max_retries: 3,
            next_retry_at: None,
            claimed_at: None,
            worker_id: None,
            last_error: None,
            filter_decision: None,
            filter_reason: None,
            policy_version: 1,
            verification_status: MediaVerificationStatus::Unverified,
            created_at: 1700000000,
            updated_at: 1700000000,
        };
        db.insert_or_update_media(&media).unwrap();
        db.link_message_media(&MessageMediaJoin {
            key: MessageKey::new(peer.peer_id, MessageId::new(i)),
            media_id: media.media_id.clone(),
            role: MediaRole::Attachment,
            position: 0,
        })
        .unwrap();
    }

    let options = ExportOptions {
        output_dir: export_dir.clone(),
        chunk_size: 2,
        media_src_dir: Some(media_src.path().to_path_buf()),
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.chunks_count, 2);

    let page0_html = fs::read_to_string(export_dir.join("chats/p_701/page_00001.html")).unwrap();
    let page1_html = fs::read_to_string(export_dir.join("chats/p_701/page_00002.html")).unwrap();

    assert!(page0_html.contains("id=\"m-p_701-1\""));
    assert!(page0_html.contains("id=\"m-p_701-2\""));
    assert!(page0_html.contains("photo_1.jpg"));
    assert!(page0_html.contains("photo_2.jpg"));
    assert!(!page0_html.contains("photo_3.jpg"));
    assert!(!page0_html.contains("photo_4.jpg"));
    assert!(page0_html.contains("badge-next"));
    assert!(page0_html.contains("href=\"page_00002.html\""));
    assert!(page0_html.contains("Continues on next page"));

    assert!(page1_html.contains("id=\"m-p_701-3\""));
    assert!(page1_html.contains("id=\"m-p_701-4\""));
    assert!(page1_html.contains("photo_3.jpg"));
    assert!(page1_html.contains("photo_4.jpg"));
    assert!(!page1_html.contains("photo_1.jpg"));
    assert!(!page1_html.contains("photo_2.jpg"));
    assert!(page1_html.contains("badge-prev"));
    assert!(page1_html.contains("href=\"page_00001.html\""));
    assert!(page1_html.contains("Continued from previous page"));
}

#[test]
fn multiple_adjacent_albums_render_without_merging() {
    let (db, _db_dir) = create_test_db();
    let export_temp = tempdir().unwrap();
    let export_dir = export_temp.path().join("adjacent_albums_export");
    let media_src = tempdir().unwrap();

    let peer = PeerRecord {
        peer_id: PeerId::new(702),
        peer_type: PeerType::User,
        name: Some("Two Albums".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let msgs = vec![
        MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new(1)),
            date: 1700000001,
            sender_id: Some(peer.peer_id),
            text: Some("Album A".to_string()),
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: None,
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: Some(111),
            forward_json: None,
            reactions_json: None,
            views: None,
            forwards_count: None,
            raw_tl: None,
        },
        MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new(2)),
            date: 1700000002,
            sender_id: Some(peer.peer_id),
            text: None,
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: None,
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: Some(111),
            forward_json: None,
            reactions_json: None,
            views: None,
            forwards_count: None,
            raw_tl: None,
        },
        MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new(3)),
            date: 1700000003,
            sender_id: Some(peer.peer_id),
            text: Some("Album B".to_string()),
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: None,
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: Some(222),
            forward_json: None,
            reactions_json: None,
            views: None,
            forwards_count: None,
            raw_tl: None,
        },
        MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new(4)),
            date: 1700000004,
            sender_id: Some(peer.peer_id),
            text: None,
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: None,
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: Some(222),
            forward_json: None,
            reactions_json: None,
            views: None,
            forwards_count: None,
            raw_tl: None,
        },
    ];
    db.insert_messages_batch(&msgs).unwrap();

    fs::create_dir_all(media_src.path().join("media/photos")).unwrap();
    for i in 1..=4 {
        let rel = format!("photos/photo_{i}.jpg");
        fs::write(media_src.path().join("media").join(&rel), b"dummy photo").unwrap();

        let media = MediaRecord {
            media_id: format!("media_{i}"),
            kind: MediaKind::Photo,
            mime_type: Some("image/jpeg".to_string()),
            size_bytes: Some(1024),
            file_name: Some(format!("photo_{i}.jpg")),
            size_type: None,
            width: Some(800),
            height: Some(600),
            dc_id: 2,
            source_location_tl: None,
            file_reference: None,
            local_rel_path: Some(rel),
            sha256: None,
            download_status: MediaDownloadStatus::Completed,
            downloaded_bytes: 1024,
            chunk_size: 1024,
            retry_count: 0,
            max_retries: 3,
            next_retry_at: None,
            claimed_at: None,
            worker_id: None,
            last_error: None,
            filter_decision: None,
            filter_reason: None,
            policy_version: 1,
            verification_status: MediaVerificationStatus::Unverified,
            created_at: 1700000000,
            updated_at: 1700000000,
        };
        db.insert_or_update_media(&media).unwrap();
        db.link_message_media(&MessageMediaJoin {
            key: MessageKey::new(peer.peer_id, MessageId::new(i)),
            media_id: media.media_id.clone(),
            role: MediaRole::Attachment,
            position: 0,
        })
        .unwrap();
    }

    let exporter = HtmlArchiveExporter::new(
        &db,
        ExportOptions {
            output_dir: export_dir.clone(),
            media_src_dir: Some(media_src.path().to_path_buf()),
            ..Default::default()
        },
    );
    exporter.export().unwrap();

    let page_html = fs::read_to_string(export_dir.join("chats/p_702/page_00001.html")).unwrap();
    assert!(page_html.contains("data-grouped-id=\"111\""));
    assert!(page_html.contains("data-grouped-id=\"222\""));
}

#[test]
fn source_and_export_config_fingerprints_are_separated() {
    let (db, _db_dir) = create_test_db();
    let export_temp = tempdir().unwrap();

    let peer = PeerRecord {
        peer_id: PeerId::new(704),
        peer_type: PeerType::User,
        name: Some("Fingerprint User".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let msg = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(1)),
        date: 1700000000,
        sender_id: Some(peer.peer_id),
        text: Some("Test fingerprint".to_string()),
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
    db.insert_messages_batch(&[msg]).unwrap();

    let dir1 = export_temp.path().join("exp_telegram");
    let dir2 = export_temp.path().join("exp_dense");

    let opt1 = ExportOptions {
        output_dir: dir1.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        ..Default::default()
    };
    let opt2 = ExportOptions {
        output_dir: dir2.clone(),
        presentation_mode: PresentationMode::ArchiveOptimized,
        ..Default::default()
    };

    HtmlArchiveExporter::new(&db, opt1).export().unwrap();
    HtmlArchiveExporter::new(&db, opt2).export().unwrap();

    let m1 = HtmlExportManifest::read_from_file(&dir1.join("manifest.json")).unwrap();
    let m2 = HtmlExportManifest::read_from_file(&dir2.join("manifest.json")).unwrap();

    assert_eq!(
        m1.source_fingerprint.source_digest,
        m2.source_fingerprint.source_digest
    );
    assert_ne!(m1.export_config_fingerprint, m2.export_config_fingerprint);
}

#[test]
fn search_prefix_index_bounds_candidate_results() {
    let (db, _db_dir) = create_test_db();
    let export_temp = tempdir().unwrap();
    let export_dir = export_temp.path().join("search_prefix_export");

    let peer = PeerRecord {
        peer_id: PeerId::new(705),
        peer_type: PeerType::User,
        name: Some("Searcher".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let msg = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(1)),
        date: 1700000000,
        sender_id: Some(peer.peer_id),
        text: Some("Привіт Світ Hello Rustacean".to_string()),
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
    db.insert_messages_batch(&[msg]).unwrap();

    let exporter = HtmlArchiveExporter::new(
        &db,
        ExportOptions {
            output_dir: export_dir.clone(),
            ..Default::default()
        },
    );
    exporter.export().unwrap();

    let manifest_js = fs::read_to_string(export_dir.join("search/manifest.js")).unwrap();
    assert!(manifest_js.contains("prefix_index"));
    assert!(manifest_js.contains("\"hel\":[1]"));
    assert!(manifest_js.contains("\"rus\":[1]"));
    assert!(manifest_js.contains("\"при\":[1]"));
    assert!(manifest_js.contains("\"сві\":[1]"));
}

#[test]
fn verifier_detects_link_path_traversals() {
    let (db, _db_dir) = create_test_db();
    let export_temp = tempdir().unwrap();
    let export_dir = export_temp.path().join("traversal_export");

    let peer = PeerRecord {
        peer_id: PeerId::new(706),
        peer_type: PeerType::User,
        name: Some("Traversal Test".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let msg = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(1)),
        date: 1700000000,
        sender_id: Some(peer.peer_id),
        text: Some("Valid message".to_string()),
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
    db.insert_messages_batch(&[msg]).unwrap();

    HtmlArchiveExporter::new(
        &db,
        ExportOptions {
            output_dir: export_dir.clone(),
            ..Default::default()
        },
    )
    .export()
    .unwrap();

    let page_path = export_dir.join("chats/p_706/page_00001.html");
    let mut page_content = fs::read_to_string(&page_path).unwrap();
    page_content.push_str("<a href=\"../../../../etc/passwd\">Malicious Link</a>");
    fs::write(&page_path, page_content).unwrap();

    let verifier = HtmlArchiveVerifier::new(&export_dir);
    let report = verifier.verify().unwrap_err();
    assert!(
        report
            .to_string()
            .contains("Link traversal escape detected")
    );
}

#[test]
fn search_manifest_and_shard_verifier_perform_deep_checks() {
    let (db, _db_dir) = create_test_db();
    let export_temp = tempdir().unwrap();
    let export_dir = export_temp.path().join("search_deep_verifier_export");

    let peer = PeerRecord {
        peer_id: PeerId::new(707),
        peer_type: PeerType::User,
        name: Some("Search Verifier User".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let msg = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(1)),
        date: 1700000000,
        sender_id: Some(peer.peer_id),
        text: Some("Search verifier payload test".to_string()),
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
    db.insert_messages_batch(&[msg]).unwrap();

    HtmlArchiveExporter::new(
        &db,
        ExportOptions {
            output_dir: export_dir.clone(),
            ..Default::default()
        },
    )
    .export()
    .unwrap();

    let verifier = HtmlArchiveVerifier::new(&export_dir);
    assert!(verifier.verify().is_ok());

    let extra_shard = export_dir.join("search/shards/shard_99999.js");
    fs::write(
        &extra_shard,
        "window.__VENDETTA_REGISTER_SEARCH_SHARD__({});",
    )
    .unwrap();
    let err = verifier.verify().unwrap_err();
    assert!(err.to_string().contains("Undeclared search shard file"));
    fs::remove_file(&extra_shard).unwrap();

    let valid_shard = export_dir.join("search/shards/shard_00001.js");
    let original_content = fs::read_to_string(&valid_shard).unwrap();
    fs::write(&valid_shard, "corrupted javascript content without wrapper").unwrap();
    let err = verifier.verify().unwrap_err();
    assert!(err.to_string().contains("Invalid search shard wrapper"));

    fs::remove_file(&valid_shard).unwrap();
    let err = verifier.verify().unwrap_err();
    assert!(
        err.to_string()
            .contains("Declared search shard file missing")
    );

    fs::write(&valid_shard, original_content).unwrap();
    assert!(verifier.verify().is_ok());
}

#[test]
fn multilingual_prefix_index_matches_exhaustive_scan() {
    let (db, _db_dir) = create_test_db();
    let export_temp = tempdir().unwrap();
    let export_dir = export_temp.path().join("multilingual_search_export");

    let peer = PeerRecord {
        peer_id: PeerId::new(708),
        peer_type: PeerType::User,
        name: Some("Polyglot User".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let sample_messages = vec![
        "Hello Telegram static archive",
        "Привіт усім українським користувачам",
        "Київ - столиця України",
        "Єдність та гідність",
        "Ґанок біля будинку",
        "Café crème and fresh croissants",
        "Resume and CV document updated",
        "Year 2024 milestone 6 completed",
        "42 is the ultimate answer",
        "C++ and Rust systems programming",
    ];

    let mut msgs = Vec::new();
    for (i, text) in sample_messages.into_iter().enumerate() {
        msgs.push(MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new((i + 1) as i64)),
            date: 1700000000 + (i as i64 * 10),
            sender_id: Some(peer.peer_id),
            text: Some(text.to_string()),
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
        });
    }
    db.insert_messages_batch(&msgs).unwrap();

    let loc_map = ReplyLocationMap::new();
    let indexer = SearchIndexer::new(&db, &loc_map).with_entries_per_shard(2);
    indexer
        .build_and_write_index(&export_dir, std::slice::from_ref(&peer))
        .unwrap();

    let manifest_js = fs::read_to_string(export_dir.join("search/manifest.js")).unwrap();
    assert!(manifest_js.contains("prefix_index"));

    let start_idx = manifest_js.find('{').unwrap();
    let end_idx = manifest_js.rfind('}').unwrap() + 1;
    let manifest_val: serde_json::Value =
        serde_json::from_str(&manifest_js[start_idx..end_idx]).unwrap();
    let prefix_index = manifest_val["prefix_index"].as_object().unwrap();

    let shards_val = manifest_val["shards"].as_array().unwrap();
    let mut all_shards = Vec::new();
    for s in shards_val {
        let file_name = s["file_name"].as_str().unwrap();
        let shard_js =
            fs::read_to_string(export_dir.join("search/shards").join(file_name)).unwrap();
        let s_start = shard_js.find('{').unwrap();
        let s_end = shard_js.rfind('}').unwrap() + 1;
        let shard_obj: serde_json::Value = serde_json::from_str(&shard_js[s_start..s_end]).unwrap();
        all_shards.push(shard_obj);
    }

    let test_queries = vec![
        "h",
        "п",
        "к",
        "є",
        "ґ",
        "c",
        "4",
        "2",
        "he",
        "пр",
        "ки",
        "єд",
        "ґа",
        "ca",
        "20",
        "42",
        "hel",
        "при",
        "кий",
        "єдн",
        "ґан",
        "caf",
        "202",
        "hello",
        "привіт",
        "київ",
        "єдність",
        "ґанок",
        "café",
        "2024",
        "rust",
        "київ україни",
        "hello telegram",
        "year 2024",
    ];

    for q in test_queries {
        let q_tokens = tokenize_search_text(q);

        let mut candidate_shard_ids: Option<HashSet<i64>> = None;
        for token in &q_tokens {
            let mut token_matches = HashSet::new();
            let char_indices: Vec<usize> = token.char_indices().map(|(i, _)| i).collect();
            let char_count = char_indices.len();

            let lookup_prefix = if char_count > 3 {
                token[..char_indices[3]].to_string()
            } else {
                token.to_string()
            };

            if let Some(list) = prefix_index.get(&lookup_prefix) {
                for sid in list.as_array().unwrap() {
                    token_matches.insert(sid.as_i64().unwrap());
                }
            }

            if let Some(existing) = candidate_shard_ids {
                candidate_shard_ids =
                    Some(existing.intersection(&token_matches).copied().collect());
            } else {
                candidate_shard_ids = Some(token_matches);
            }
        }
        let candidates = candidate_shard_ids.unwrap_or_default();

        let mut exhaustive_matching_shards = HashSet::new();
        for shard in &all_shards {
            let s_id = shard["shard_id"].as_i64().unwrap();
            let entries = shard["entries"].as_array().unwrap();
            for e in entries {
                let text = e["text"].as_str().unwrap();
                let tokens: Vec<String> = e["tokens"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| t.as_str().unwrap().to_string())
                    .collect();
                if score_search_query(q, text, &tokens).is_some() {
                    exhaustive_matching_shards.insert(s_id);
                }
            }
        }

        for match_shard_id in &exhaustive_matching_shards {
            assert!(
                candidates.contains(match_shard_id),
                "Candidate pruning false negative for query '{q}': missing shard {match_shard_id}"
            );
        }
    }
}

#[test]
fn search_100_shards_streams_with_bounded_memory_and_eviction() {
    let (db, _db_dir) = create_test_db();
    let export_temp = tempdir().unwrap();
    let export_dir = export_temp.path().join("shards_100_export");

    let peer = PeerRecord {
        peer_id: PeerId::new(709),
        peer_type: PeerType::User,
        name: Some("Large Chat".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let mut msgs = Vec::new();
    for i in 1..=105 {
        let text = if i == 1 {
            "SpecialTarget alpha in shard 1".to_string()
        } else if i == 52 {
            "SpecialTarget beta in shard 52".to_string()
        } else if i == 105 {
            "SpecialTarget gamma in shard 105".to_string()
        } else {
            format!("Routine message payload number {i}")
        };

        msgs.push(MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new(i)),
            date: 1700000000 + (i * 10),
            sender_id: Some(peer.peer_id),
            text: Some(text),
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
        });
    }
    db.insert_messages_batch(&msgs).unwrap();

    HtmlArchiveExporter::new(
        &db,
        ExportOptions {
            output_dir: export_dir.clone(),
            replace: true,
            ..Default::default()
        },
    )
    .export()
    .unwrap();

    let loc_map = ReplyLocationMap::new();
    let indexer = SearchIndexer::new(&db, &loc_map).with_entries_per_shard(1);
    let total = indexer
        .build_and_write_index(&export_dir, std::slice::from_ref(&peer))
        .unwrap();
    assert_eq!(total, 105);

    let verifier = HtmlArchiveVerifier::new(&export_dir);
    assert!(verifier.verify().is_ok());

    let js_file = fs::read_to_string(export_dir.join("assets/js/search.js")).unwrap();
    assert!(js_file.contains("insertTopMatch"));
    assert!(js_file.contains("compareMatches"));
    assert!(js_file.contains("maxCachedShards: 10"));

    let mut collector = BoundedTopResults::new(50);
    let mut loaded_shards_in_simulated_ram = VecDeque::new();
    const MAX_CACHED_SHARDS: usize = 10;

    for shard_id in 1..=105 {
        let shard_file = export_dir
            .join("search/shards")
            .join(format!("shard_{shard_id:05}.js"));
        let content = fs::read_to_string(&shard_file).unwrap();
        let s_start = content.find('{').unwrap();
        let s_end = content.rfind('}').unwrap() + 1;
        let shard_obj: serde_json::Value = serde_json::from_str(&content[s_start..s_end]).unwrap();

        loaded_shards_in_simulated_ram.push_back(shard_id);
        if loaded_shards_in_simulated_ram.len() > MAX_CACHED_SHARDS {
            loaded_shards_in_simulated_ram.pop_front();
        }

        for e in shard_obj["entries"].as_array().unwrap() {
            let text = e["text"].as_str().unwrap();
            let tokens: Vec<String> = e["tokens"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t.as_str().unwrap().to_string())
                .collect();
            if let Some(score) = score_search_query("SpecialTarget", text, &tokens) {
                collector.insert(BoundedSearchResult {
                    score,
                    date: e["date"].as_i64().unwrap(),
                    peer_id: e["peer_id"].as_i64().unwrap(),
                    msg_id: e["msg_id"].as_i64().unwrap(),
                    entry_id: format!("shard_{shard_id}"),
                });
            }
        }
    }

    assert!(loaded_shards_in_simulated_ram.len() <= MAX_CACHED_SHARDS);

    let results = collector.results();
    let entry_ids: Vec<&str> = results.iter().map(|r| r.entry_id.as_str()).collect();
    assert!(entry_ids.contains(&"shard_1"));
    assert!(entry_ids.contains(&"shard_52"));
    assert!(entry_ids.contains(&"shard_105"));
}

#[test]
fn theme_css_includes_custom_properties_and_dark_mode() {
    assert!(THEME_CSS.contains(":root,"));
    assert!(THEME_CSS.contains("[data-theme=\"light\"]"));
    assert!(THEME_CSS.contains("--bg-primary: #ffffff;"));
    assert!(THEME_CSS.contains("--bg-secondary: #f4f4f5;"));
    assert!(THEME_CSS.contains("--bg-sidebar: #f8f9fa;"));
    assert!(THEME_CSS.contains("--bg-bubble-in: #ffffff;"));
    assert!(THEME_CSS.contains("--bg-bubble-out: #effdde;"));

    assert!(THEME_CSS.contains("[data-theme=\"dark\"] {"));
    assert!(THEME_CSS.contains("--bg-primary: #18181b;"));
    assert!(THEME_CSS.contains("--bg-secondary: #09090b;"));
    assert!(THEME_CSS.contains("--bg-sidebar: #121215;"));
    assert!(THEME_CSS.contains("--bg-bubble-in: #27272a;"));
    assert!(THEME_CSS.contains("--bg-bubble-out: #2b5278;"));

    assert!(!THEME_CSS.contains("[data-theme=\"dark\"], @media"));
    assert!(THEME_CSS.contains("@media (prefers-color-scheme: dark) {"));
}

#[test]
fn html_theme_initialization_and_toggle_integrate_e2e() {
    let (db, _dir) = create_test_db();
    let export_dir = tempdir().unwrap();

    let peer = PeerRecord {
        peer_id: PeerId::new(5001),
        peer_type: PeerType::User,
        name: Some("Theme Test User".to_string()),
        username: Some("themetest".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1000,
    };
    db.upsert_peer(&peer).unwrap();

    let msg = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(1)),
        date: 1700000000,
        sender_id: Some(peer.peer_id),
        text: Some("Theme test message".to_string()),
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
    db.insert_messages_batch(&[msg]).unwrap();

    let options = ExportOptions {
        output_dir: export_dir.path().join("export"),
        replace: true,
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.messages_count, 1);

    let index_html = fs::read_to_string(export_dir.path().join("export/index.html")).unwrap();
    assert!(index_html.contains("id=\"theme-toggle\""));
    assert!(index_html.contains("<script>"));
    assert!(index_html.contains("localStorage.getItem('vendetta-theme')"));
    assert!(index_html.contains("document.documentElement.setAttribute('data-theme', theme)"));
    assert!(index_html.contains("<symbol id=\"icon-moon\""));
    assert!(index_html.contains("<symbol id=\"icon-sun\""));

    let chat_page = fs::read_to_string(
        export_dir
            .path()
            .join("export/chats/p_5001/page_00001.html"),
    )
    .unwrap();
    assert!(chat_page.contains("id=\"theme-toggle\""));
    assert!(chat_page.contains("localStorage.getItem('vendetta-theme')"));
    assert!(chat_page.contains("document.documentElement.setAttribute('data-theme', theme)"));

    let app_js = fs::read_to_string(export_dir.path().join("export/assets/js/app.js")).unwrap();
    assert!(app_js.contains("function initTheme()"));
    assert!(app_js.contains("document.getElementById('theme-toggle')"));
    assert!(app_js.contains("localStorage.setItem('vendetta-theme', next)"));
    assert!(app_js.contains("#icon-sun"));
    assert!(app_js.contains("#icon-moon"));
    assert!(app_js.contains("function initBlockquotes()"));
    assert!(app_js.contains("initBlockquotes();"));
}

#[test]
fn telegram_reply_quote_renders_with_accent_bar() {
    let (db, _dir) = create_test_db();
    let export_dir = tempdir().unwrap();

    let peer1 = PeerRecord {
        peer_id: PeerId::new(6001),
        peer_type: PeerType::Group,
        name: Some("Test Group".to_string()),
        username: Some("testgroup".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1000,
    };
    let peer2 = PeerRecord {
        peer_id: PeerId::new(6002),
        peer_type: PeerType::User,
        name: Some("Alice Author".to_string()),
        username: Some("alice".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1000,
    };
    let peer3 = PeerRecord {
        peer_id: PeerId::new(6003),
        peer_type: PeerType::User,
        name: Some("Bob Sender".to_string()),
        username: Some("bob".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1000,
    };
    db.upsert_peer(&peer1).unwrap();
    db.upsert_peer(&peer2).unwrap();
    db.upsert_peer(&peer3).unwrap();

    let m1 = MessageRecord {
        key: MessageKey::new(peer1.peer_id, MessageId::new(10)),
        date: 1700000000,
        sender_id: Some(peer2.peer_id),
        text: Some("Original message from Alice with detailed content that might be long enough to test snippet truncation accurately across multiple lines of text.".to_string()),
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

    let m2 = MessageRecord {
        key: MessageKey::new(peer1.peer_id, MessageId::new(11)),
        date: 1700000010,
        sender_id: Some(peer3.peer_id),
        text: Some("Reply from Bob to Alice".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: Some(MessageId::new(10)),
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: None,
        views: None,
        forwards_count: None,
        raw_tl: None,
    };

    let m3 = MessageRecord {
        key: MessageKey::new(peer1.peer_id, MessageId::new(12)),
        date: 1700000020,
        sender_id: Some(peer3.peer_id),
        text: Some("Reply to deleted / missing target".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: Some(MessageId::new(9999)),
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: None,
        views: None,
        forwards_count: None,
        raw_tl: None,
    };

    db.insert_messages_batch(&[m1, m2, m3]).unwrap();

    let options = ExportOptions {
        output_dir: export_dir.path().join("export"),
        replace: true,
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.messages_count, 3);

    let path_candidate1 = export_dir
        .path()
        .join("export/chats/p_neg_1000000006001/page_00001.html");
    let target_path = if path_candidate1.is_file() {
        path_candidate1
    } else {
        export_dir
            .path()
            .join("export/chats/p_6001/page_00001.html")
    };
    let chat_page = fs::read_to_string(target_path).unwrap();

    assert!(chat_page.contains("class=\"msg-reply-preview reply-card\""));
    assert!(chat_page.contains("class=\"reply-accent-bar\""));
    assert!(chat_page.contains("class=\"reply-sender\">Alice Author</span>"));
    assert!(chat_page.contains("class=\"reply-snippet\">Original message from Alice"));
    assert!(chat_page.contains("#m-p_6001-10"));

    assert!(chat_page.contains("<div class=\"message-text\">Reply from Bob to Alice</div>"));

    assert!(chat_page.contains("reply-unlinked"));
    assert!(chat_page.contains("[Unavailable]"));
    assert!(chat_page.contains("[Original message unavailable]"));
}

#[test]
fn telegram_blockquote_and_collapsed_quote_render_cleanly() {
    let text = "First line\nQuote line 1\nQuote line 2\nLast line";
    let entities_json = r#"[{"Blockquote":{"collapsed":true,"offset":11,"length":25}}]"#;

    let rendered = render_formatted_text(text, Some(entities_json));
    assert!(
        rendered.contains(
            "<blockquote class=\"tg-blockquote tg-blockquote-collapsed\" data-collapsed=\"true\">"
        ),
        "Rendered HTML must include collapsed blockquote markup: {rendered}"
    );
    assert!(rendered.contains("Quote line 1<br>Quote line 2"));
    assert!(rendered.contains("</blockquote>"));
}

#[test]
fn real_avatar_renders_with_initials_fallback() {
    let (db, _dir) = create_test_db();
    let export_dir = tempdir().unwrap();
    let media_src_dir = tempdir().unwrap();

    let avatars_dir = media_src_dir.path().join("media/avatars");
    fs::create_dir_all(&avatars_dir).unwrap();
    fs::write(avatars_dir.join("p_7001.jpg"), b"FAKE_JPEG_DATA_PEER_7001").unwrap();

    let peer1 = PeerRecord {
        peer_id: PeerId::new(7001),
        peer_type: PeerType::Group,
        name: Some("Alice Group".to_string()),
        username: Some("alice_group".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1000,
    };
    let peer2 = PeerRecord {
        peer_id: PeerId::new(7002),
        peer_type: PeerType::User,
        name: Some("Bob User".to_string()),
        username: Some("bob_user".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1000,
    };

    db.upsert_peer(&peer1).unwrap();
    db.upsert_peer(&peer2).unwrap();

    let m1 = MessageRecord {
        key: MessageKey::new(PeerId::new(7001), MessageId::new(1)),
        sender_id: Some(PeerId::new(7001)),
        date: 1000,
        text: Some("Message from Alice Group".to_string()),
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
    let m2 = MessageRecord {
        key: MessageKey::new(PeerId::new(7001), MessageId::new(2)),
        sender_id: Some(PeerId::new(7002)),
        date: 1005,
        text: Some("Message from Bob User".to_string()),
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
    let m3_out = MessageRecord {
        key: MessageKey::new(PeerId::new(7001), MessageId::new(3)),
        sender_id: None,
        date: 1010,
        text: Some("Outgoing message from me".to_string()),
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

    db.insert_messages_batch(&[m1, m2, m3_out]).unwrap();

    let options = ExportOptions {
        output_dir: export_dir.path().join("export"),
        media_src_dir: Some(media_src_dir.path().to_path_buf()),
        replace: true,
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.messages_count, 3);

    let exported_avatar = export_dir.path().join("export/media/avatars/p_7001.jpg");
    assert!(exported_avatar.is_file());
    assert_eq!(
        fs::read(&exported_avatar).unwrap(),
        b"FAKE_JPEG_DATA_PEER_7001"
    );

    let index_html = fs::read_to_string(export_dir.path().join("export/index.html")).unwrap();
    assert!(index_html.contains(
        "<img src=\"media/avatars/p_7001.jpg\" alt=\"Alice Group\" class=\"avatar-img\">"
    ));

    let chat_page = fs::read_to_string(
        export_dir
            .path()
            .join("export/chats/p_7001/page_00001.html"),
    )
    .unwrap();
    assert!(chat_page.contains(
        "<img src=\"../../media/avatars/p_7001.jpg\" alt=\"Alice Group\" class=\"avatar-img\">"
    ));
    assert!(chat_page.contains("<span class=\"avatar-text\">B</span>"));
}

#[test]
fn export_html_media_link_mode_verifies_links() {
    let (db, _db_dir) = create_test_db();
    let export_dir = tempdir().unwrap();
    let media_src = tempdir().unwrap();

    let peer = PeerRecord {
        peer_id: PeerId::new(88001),
        peer_type: PeerType::User,
        name: Some("Link User".to_string()),
        username: Some("link_user".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let msg = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(1)),
        date: 1700000010,
        sender_id: Some(peer.peer_id),
        state: MessageState::Active,
        text: Some("Here is a linked photo".to_string()),
        entities_json: None,
        edit_date: None,
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
    db.insert_messages_batch(&[msg]).unwrap();

    let rel_path = "photos/88abcdef01234567.jpg".to_string();
    let disk_path = media_src.path().join("media").join(&rel_path);
    fs::create_dir_all(disk_path.parent().unwrap()).unwrap();
    fs::write(&disk_path, b"FAKE_PHOTO_DATA_LINK_MODE").unwrap();

    let media = MediaRecord {
        media_id: "media_link_88001".to_string(),
        kind: MediaKind::Photo,
        mime_type: Some("image/jpeg".to_string()),
        size_bytes: Some(25),
        file_name: Some("photo.jpg".to_string()),
        size_type: None,
        width: Some(800),
        height: Some(600),
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(rel_path),
        sha256: Some("88abcdef01234567".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 25,
        chunk_size: 1024,
        retry_count: 0,
        max_retries: 3,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: None,
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Unverified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&media).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: MessageKey::new(peer.peer_id, MessageId::new(1)),
        media_id: media.media_id.clone(),
        role: MediaRole::Attachment,
        position: 0,
    })
    .unwrap();

    let out_path = export_dir.path().join("html_out");
    let options = ExportOptions {
        output_dir: out_path.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        media_mode: MediaMode::Link,
        media_src_dir: Some(media_src.path().to_path_buf()),
        replace: true,
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.messages_count, 1);
    assert_eq!(summary.media_copied_count, 1);

    let verifier = HtmlArchiveVerifier::new(&out_path);
    let report = verifier.verify().unwrap();
    assert_eq!(report.errors.len(), 0);
}

#[test]
fn forwarded_message_renders_provenance_and_media() {
    let export_dir = tempdir().unwrap();
    let media_src = tempdir().unwrap();
    let (db, _db_dir) = create_test_db();

    let peer_a = PeerRecord {
        peer_id: PeerId::new(-1_003_563_998_964),
        peer_type: PeerType::Channel,
        name: Some("Main Export Group".to_string()),
        username: Some("mainexport".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer_a).unwrap();

    let peer_b = PeerRecord {
        peer_id: PeerId::new(-1_001_234_567_890),
        peer_type: PeerType::Channel,
        name: Some("Source News Channel".to_string()),
        username: Some("sourcenews".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer_b).unwrap();

    let avatars_dir = media_src.path().join("avatars");
    fs::create_dir_all(&avatars_dir).unwrap();
    fs::write(avatars_dir.join("p_neg_1001234567890.jpg"), b"avatar_bytes").unwrap();

    let rel_path = "01/0123456789abcdef.jpg".to_string();
    let disk_path = media_src.path().join("media").join(&rel_path);
    fs::create_dir_all(disk_path.parent().unwrap()).unwrap();
    fs::write(&disk_path, b"test_photo_bytes").unwrap();

    let media_rec = MediaRecord {
        media_id: "media_photo_01".to_string(),
        kind: MediaKind::Photo,
        mime_type: Some("image/jpeg".to_string()),
        size_bytes: Some(16),
        file_name: Some("photo.jpg".to_string()),
        size_type: None,
        width: Some(800),
        height: Some(600),
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(rel_path),
        sha256: Some("0123456789abcdef".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 16,
        chunk_size: 1024,
        retry_count: 0,
        max_retries: 3,
        next_retry_at: None,
        claimed_at: None,
        worker_id: None,
        last_error: None,
        filter_decision: None,
        filter_reason: None,
        policy_version: 1,
        verification_status: MediaVerificationStatus::Verified,
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&media_rec).unwrap();

    let fwd_json_1 = r#"{"from_id":{"channel_id":1234567890},"channel_post":54321,"date":1787758000,"post_author":"Chief Editor"}"#;
    let msg1 = MessageRecord {
        key: MessageKey::new(peer_a.peer_id, MessageId::new(101)),
        sender_id: Some(PeerId::new(55555)),
        date: 1787758100,
        text: Some("Breaking news with photo".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: Some(fwd_json_1.to_string()),
        reactions_json: None,
        views: Some(120),
        forwards_count: Some(5),
        raw_tl: None,
    };

    let fwd_json_2 = r#"{"from_id":{"channel_id":999999999},"date":1787759000}"#;
    let msg2 = MessageRecord {
        key: MessageKey::new(peer_a.peer_id, MessageId::new(102)),
        sender_id: Some(PeerId::new(55555)),
        date: 1787759100,
        text: Some("Forwarded from unknown channel".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: Some(fwd_json_2.to_string()),
        reactions_json: None,
        views: None,
        forwards_count: None,
        raw_tl: None,
    };

    let fwd_json_3 = r#"{"from_name":"Anonymous Whistleblower","date":1787760000}"#;
    let msg3 = MessageRecord {
        key: MessageKey::new(peer_a.peer_id, MessageId::new(103)),
        sender_id: Some(PeerId::new(55555)),
        date: 1787760100,
        text: Some("Forwarded from anonymous".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: Some(fwd_json_3.to_string()),
        reactions_json: None,
        views: None,
        forwards_count: None,
        raw_tl: None,
    };

    let msg4 = MessageRecord {
        key: MessageKey::new(peer_a.peer_id, MessageId::new(104)),
        sender_id: Some(PeerId::new(55555)),
        date: 1787761100,
        text: Some("Corrupted forward header".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: Some("{}".to_string()),
        reactions_json: None,
        views: None,
        forwards_count: None,
        raw_tl: None,
    };

    db.insert_messages_batch(&[msg1, msg2, msg3, msg4]).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: MessageKey::new(peer_a.peer_id, MessageId::new(101)),
        media_id: media_rec.media_id.clone(),
        role: MediaRole::Attachment,
        position: 0,
    })
    .unwrap();

    let out_path = export_dir.path().join("html_h20");
    let options = ExportOptions {
        output_dir: out_path.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        media_mode: MediaMode::Link,
        media_src_dir: Some(media_src.path().to_path_buf()),
        target_peers: Some(vec![peer_a.peer_id]),
        replace: true,
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.messages_count, 4);
    assert_eq!(summary.media_copied_count, 2);

    let chat_page = fs::read_to_string(out_path.join(format!(
        "chats/{}/page_00001.html",
        ArchiveUrlBuilder::peer_token(peer_a.peer_id)
    )))
    .unwrap();

    assert!(chat_page.contains("Source News Channel"));
    assert!(chat_page.contains("(@sourcenews)"));
    assert!(chat_page.contains("(Chief Editor)"));
    assert!(chat_page.contains("ID: -1001234567890"));
    assert!(chat_page.contains("Msg: #54321"));
    assert!(chat_page.contains("p_neg_1001234567890.jpg"));
    assert!(chat_page.contains("0123456789abcdef.jpg"));
    assert!(!chat_page.contains("href=\"../p_neg_1001234567890/page_00001.html\""));
    assert!(chat_page.contains("<strong class=\"fwd-origin\">Source News Channel</strong>"));
    assert!(chat_page.contains("channel -1000999999999"));
    assert!(chat_page.contains("ID: -1000999999999"));
    assert!(chat_page.contains("Anonymous Whistleblower"));
    assert!(chat_page.contains("unavailable source"));

    let verifier = HtmlArchiveVerifier::new(&out_path);
    let report = verifier.verify().unwrap();
    assert_eq!(report.errors.len(), 0);

    let msg_b = MessageRecord {
        key: MessageKey::new(peer_b.peer_id, MessageId::new(54321)),
        sender_id: Some(peer_b.peer_id),
        date: 1787758000,
        text: Some("Original post in Source News Channel".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: None,
        views: Some(500),
        forwards_count: Some(20),
        raw_tl: None,
    };
    db.insert_messages_batch(&[msg_b]).unwrap();

    let out_path_all = export_dir.path().join("html_h20_all");
    let options_all = ExportOptions {
        output_dir: out_path_all.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        media_mode: MediaMode::Link,
        media_src_dir: Some(media_src.path().to_path_buf()),
        target_peers: None,
        replace: true,
        ..Default::default()
    };

    let exporter_all = HtmlArchiveExporter::new(&db, options_all);
    let summary_all = exporter_all.export().unwrap();
    assert_eq!(summary_all.messages_count, 5);

    let chat_page_all = fs::read_to_string(out_path_all.join(format!(
        "chats/{}/page_00001.html",
        ArchiveUrlBuilder::peer_token(peer_a.peer_id)
    )))
    .unwrap();

    assert!(chat_page_all.contains("<a href=\"../../chats/p_neg_1001234567890/page_00001.html\" class=\"fwd-origin\">Source News Channel</a>"));

    let verifier_all = HtmlArchiveVerifier::new(&out_path_all);
    let report_all = verifier_all.verify().unwrap();
    assert_eq!(report_all.errors.len(), 0);
}

#[test]
fn verifier_allows_tg_custom_schemes_and_external_links() {
    let export_dir = tempdir().unwrap();
    let (db, _db_dir) = create_test_db();

    let peer = PeerRecord {
        peer_id: PeerId::new(99001),
        peer_type: PeerType::User,
        name: Some("Custom Scheme User".to_string()),
        username: Some("scheme_user".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let text = "Check out user profile, open message, ton wallet, and blocked url";
    let entities = vec![
        tl::enums::MessageEntity::TextUrl(tl::types::MessageEntityTextUrl {
            offset: 10,
            length: 12,
            url: "tg://user?id=7014130709".to_string(),
        }),
        tl::enums::MessageEntity::TextUrl(tl::types::MessageEntityTextUrl {
            offset: 24,
            length: 12,
            url: "tg://openmessage?user_id=5579715320".to_string(),
        }),
        tl::enums::MessageEntity::TextUrl(tl::types::MessageEntityTextUrl {
            offset: 38,
            length: 10,
            url: "ton://transfer/EQ...".to_string(),
        }),
        tl::enums::MessageEntity::TextUrl(tl::types::MessageEntityTextUrl {
            offset: 54,
            length: 11,
            url: "javascript:alert(1)".to_string(),
        }),
    ];
    let entities_json = serde_json::to_string(&entities).unwrap();

    let msg = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(1)),
        sender_id: Some(peer.peer_id),
        date: 1700000000,
        text: Some(text.to_string()),
        entities_json: Some(entities_json),
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
    db.insert_messages_batch(&[msg]).unwrap();

    let out_path = export_dir.path().join("html_custom_schemes");
    let options = ExportOptions {
        output_dir: out_path.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        replace: true,
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.messages_count, 1);

    let verifier = HtmlArchiveVerifier::new(&out_path);
    let report = verifier.verify().unwrap();
    assert_eq!(report.errors.len(), 0);
}

#[test]
fn media_grouping_and_deduplication_renders_compact_albums() {
    let export_dir = tempdir().unwrap();
    let (db, _db_dir) = create_test_db();

    let peer = PeerRecord {
        peer_id: PeerId::new(55001),
        peer_type: PeerType::Channel,
        name: Some("H22 Media Group Test".to_string()),
        username: Some("h22_test".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let media_base = export_dir.path().join("media");
    fs::create_dir_all(&media_base).unwrap();
    let vid_path = "media/main_video.mp4";
    let transcode_path = "media/transcode_720.mp4";
    let transcode_1080_path = "media/transcode_1080.mp4";
    let m3u8_path = "media/stream.m3u8";
    let sb_path = "media/storyboard.sb.jpg";
    let photo1_path = "media/photo1.jpg";
    let photo2_path = "media/photo2.jpg";
    let doc1_path = "media/document1.pdf";
    let doc2_path = "media/document2.pdf";

    fs::write(export_dir.path().join(vid_path), b"video_data").unwrap();
    fs::write(export_dir.path().join(transcode_path), b"transcode_data").unwrap();
    fs::write(
        export_dir.path().join(transcode_1080_path),
        b"transcode_1080_data",
    )
    .unwrap();
    fs::write(export_dir.path().join(m3u8_path), b"m3u8_data").unwrap();
    fs::write(export_dir.path().join(sb_path), b"storyboard_data").unwrap();
    fs::write(export_dir.path().join(photo1_path), b"photo1_data").unwrap();
    fs::write(export_dir.path().join(photo2_path), b"photo2_data").unwrap();
    fs::write(export_dir.path().join(doc1_path), b"doc1_data").unwrap();
    fs::write(export_dir.path().join(doc2_path), b"doc2_data").unwrap();

    let msg1_key = MessageKey::new(peer.peer_id, MessageId::new(104099));
    let msg1 = MessageRecord {
        key: msg1_key,
        sender_id: Some(peer.peer_id),
        date: 1700000000,
        text: Some("Video with adaptive qualities".to_string()),
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
    db.insert_messages_batch(&[msg1]).unwrap();

    let mo_main_vid = MediaRecord {
        media_id: "doc_main_vid".to_string(),
        kind: MediaKind::Video,
        mime_type: Some("video/mp4".to_string()),
        size_bytes: Some(6670923),
        file_name: None,
        size_type: None,
        width: Some(1920),
        height: Some(1080),
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(vid_path.to_string()),
        sha256: Some("sha_main_vid".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 6670923,
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
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&mo_main_vid).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: msg1_key,
        media_id: "doc_main_vid".to_string(),
        role: MediaRole::Attachment,
        position: 0,
    })
    .unwrap();

    let mo_transcode = MediaRecord {
        media_id: "doc_transcode_720".to_string(),
        kind: MediaKind::Video,
        mime_type: Some("video/mp4".to_string()),
        size_bytes: Some(1905757),
        file_name: None,
        size_type: None,
        width: Some(1280),
        height: Some(720),
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(transcode_path.to_string()),
        sha256: Some("sha_transcode".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 1905757,
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
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&mo_transcode).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: msg1_key,
        media_id: "doc_transcode_720".to_string(),
        role: MediaRole::AlternativeQuality,
        position: 1,
    })
    .unwrap();

    let mo_m3u8 = MediaRecord {
        media_id: "doc_stream_m3u8".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/x-mpegurl".to_string()),
        size_bytes: Some(463),
        file_name: Some("mtproto:5474447993602088289".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(m3u8_path.to_string()),
        sha256: Some("sha_m3u8".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 463,
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
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&mo_m3u8).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: msg1_key,
        media_id: "doc_stream_m3u8".to_string(),
        role: MediaRole::StreamingManifest,
        position: 2,
    })
    .unwrap();

    let mo_sb = MediaRecord {
        media_id: "doc_storyboard".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/x-tgstoryboard".to_string()),
        size_bytes: Some(21502),
        file_name: Some(".sb.jpg".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(sb_path.to_string()),
        sha256: Some("sha_sb".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 21502,
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
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    db.insert_or_update_media(&mo_sb).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: msg1_key,
        media_id: "doc_storyboard".to_string(),
        role: MediaRole::Storyboard,
        position: 3,
    })
    .unwrap();

    let msg2_key = MessageKey::new(peer.peer_id, MessageId::new(104100));
    let msg2 = MessageRecord {
        key: msg2_key,
        sender_id: Some(peer.peer_id),
        date: 1700000010,
        text: Some("Test 19 Video".to_string()),
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
    db.insert_messages_batch(&[msg2]).unwrap();

    let mo_primary_360 = MediaRecord {
        media_id: "doc_primary_360".to_string(),
        kind: MediaKind::Video,
        mime_type: Some("video/mp4".to_string()),
        size_bytes: Some(1000000),
        file_name: None,
        size_type: None,
        width: Some(640),
        height: Some(360),
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(vid_path.to_string()),
        sha256: Some("sha_360".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 1000000,
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
        created_at: 1700000010,
        updated_at: 1700000010,
    };
    db.insert_or_update_media(&mo_primary_360).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: msg2_key,
        media_id: "doc_primary_360".to_string(),
        role: MediaRole::Attachment,
        position: 0,
    })
    .unwrap();

    let mo_aux_1080 = MediaRecord {
        media_id: "doc_aux_1080".to_string(),
        kind: MediaKind::Video,
        mime_type: Some("video/mp4".to_string()),
        size_bytes: Some(8000000),
        file_name: None,
        size_type: None,
        width: Some(1920),
        height: Some(1080),
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(transcode_1080_path.to_string()),
        sha256: Some("sha_1080".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 8000000,
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
        created_at: 1700000010,
        updated_at: 1700000010,
    };
    db.insert_or_update_media(&mo_aux_1080).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: msg2_key,
        media_id: "doc_aux_1080".to_string(),
        role: MediaRole::AlternativeQuality,
        position: 1,
    })
    .unwrap();

    let msg3_key = MessageKey::new(peer.peer_id, MessageId::new(104101));
    let msg4_key = MessageKey::new(peer.peer_id, MessageId::new(104102));
    let msg3 = MessageRecord {
        key: msg3_key,
        sender_id: Some(peer.peer_id),
        date: 1700000020,
        text: Some("Album photo 1".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: Some(999),
        forward_json: None,
        reactions_json: None,
        views: None,
        forwards_count: None,
        raw_tl: None,
    };
    let msg4 = MessageRecord {
        key: msg4_key,
        sender_id: Some(peer.peer_id),
        date: 1700000020,
        text: Some("Album photo 2".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: Some(999),
        forward_json: None,
        reactions_json: None,
        views: None,
        forwards_count: None,
        raw_tl: None,
    };
    db.insert_messages_batch(&[msg3, msg4]).unwrap();

    let mo_p1 = MediaRecord {
        media_id: "photo_1".to_string(),
        kind: MediaKind::Photo,
        mime_type: Some("image/jpeg".to_string()),
        size_bytes: Some(50000),
        file_name: Some("photo1.jpg".to_string()),
        size_type: None,
        width: Some(800),
        height: Some(600),
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(photo1_path.to_string()),
        sha256: Some("sha_p1".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 50000,
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
        created_at: 1700000020,
        updated_at: 1700000020,
    };
    let mo_p2 = MediaRecord {
        media_id: "photo_2".to_string(),
        kind: MediaKind::Photo,
        mime_type: Some("image/jpeg".to_string()),
        size_bytes: Some(60000),
        file_name: Some("photo2.jpg".to_string()),
        size_type: None,
        width: Some(800),
        height: Some(600),
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(photo2_path.to_string()),
        sha256: Some("sha_p2".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 60000,
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
        created_at: 1700000020,
        updated_at: 1700000020,
    };
    db.insert_or_update_media(&mo_p1).unwrap();
    db.insert_or_update_media(&mo_p2).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: msg3_key,
        media_id: "photo_1".to_string(),
        role: MediaRole::Attachment,
        position: 0,
    })
    .unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: msg4_key,
        media_id: "photo_2".to_string(),
        role: MediaRole::Attachment,
        position: 0,
    })
    .unwrap();

    let msg5_key = MessageKey::new(peer.peer_id, MessageId::new(104103));
    let msg5 = MessageRecord {
        key: msg5_key,
        sender_id: Some(peer.peer_id),
        date: 1700000030,
        text: Some("Two PDF documents".to_string()),
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
    db.insert_messages_batch(&[msg5]).unwrap();

    let mo_doc1 = MediaRecord {
        media_id: "doc_1".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/pdf".to_string()),
        size_bytes: Some(12345),
        file_name: Some("report1.pdf".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(doc1_path.to_string()),
        sha256: Some("sha_d1".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 12345,
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
        created_at: 1700000030,
        updated_at: 1700000030,
    };
    let mo_doc2 = MediaRecord {
        media_id: "doc_2".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/pdf".to_string()),
        size_bytes: Some(67890),
        file_name: Some("report2.pdf".to_string()),
        size_type: None,
        width: None,
        height: None,
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(doc2_path.to_string()),
        sha256: Some("sha_d2".to_string()),
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 67890,
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
        created_at: 1700000030,
        updated_at: 1700000030,
    };
    db.insert_or_update_media(&mo_doc1).unwrap();
    db.insert_or_update_media(&mo_doc2).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: msg5_key,
        media_id: "doc_1".to_string(),
        role: MediaRole::Attachment,
        position: 0,
    })
    .unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: msg5_key,
        media_id: "doc_2".to_string(),
        role: MediaRole::Attachment,
        position: 1,
    })
    .unwrap();

    let msg6_key = MessageKey::new(peer.peer_id, MessageId::new(104104));
    let msg6 = MessageRecord {
        key: msg6_key,
        sender_id: Some(peer.peer_id),
        date: 1700000040,
        text: Some("Duplicate joins message".to_string()),
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
    db.insert_messages_batch(&[msg6]).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: msg6_key,
        media_id: "photo_1".to_string(),
        role: MediaRole::Attachment,
        position: 0,
    })
    .unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: msg6_key,
        media_id: "photo_1".to_string(),
        role: MediaRole::Attachment,
        position: 1,
    })
    .unwrap();

    let out_path = export_dir.path().join("html_h22");
    let options = ExportOptions {
        output_dir: out_path.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        media_mode: MediaMode::Link,
        media_src_dir: Some(export_dir.path().to_path_buf()),
        replace: true,
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.messages_count, 6);

    let chat_page = fs::read_to_string(out_path.join(format!(
        "chats/{}/page_00001.html",
        ArchiveUrlBuilder::peer_token(peer.peer_id)
    )))
    .unwrap();

    let m1_section = chat_page
        .split("id=\"m-p_55001-104099\"")
        .nth(1)
        .unwrap()
        .split("id=\"m-p_55001-104100\"")
        .next()
        .unwrap();

    let video_count_m1 = m1_section.matches("<video").count();
    assert_eq!(video_count_m1, 1);
    assert!(!m1_section.contains("mtproto:5474447993602088289"));
    assert!(!m1_section.contains(".sb.jpg"));
    assert!(!m1_section.contains("transcode_720.mp4"));

    let m2_section = chat_page
        .split("id=\"m-p_55001-104100\"")
        .nth(1)
        .unwrap()
        .split("id=\"m-p_55001-104101\"")
        .next()
        .unwrap();
    let video_count_m2 = m2_section.matches("<video").count();
    assert_eq!(video_count_m2, 1);
    assert!(!m2_section.contains("transcode_1080.mp4"));

    let album_section = chat_page
        .split("id=\"m-p_55001-104101\"")
        .nth(1)
        .unwrap()
        .split("id=\"m-p_55001-104103\"")
        .next()
        .unwrap();
    assert!(album_section.contains("class=\"media-album album-count-2\""));
    assert_eq!(album_section.matches("<img").count(), 2);

    let m5_section = chat_page
        .split("id=\"m-p_55001-104103\"")
        .nth(1)
        .unwrap()
        .split("id=\"m-p_55001-104104\"")
        .next()
        .unwrap();
    assert!(m5_section.contains("report1.pdf"));
    assert!(m5_section.contains("report2.pdf"));

    let m6_section = chat_page.split("id=\"m-p_55001-104104\"").nth(1).unwrap();
    assert_eq!(m6_section.matches("<img").count(), 1);

    let verifier = HtmlArchiveVerifier::new(&out_path);
    let report = verifier.verify().unwrap();
    assert_eq!(report.errors.len(), 0);
}

#[test]
fn reactions_and_reactor_list_render_interactively() {
    let (db, _db_dir) = create_test_db();
    let export_dir = tempdir().unwrap();

    let main_peer = PeerRecord {
        peer_id: PeerId::new(66001),
        peer_type: PeerType::Channel,
        name: Some("Reactions Chat".to_string()),
        username: Some("reaction_chat".to_string()),
        phone: None,
        updated_at: 1787755000,
        raw_tl: None,
    };
    db.upsert_peer(&main_peer).unwrap();

    let reactor_nikita = PeerRecord {
        peer_id: PeerId::new(77001),
        peer_type: PeerType::User,
        name: Some("Nikita".to_string()),
        username: Some("nikita".to_string()),
        phone: None,
        updated_at: 1787755000,
        raw_tl: None,
    };
    db.upsert_peer(&reactor_nikita).unwrap();

    let reactor_sonya = PeerRecord {
        peer_id: PeerId::new(77002),
        peer_type: PeerType::User,
        name: Some("Sonya".to_string()),
        username: None,
        phone: None,
        updated_at: 1787755000,
        raw_tl: None,
    };
    db.upsert_peer(&reactor_sonya).unwrap();

    let reactor_oleg = PeerRecord {
        peer_id: PeerId::new(77003),
        peer_type: PeerType::User,
        name: Some("Олег".to_string()),
        username: Some("oleg_tg".to_string()),
        phone: None,
        updated_at: 1787755000,
        raw_tl: None,
    };
    db.upsert_peer(&reactor_oleg).unwrap();

    let avatars_dir = export_dir.path().join("media").join("avatars");
    fs::create_dir_all(&avatars_dir).unwrap();
    fs::write(avatars_dir.join("p_77001.jpg"), b"fake avatar jpg").unwrap();

    let reactions_dir = export_dir.path().join("media").join("reactions");
    fs::create_dir_all(&reactions_dir).unwrap();
    fs::write(
        reactions_dir.join("5256103272296499934.webp"),
        b"fake webp custom reaction",
    )
    .unwrap();

    let msg1_reactions_json = r#"{
        "Reactions": {
            "results": [
                { "Count": { "reaction": { "Emoji": { "emoticon": "👍" } }, "count": 1, "chosen_order": 0 } }
            ],
            "recent_reactions": [
                { "Reaction": { "peer_id": { "User": { "user_id": 77001 } }, "date": 1787751000, "my": true, "reaction": { "Emoji": { "emoticon": "👍" } } } }
            ]
        }
    }"#;
    let msg1 = MessageRecord {
        key: MessageKey::new(main_peer.peer_id, MessageId::new(101)),
        date: 1787751000,
        sender_id: Some(reactor_nikita.peer_id),
        text: Some("Message with single reaction".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: Some(msg1_reactions_json.to_string()),
        views: Some(10),
        forwards_count: None,
        raw_tl: None,
    };
    db.insert_messages_batch(&[msg1]).unwrap();

    let msg2_reactions_json = r#"{
        "Reactions": {
            "results": [
                { "Count": { "reaction": { "Emoji": { "emoticon": "👍" } }, "count": 3, "chosen_order": null } },
                { "Count": { "reaction": { "Emoji": { "emoticon": "❤️" } }, "count": 5, "chosen_order": null } },
                { "Count": { "reaction": { "Emoji": { "emoticon": "🔥" } }, "count": 2, "chosen_order": null } }
            ],
            "recent_reactions": [
                { "Reaction": { "peer_id": { "User": { "user_id": 77001 } }, "date": 1787752000, "my": false, "reaction": { "Emoji": { "emoticon": "👍" } } } },
                { "Reaction": { "peer_id": { "User": { "user_id": 77002 } }, "date": 1787752001, "my": false, "reaction": { "Emoji": { "emoticon": "👍" } } } },
                { "Reaction": { "peer_id": { "User": { "user_id": 77003 } }, "date": 1787752002, "my": false, "reaction": { "Emoji": { "emoticon": "👍" } } } },
                { "Reaction": { "peer_id": { "User": { "user_id": 77002 } }, "date": 1787752003, "my": false, "reaction": { "Emoji": { "emoticon": "❤️" } } } }
            ]
        }
    }"#;
    let msg2 = MessageRecord {
        key: MessageKey::new(main_peer.peer_id, MessageId::new(102)),
        date: 1787752000,
        sender_id: Some(reactor_sonya.peer_id),
        text: Some("Message with multiple reactions".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: Some(msg2_reactions_json.to_string()),
        views: Some(25),
        forwards_count: None,
        raw_tl: None,
    };
    db.insert_messages_batch(&[msg2]).unwrap();

    let msg3_reactions_json = r#"{
        "Reactions": {
            "can_see_list": false,
            "results": [
                { "Count": { "reaction": { "Emoji": { "emoticon": "🎉" } }, "count": 10, "chosen_order": null } }
            ],
            "recent_reactions": []
        }
    }"#;
    let msg3 = MessageRecord {
        key: MessageKey::new(main_peer.peer_id, MessageId::new(103)),
        date: 1787753000,
        sender_id: Some(reactor_oleg.peer_id),
        text: Some("Message with aggregate-only reactions".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: Some(msg3_reactions_json.to_string()),
        views: Some(100),
        forwards_count: None,
        raw_tl: None,
    };
    db.insert_messages_batch(&[msg3]).unwrap();

    let msg4_reactions_json = r#"{
        "Reactions": {
            "results": [
                { "Count": { "reaction": { "CustomEmoji": { "document_id": 5256103272296499934 } }, "count": 1, "chosen_order": null } },
                { "Count": { "reaction": { "CustomEmoji": { "document_id": 9123456789012345678 } }, "count": 1, "chosen_order": null } }
            ],
            "recent_reactions": [
                { "Reaction": { "peer_id": { "User": { "user_id": 77001 } }, "date": 1787754000, "my": false, "reaction": { "CustomEmoji": { "document_id": 5256103272296499934 } } } }
            ]
        }
    }"#;
    let msg4 = MessageRecord {
        key: MessageKey::new(main_peer.peer_id, MessageId::new(104)),
        date: 1787754000,
        sender_id: Some(reactor_nikita.peer_id),
        text: Some("Message with custom reactions".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: Some(msg4_reactions_json.to_string()),
        views: Some(50),
        forwards_count: None,
        raw_tl: None,
    };
    db.insert_messages_batch(&[msg4]).unwrap();

    let out_path = export_dir.path().join("html_h23");
    let options = ExportOptions {
        output_dir: out_path.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        media_mode: MediaMode::Link,
        media_src_dir: Some(export_dir.path().to_path_buf()),
        replace: true,
        ..Default::default()
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.messages_count, 4);

    let page_path = out_path
        .join("chats")
        .join("p_66001")
        .join("page_00001.html");
    let chat_page = fs::read_to_string(&page_path).unwrap();

    let m1_section = chat_page
        .split("id=\"m-p_66001-101\"")
        .nth(1)
        .unwrap()
        .split("id=\"m-p_66001-102\"")
        .next()
        .unwrap();
    assert!(m1_section.contains("class=\"message-reactions\""));
    assert!(m1_section.contains("class=\"reaction-badge reaction-chosen\""));
    assert!(m1_section.contains("<span class=\"reaction-emoji\">👍</span>"));
    assert!(m1_section.contains("<span class=\"reaction-count\">1</span>"));
    assert!(m1_section.contains("Nikita"));
    assert!(m1_section.contains("@nikita"));
    assert!(m1_section.contains("../../media/avatars/p_77001.jpg"));

    let m2_section = chat_page
        .split("id=\"m-p_66001-102\"")
        .nth(1)
        .unwrap()
        .split("id=\"m-p_66001-103\"")
        .next()
        .unwrap();
    assert!(m2_section.contains("<span class=\"reaction-emoji\">👍</span>"));
    assert!(m2_section.contains("<span class=\"reaction-count\">3</span>"));
    assert!(m2_section.contains("<span class=\"reaction-emoji\">❤️</span>"));
    assert!(m2_section.contains("<span class=\"reaction-count\">5</span>"));
    assert!(m2_section.contains("<span class=\"reaction-emoji\">🔥</span>"));
    assert!(m2_section.contains("<span class=\"reaction-count\">2</span>"));

    assert!(m2_section.contains("Nikita"));
    assert!(m2_section.contains("Sonya"));
    assert!(m2_section.contains("Олег"));
    assert!(m2_section.contains("+ 4 more"));
    assert!(m2_section.contains("Reactor details unavailable in archive"));

    let m3_section = chat_page
        .split("id=\"m-p_66001-103\"")
        .nth(1)
        .unwrap()
        .split("id=\"m-p_66001-104\"")
        .next()
        .unwrap();
    assert!(m3_section.contains("<span class=\"reaction-emoji\">🎉</span>"));
    assert!(m3_section.contains("<span class=\"reaction-count\">10</span>"));
    assert!(m3_section.contains("Reactor details unavailable in archive"));

    let m4_section = chat_page.split("id=\"m-p_66001-104\"").nth(1).unwrap();
    assert!(m4_section.contains("../../media/reactions/5256103272296499934.webp"));
    assert!(m4_section.contains("class=\"reaction-custom-fallback\""));
    assert!(!m4_section.contains("5256103272296499934<"));
    assert!(chat_page.contains("tabindex=\"0\" role=\"button\" aria-haspopup=\"true\""));

    let verifier = HtmlArchiveVerifier::new(&out_path);
    let report = verifier.verify().unwrap();
    assert_eq!(report.errors.len(), 0);
}

#[test]
fn empty_dialogs_are_filtered_from_export() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = ArchiveDb::open(&db_path).unwrap();

    let media_dir = dir.path().join("media");
    let avatars_dir = media_dir.join("avatars");
    fs::create_dir_all(&avatars_dir).unwrap();

    let peer_a = PeerRecord {
        peer_id: PeerId::new(1001),
        peer_type: PeerType::Group,
        name: Some("Alpha Chat".to_string()),
        username: Some("alpha_chat".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1787750000,
    };
    db.upsert_peer(&peer_a).unwrap();
    fs::write(avatars_dir.join("p_1001.jpg"), b"AVATAR_A").unwrap();

    let peer_b = PeerRecord {
        peer_id: PeerId::new(1002),
        peer_type: PeerType::Channel,
        name: Some("Beta Channel".to_string()),
        username: Some("beta_channel".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1787760000,
    };
    db.upsert_peer(&peer_b).unwrap();
    fs::write(avatars_dir.join("p_1002.jpg"), b"AVATAR_B").unwrap();

    let peer_empty = PeerRecord {
        peer_id: PeerId::new(2001),
        peer_type: PeerType::User,
        name: Some("Empty Contact".to_string()),
        username: Some("empty_contact".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1787700000,
    };
    db.upsert_peer(&peer_empty).unwrap();

    let peer_reply = PeerRecord {
        peer_id: PeerId::new(3001),
        peer_type: PeerType::User,
        name: Some("Replied Author".to_string()),
        username: Some("replied_author".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1787710000,
    };
    db.upsert_peer(&peer_reply).unwrap();
    fs::write(avatars_dir.join("p_3001.jpg"), b"AVATAR_REPLY").unwrap();

    let peer_forward = PeerRecord {
        peer_id: PeerId::new(4001),
        peer_type: PeerType::Channel,
        name: Some("Forwarded Source Channel".to_string()),
        username: Some("fwd_channel".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1787720000,
    };
    db.upsert_peer(&peer_forward).unwrap();
    fs::write(avatars_dir.join("p_4001.jpg"), b"AVATAR_FWD").unwrap();

    let peer_reactor = PeerRecord {
        peer_id: PeerId::new(5001),
        peer_type: PeerType::User,
        name: Some("Reactor Nikita".to_string()),
        username: Some("nikita_reactor".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1787730000,
    };
    db.upsert_peer(&peer_reactor).unwrap();
    fs::write(avatars_dir.join("p_5001.jpg"), b"AVATAR_REACTOR").unwrap();

    let mut msgs_a = Vec::new();
    for i in 1..=5 {
        let fwd_json = if i == 2 {
            Some(
                r#"{"from_name": "Forwarded Source Channel", "from_id": 4001, "date": 1787740000}"#
                    .to_string(),
            )
        } else {
            None
        };
        msgs_a.push(MessageRecord {
            key: MessageKey::new(peer_a.peer_id, MessageId::new(i)),
            date: 1787750000i64 + i * 100,
            sender_id: Some(if i == 1 {
                peer_reply.peer_id
            } else {
                peer_a.peer_id
            }),
            text: Some(format!("Message A-{i}")),
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: if i == 3 {
                Some(MessageId::new(1))
            } else {
                None
            },
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: None,
            forward_json: fwd_json,
            reactions_json: None,
            views: None,
            forwards_count: None,
            raw_tl: None,
        });
    }
    db.insert_messages_batch(&msgs_a).unwrap();

    let mut msgs_b = Vec::new();
    for i in 1..=12 {
        let rx_json = if i == 5 {
            Some(r#"{
                "Reactions": {
                    "results": [{ "Count": { "reaction": { "Emoji": { "emoticon": "🔥" } }, "count": 1, "chosen_order": null } }],
                    "recent_reactions": [{ "Reaction": { "peer_id": { "User": { "user_id": 5001 } }, "date": 1787760000, "my": false, "reaction": { "Emoji": { "emoticon": "🔥" } } } }]
                }
            }"#.to_string())
        } else {
            None
        };
        msgs_b.push(MessageRecord {
            key: MessageKey::new(peer_b.peer_id, MessageId::new(i)),
            date: 1787760000i64 + i * 100,
            sender_id: Some(peer_b.peer_id),
            text: Some(format!("Message B-{i}")),
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: None,
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: None,
            forward_json: None,
            reactions_json: rx_json,
            views: None,
            forwards_count: None,
            raw_tl: None,
        });
    }
    db.insert_messages_batch(&msgs_b).unwrap();

    let out_dir = dir.path().join("export_default");
    let options = ExportOptions {
        output_dir: out_dir.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        media_mode: MediaMode::Copy,
        theme: vendetta_render::ThemeMode::System,
        chunk_size: 250,
        replace: true,
        media_src_dir: Some(media_dir.clone()),
        include_service_messages: true,
        include_deleted_messages: true,
        include_edit_history: true,
        build_search_index: true,
        build_date_index: true,
        target_peers: None,
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();

    assert_eq!(summary.dialogs_count, 2);

    let index_html = fs::read_to_string(out_dir.join("index.html")).unwrap();
    assert!(index_html.contains("Alpha Chat"));
    assert!(index_html.contains("Beta Channel"));
    assert!(!index_html.contains("Empty Contact"));
    assert!(!index_html.contains("Replied Author"));
    assert!(!index_html.contains("Forwarded Source Channel"));
    assert!(!index_html.contains("Reactor Nikita"));
    assert!(!index_html.contains("0 messages"));
    assert!(index_html.contains("5 messages"));
    assert!(index_html.contains("12 messages"));

    let pos_b = index_html.find("Beta Channel").unwrap();
    let pos_a = index_html.find("Alpha Chat").unwrap();
    assert!(pos_b < pos_a);

    let chat_a_page = fs::read_to_string(out_dir.join("chats/p_1001/page_00001.html")).unwrap();
    assert!(chat_a_page.contains("class=\"dialog-name\">Alpha Chat</span>"));
    assert!(chat_a_page.contains("class=\"dialog-name\">Beta Channel</span>"));
    assert!(!chat_a_page.contains("class=\"dialog-name\">Empty Contact</span>"));
    assert!(!chat_a_page.contains("class=\"dialog-name\">Reactor Nikita</span>"));
    assert!(chat_a_page.contains("Forwarded Source Channel"));
    assert!(chat_a_page.contains("../../media/avatars/p_4001.jpg"));

    let chat_b_page = fs::read_to_string(out_dir.join("chats/p_1002/page_00001.html")).unwrap();
    assert!(chat_b_page.contains("Reactor Nikita"));
    assert!(chat_b_page.contains("../../media/avatars/p_5001.jpg"));

    assert!(out_dir.join("chats/p_1001").is_dir());
    assert!(out_dir.join("chats/p_1002").is_dir());
    assert!(!out_dir.join("chats/p_2001").exists());
    assert!(!out_dir.join("chats/p_3001").exists());
    assert!(!out_dir.join("chats/p_4001").exists());
    assert!(!out_dir.join("chats/p_5001").exists());

    let verifier = HtmlArchiveVerifier::new(&out_dir);
    let report = verifier.verify().unwrap();
    assert_eq!(report.errors.len(), 0);

    let out_dir_scoped = dir.path().join("export_scoped");
    let options_scoped = ExportOptions {
        output_dir: out_dir_scoped.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        media_mode: MediaMode::Copy,
        theme: vendetta_render::ThemeMode::System,
        chunk_size: 250,
        replace: true,
        media_src_dir: Some(media_dir),
        include_service_messages: true,
        include_deleted_messages: true,
        include_edit_history: true,
        build_search_index: true,
        build_date_index: true,
        target_peers: Some(vec![PeerId::new(1001)]),
    };

    let exporter_scoped = HtmlArchiveExporter::new(&db, options_scoped);
    let summary_scoped = exporter_scoped.export().unwrap();
    assert_eq!(summary_scoped.dialogs_count, 1);
    let index_scoped = fs::read_to_string(out_dir_scoped.join("index.html")).unwrap();
    assert!(index_scoped.contains("Alpha Chat"));
    assert!(!index_scoped.contains("Beta Channel"));
}

#[test]
fn test_channel_title_and_post_rendering() {
    use grammers_tl_types::{self as tl, Serializable};
    use vendetta_model::{
        MediaDownloadStatus, MediaKind, MediaRecord, MediaRole, MediaVerificationStatus,
        MessageMediaJoin,
    };

    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = ArchiveDb::open(&db_path).unwrap();

    let media_dir = dir.path().join("media");
    let avatars_dir = media_dir.join("avatars");
    fs::create_dir_all(&avatars_dir).unwrap();

    let channel_peer_id = PeerId::new(-1003412444041);

    let chan_tl = tl::types::Channel {
        creator: false,
        left: false,
        broadcast: true,
        verified: false,
        megagroup: false,
        restricted: false,
        signatures: true,
        min: false,
        scam: false,
        has_link: false,
        has_geo: false,
        slowmode_enabled: false,
        call_active: false,
        call_not_empty: false,
        fake: false,
        gigagroup: false,
        noforwards: false,
        join_to_send: false,
        join_request: false,
        forum: false,
        stories_hidden: false,
        stories_hidden_min: false,
        stories_unavailable: false,
        signature_profiles: false,
        autotranslation: false,
        broadcast_messages_allowed: false,
        monoforum: false,
        forum_tabs: false,
        id: 3412444041,
        access_hash: Some(12345),
        title: "Ну типа....".to_string(),
        username: None,
        photo: tl::enums::ChatPhoto::Empty,
        date: 1764865000,
        restriction_reason: None,
        admin_rights: None,
        banned_rights: None,
        default_banned_rights: None,
        participants_count: Some(42),
        usernames: None,
        stories_max_id: None,
        color: None,
        profile_color: None,
        emoji_status: None,
        level: None,
        subscription_until_date: None,
        bot_verification_icon: None,
        send_paid_messages_stars: None,
        linked_monoforum_id: None,
        linked_community_id: None,
    };
    let raw_chat = tl::enums::Chat::Channel(chan_tl).to_bytes();

    let peer = PeerRecord {
        peer_id: channel_peer_id,
        peer_type: PeerType::Channel,
        name: Some("Ну типа....".to_string()),
        username: None,
        phone: None,
        raw_tl: Some(raw_chat),
        updated_at: 1787900000,
    };
    db.upsert_peer(&peer).unwrap();
    fs::write(
        avatars_dir.join("p_neg_1003412444041.jpg"),
        b"CHANNEL_AVATAR",
    )
    .unwrap();

    let srv_msg = MessageRecord {
        key: MessageKey::new(channel_peer_id, MessageId::new(1)),
        date: 1764865118,
        sender_id: None,
        text: Some("Created channel: \"Ну типа....\"".to_string()),
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
    db.insert_messages_batch(&[srv_msg]).unwrap();

    let post_msg = MessageRecord {
        key: MessageKey::new(channel_peer_id, MessageId::new(80)),
        date: 1785869329,
        sender_id: None,
        text: Some("ААААААААААААААААААААА".to_string()),
        entities_json: None,
        edit_date: Some(1785869400),
        state: MessageState::Edited,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: None,
        views: Some(14),
        forwards_count: Some(2),
        raw_tl: None,
    };
    db.insert_messages_batch(&[post_msg]).unwrap();

    let video_rel = "media/videos/lesson.mp4";
    let abs_video = media_dir.join(video_rel);
    fs::create_dir_all(abs_video.parent().unwrap()).unwrap();
    fs::write(&abs_video, b"MP4_VIDEO_BYTES").unwrap();

    let video_rec = MediaRecord {
        media_id: "doc_video_80".to_string(),
        kind: MediaKind::Video,
        mime_type: Some("video/mp4".to_string()),
        size_bytes: Some(15),
        file_name: Some("lesson.mp4".to_string()),
        size_type: None,
        width: Some(1280),
        height: Some(720),
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(video_rel.to_string()),
        sha256: None,
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 15,
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
        created_at: 1785869329,
        updated_at: 1785869329,
    };
    db.insert_or_update_media(&video_rec).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: MessageKey::new(channel_peer_id, MessageId::new(80)),
        media_id: "doc_video_80".to_string(),
        role: MediaRole::Attachment,
        position: 0,
    })
    .unwrap();

    let sticker_msg = MessageRecord {
        key: MessageKey::new(channel_peer_id, MessageId::new(93)),
        date: 1787578439,
        sender_id: None,
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
        views: Some(11),
        forwards_count: None,
        raw_tl: None,
    };
    db.insert_messages_batch(&[sticker_msg]).unwrap();

    let sticker_rel = "media/stickers/sticker.webm";
    let abs_sticker = media_dir.join(sticker_rel);
    fs::create_dir_all(abs_sticker.parent().unwrap()).unwrap();
    fs::write(&abs_sticker, b"WEBM_STICKER_BYTES").unwrap();

    let sticker_rec = MediaRecord {
        media_id: "doc_sticker_93".to_string(),
        kind: MediaKind::Sticker,
        mime_type: Some("video/webm".to_string()),
        size_bytes: Some(18),
        file_name: Some("sticker.webm".to_string()),
        size_type: None,
        width: Some(512),
        height: Some(512),
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(sticker_rel.to_string()),
        sha256: None,
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 18,
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
        created_at: 1787578439,
        updated_at: 1787578439,
    };
    db.insert_or_update_media(&sticker_rec).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: MessageKey::new(channel_peer_id, MessageId::new(93)),
        media_id: "doc_sticker_93".to_string(),
        role: MediaRole::Sticker,
        position: 0,
    })
    .unwrap();

    let tgs_msg = MessageRecord {
        key: MessageKey::new(channel_peer_id, MessageId::new(94)),
        date: 1787578500,
        sender_id: None,
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
        views: Some(15),
        forwards_count: None,
        raw_tl: None,
    };
    db.insert_messages_batch(&[tgs_msg]).unwrap();

    let tgs_rel = "media/stickers/animated.tgs";
    let abs_tgs = media_dir.join(tgs_rel);
    fs::create_dir_all(abs_tgs.parent().unwrap()).unwrap();
    fs::write(&abs_tgs, b"TGS_GZIP_BYTES").unwrap();

    let tgs_rec = MediaRecord {
        media_id: "doc_tgs_94".to_string(),
        kind: MediaKind::Sticker,
        mime_type: Some("application/x-tgsticker".to_string()),
        size_bytes: Some(14),
        file_name: Some("animated.tgs".to_string()),
        size_type: None,
        width: Some(512),
        height: Some(512),
        dc_id: 2,
        source_location_tl: None,
        file_reference: None,
        local_rel_path: Some(tgs_rel.to_string()),
        sha256: None,
        download_status: MediaDownloadStatus::Completed,
        downloaded_bytes: 14,
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
        created_at: 1787578500,
        updated_at: 1787578500,
    };
    db.insert_or_update_media(&tgs_rec).unwrap();
    db.link_message_media(&MessageMediaJoin {
        key: MessageKey::new(channel_peer_id, MessageId::new(94)),
        media_id: "doc_tgs_94".to_string(),
        role: MediaRole::Sticker,
        position: 0,
    })
    .unwrap();

    let out_dir = dir.path().join("export_html");
    let options = ExportOptions {
        output_dir: out_dir.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        media_mode: MediaMode::Copy,
        theme: vendetta_render::ThemeMode::System,
        chunk_size: 250,
        replace: true,
        media_src_dir: Some(media_dir),
        include_service_messages: true,
        include_deleted_messages: true,
        include_edit_history: true,
        build_search_index: true,
        build_date_index: true,
        target_peers: None,
    };

    let exporter = HtmlArchiveExporter::new(&db, options);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.dialogs_count, 1);

    let chat_page =
        fs::read_to_string(out_dir.join("chats/p_neg_1003412444041/page_00001.html")).unwrap();

    assert!(
        chat_page.contains("<h2>Ну типа....</h2>"),
        "Chat header must display Ну типа...."
    );
    assert!(
        chat_page.contains("<span class=\"dialog-name\">Ну типа....</span>"),
        "Sidebar must display Ну типа...."
    );
    assert!(
        !chat_page.contains("<div class=\"message-sender\">Unknown</div>"),
        "Must NOT contain Unknown senders"
    );

    assert!(
        chat_page.contains("channel-post"),
        "Post must have channel-post class"
    );
    assert!(
        chat_page.contains("ААААААААААААААААААААА"),
        "Post must render caption text"
    );
    assert!(
        chat_page.contains("class=\"message-text message-caption\""),
        "Caption must be attached"
    );
    assert!(
        chat_page.contains("<span class=\"meta-views\">👁 14</span>"),
        "Post must render views"
    );

    assert!(
        chat_page.contains("<video class=\"sticker-video\" autoplay loop muted playsinline>"),
        "WebM sticker must render as video"
    );
    assert!(
        chat_page.contains("type=\"video/webm\""),
        "WebM sticker video source must have video/webm type"
    );
    assert!(
        chat_page.contains("msg-sticker"),
        "Sticker message must have msg-sticker class"
    );

    assert!(
        chat_page.contains("<canvas class=\"sticker-canvas\" data-tgs-url="),
        "TGS sticker must render as canvas"
    );

    let verifier = HtmlArchiveVerifier::new(&out_dir);
    let report = verifier.verify().unwrap();
    assert_eq!(
        report.errors.len(),
        0,
        "HTML verification must pass with 0 errors"
    );
}

#[test]
fn test_multiple_reactions() {
    use vendetta_render::message::reactions::render_message_reactions;
    use vendetta_render::model::{RenderReactionGroup, RenderReactionKey};

    let groups = vec![
        RenderReactionGroup {
            reaction: RenderReactionKey::Emoji("👍".to_string()),
            count: 3,
            is_chosen_by_me: false,
            reactors: vec![],
        },
        RenderReactionGroup {
            reaction: RenderReactionKey::Emoji("❤️".to_string()),
            count: 2,
            is_chosen_by_me: false,
            reactors: vec![],
        },
        RenderReactionGroup {
            reaction: RenderReactionKey::Emoji("🔥".to_string()),
            count: 1,
            is_chosen_by_me: false,
            reactors: vec![],
        },
    ];

    let html = render_message_reactions(&groups);
    let badge_count = html.matches("class=\"reaction-badge").count();
    assert_eq!(
        badge_count, 3,
        "Must render exactly 3 distinct reaction badges"
    );
    assert!(html.contains("👍"), "Must contain 👍");
    assert!(html.contains("❤️"), "Must contain ❤️");
    assert!(html.contains("🔥"), "Must contain 🔥");
    assert!(
        !html.contains("👍❤️🔥"),
        "Must not concatenate emojis into one text node"
    );
}

#[test]
fn test_zwj_reactions() {
    use vendetta_render::message::reactions::render_message_reactions;
    use vendetta_render::model::{RenderReactionGroup, RenderReactionKey};

    let groups = vec![
        RenderReactionGroup {
            reaction: RenderReactionKey::Emoji("❤‍🔥".to_string()),
            count: 2,
            is_chosen_by_me: false,
            reactors: vec![],
        },
        RenderReactionGroup {
            reaction: RenderReactionKey::Emoji("🔥".to_string()),
            count: 1,
            is_chosen_by_me: false,
            reactors: vec![],
        },
    ];

    let html = render_message_reactions(&groups);
    let badge_count = html.matches("class=\"reaction-badge").count();
    assert_eq!(
        badge_count, 2,
        "Must render exactly 2 distinct reaction badges for ZWJ and standard emoji"
    );
    assert!(
        html.contains("<span class=\"reaction-emoji\">\u{2764}\u{fe0f}\u{200d}\u{1f525}</span>")
    );
    assert!(html.contains("<span class=\"reaction-emoji\">🔥</span>"));
}

#[test]
fn test_custom_emoji_f() {
    use vendetta_render::message::reactions::render_message_reactions;
    use vendetta_render::model::{RenderReactionGroup, RenderReactionKey};

    let groups = vec![
        RenderReactionGroup {
            reaction: RenderReactionKey::CustomEmoji {
                document_id: 123456789,
                alt_text: Some("Cool Custom".to_string()),
                asset_rel_path: Some("../../media/reactions/123456789.webp".to_string()),
            },
            count: 4,
            is_chosen_by_me: false,
            reactors: vec![],
        },
        RenderReactionGroup {
            reaction: RenderReactionKey::CustomEmoji {
                document_id: 987654321,
                alt_text: None,
                asset_rel_path: None,
            },
            count: 1,
            is_chosen_by_me: false,
            reactors: vec![],
        },
    ];

    let html = render_message_reactions(&groups);
    assert!(
        html.contains("<img src=\"../../media/reactions/123456789.webp\" alt=\"Cool Custom\" class=\"reaction-custom-icon\" loading=\"lazy\">"),
        "Custom reaction with asset must render img tag"
    );
    assert!(
        html.contains("<span class=\"reaction-custom-fallback\" title=\"Custom reaction #987654321\">✨</span>"),
        "Custom reaction without asset must render truthful fallback"
    );
    assert!(
        !html.contains("987654321</span>"),
        "Must not render raw document ID as text"
    );
}

#[test]
fn test_per_reaction_count() {
    use vendetta_render::message::reactions::render_message_reactions;
    use vendetta_render::model::{RenderReactionGroup, RenderReactionKey};

    let groups = vec![
        RenderReactionGroup {
            reaction: RenderReactionKey::Paid,
            count: 7,
            is_chosen_by_me: false,
            reactors: vec![],
        },
        RenderReactionGroup {
            reaction: RenderReactionKey::Emoji("🍓".to_string()),
            count: 42,
            is_chosen_by_me: false,
            reactors: vec![],
        },
    ];

    let html = render_message_reactions(&groups);
    assert!(html.contains("<span class=\"reaction-count\">7</span>"));
    assert!(html.contains("<span class=\"reaction-count\">42</span>"));
    assert!(html.contains("⭐"));
    assert!(!html.contains("Paid</span>"));
}
