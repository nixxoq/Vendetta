use rusqlite::Connection;
use vendetta_model::{
    FilterDecision, MediaDownloadStatus, MediaKind, MediaRecord, MediaRole,
    MediaVerificationStatus, MessageId, MessageKey, MessageMediaJoin, MessageRecord,
    MessageReplyRecord, MessageState, PeerId, PeerRecord, PeerType, ReplyResolutionStatus,
    SyncStateRecord,
};
use vendetta_storage::{ArchiveDb, run_migrations};

#[test]
fn migrations_are_strictly_idempotent() {
    let db = ArchiveDb::open_in_memory().expect("failed to create in-memory db");
    let applied = db
        .with_conn(run_migrations)
        .expect("second migration run failed");
    assert_eq!(
        applied, 0,
        "No new migrations should be applied on second run"
    );
}

#[test]
fn composite_canonical_key_prevents_cross_peer_collisions() {
    let db = ArchiveDb::open_in_memory().expect("failed to create in-memory db");

    let peer_user = PeerId::new(1001);
    let peer_group = PeerId::new(-1002);
    let peer_channel = PeerId::new(-1003);
    let msg_id = MessageId::new(42);

    let msg1 = MessageRecord {
        key: MessageKey::new(peer_user, msg_id),
        date: 1700000001,
        sender_id: Some(peer_user),
        text: Some("User message 42".to_string()),
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
        raw_tl: Some(vec![1, 2, 3]),
    };

    let msg2 = MessageRecord {
        key: MessageKey::new(peer_group, msg_id),
        date: 1700000002,
        sender_id: Some(peer_user),
        text: Some("Group message 42".to_string()),
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
        raw_tl: Some(vec![4, 5, 6]),
    };

    let msg3 = MessageRecord {
        key: MessageKey::new(peer_channel, msg_id),
        date: 1700000003,
        sender_id: Some(peer_channel),
        text: Some("Channel message 42".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: None,
        views: Some(100),
        forwards_count: Some(5),
        raw_tl: Some(vec![7, 8, 9]),
    };

    let count = db
        .insert_messages_batch(&[msg1.clone(), msg2.clone(), msg3.clone()])
        .expect("batch insert failed");
    assert_eq!(count, 3);

    let res1 = db
        .get_message(MessageKey::new(peer_user, msg_id))
        .expect("query failed")
        .expect("msg1 not found");
    let res2 = db
        .get_message(MessageKey::new(peer_group, msg_id))
        .expect("query failed")
        .expect("msg2 not found");
    let res3 = db
        .get_message(MessageKey::new(peer_channel, msg_id))
        .expect("query failed")
        .expect("msg3 not found");

    assert_eq!(res1.text.as_deref(), Some("User message 42"));
    assert_eq!(res2.text.as_deref(), Some("Group message 42"));
    assert_eq!(res3.text.as_deref(), Some("Channel message 42"));
}

#[test]
fn message_edits_append_to_revision_history() {
    let db = ArchiveDb::open_in_memory().expect("failed to create in-memory db");
    let key = MessageKey::new(1001, 10);

    let initial = MessageRecord {
        key,
        date: 1700000000,
        sender_id: Some(PeerId::new(1001)),
        text: Some("Initial text".to_string()),
        entities_json: Some(r#"[{"type":"bold","offset":0,"length":7}]"#.to_string()),
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
        raw_tl: Some(vec![0xAA, 0xBB]),
    };

    db.insert_or_update_message(&initial)
        .expect("initial insert failed");

    let mut edit1 = initial.clone();
    edit1.text = Some("First edit text".to_string());
    edit1.edit_date = Some(1700000100);
    edit1.state = MessageState::Edited;
    edit1.raw_tl = Some(vec![0xAA, 0xBC]);

    db.insert_or_update_message(&edit1)
        .expect("first edit failed");

    let mut edit2 = edit1.clone();
    edit2.text = Some("Second edit text".to_string());
    edit2.edit_date = Some(1700000200);
    edit2.raw_tl = Some(vec![0xAA, 0xBD]);

    db.insert_or_update_message(&edit2)
        .expect("second edit failed");

    let current = db
        .get_message(key)
        .expect("query failed")
        .expect("message not found");
    assert_eq!(current.text.as_deref(), Some("Second edit text"));
    assert_eq!(current.edit_date, Some(1700000200));

    let revisions = db
        .list_message_revisions(key)
        .expect("revision query failed");
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0].text.as_deref(), Some("Initial text"));
    assert_eq!(revisions[0].raw_tl.as_deref(), Some(&[0xAA, 0xBB][..]));
    assert_eq!(revisions[1].text.as_deref(), Some("First edit text"));
    assert_eq!(revisions[1].raw_tl.as_deref(), Some(&[0xAA, 0xBC][..]));
}

#[test]
fn message_deletion_marks_tombstone_and_preserves_content() {
    let db = ArchiveDb::open_in_memory().expect("failed to create in-memory db");
    let key = MessageKey::new(1001, 20);

    let msg = MessageRecord {
        key,
        date: 1700000000,
        sender_id: Some(PeerId::new(1001)),
        text: Some("Crucial evidence".to_string()),
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
        raw_tl: Some(vec![1, 2, 3, 4]),
    };

    db.insert_or_update_message(&msg).expect("insert failed");
    let marked = db.mark_message_deleted(key).expect("delete marker failed");
    assert!(marked);

    let retrieved = db
        .get_message(key)
        .expect("query failed")
        .expect("msg not found");
    assert_eq!(retrieved.state, MessageState::Deleted);
    assert_eq!(
        retrieved.text.as_deref(),
        Some("Crucial evidence"),
        "Deleted content must be preserved locally"
    );
}

#[test]
fn raw_tl_payload_persists_and_restores_losslessly() {
    let db = ArchiveDb::open_in_memory().expect("failed to create in-memory db");
    let key = MessageKey::new(500, 1);
    let sample_tl = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04];

    let msg = MessageRecord {
        key,
        date: 1700000000,
        sender_id: None,
        text: Some("TL payload".to_string()),
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
        raw_tl: Some(sample_tl.clone()),
    };

    db.insert_or_update_message(&msg).expect("insert failed");
    let retrieved = db.get_message(key).expect("query failed").unwrap();
    assert_eq!(retrieved.raw_tl, Some(sample_tl));
}

#[test]
fn reply_resolution_records_targets_and_statuses() {
    let db = ArchiveDb::open_in_memory().expect("failed to create in-memory db");
    let source = MessageKey::new(100, 200);
    let target = MessageKey::new(100, 1);

    let reply = MessageReplyRecord {
        source,
        target,
        top_message_id: Some(1),
        resolution_status: ReplyResolutionStatus::Resolved,
    };

    db.upsert_reply(&reply).expect("reply insert failed");

    let retrieved = db
        .get_reply(source)
        .expect("query failed")
        .expect("reply not found");
    assert_eq!(retrieved.source, source);
    assert_eq!(retrieved.target, target);
    assert_eq!(retrieved.resolution_status, ReplyResolutionStatus::Resolved);

    let replies_to_target = db.list_replies_to(target).expect("list replies failed");
    assert_eq!(replies_to_target.len(), 1);
}

#[test]
fn peer_and_sync_state_persist_and_restore() {
    let db = ArchiveDb::open_in_memory().expect("failed to create in-memory db");
    let peer = PeerRecord {
        peer_id: PeerId::new(999),
        peer_type: PeerType::User,
        name: Some("Alice".to_string()),
        username: Some("alice".to_string()),
        phone: Some("+1234567890".to_string()),
        raw_tl: None,
        updated_at: 1700000000,
    };

    db.upsert_peer(&peer).expect("peer upsert failed");
    let fetched = db
        .get_peer(PeerId::new(999))
        .expect("peer get failed")
        .expect("peer not found");
    assert_eq!(fetched.name.as_deref(), Some("Alice"));

    let sync = SyncStateRecord {
        peer_id: PeerId::new(999),
        pts: Some(100),
        qts: Some(20),
        date: Some(1700000000),
        seq: Some(5),
        min_message_id: Some(1),
        max_message_id: Some(50),
        last_synced_at: 1700000000,
    };

    db.upsert_sync_state(&sync)
        .expect("sync state upsert failed");
    let fetched_sync = db
        .get_sync_state(PeerId::new(999))
        .expect("sync get failed")
        .expect("sync not found");
    assert_eq!(fetched_sync.pts, Some(100));
}

#[test]
fn media_storage_supports_crud_and_atomic_worker_claims() {
    let db = ArchiveDb::open_in_memory().expect("failed to create in-memory db");

    let media1 = MediaRecord {
        media_id: "doc_12345".to_string(),
        kind: MediaKind::Document,
        mime_type: Some("application/pdf".to_string()),
        size_bytes: Some(1048576),
        file_name: Some("report.pdf".to_string()),
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

    db.insert_or_update_media(&media1)
        .expect("insert media failed");

    let fetched = db
        .get_media("doc_12345")
        .expect("get failed")
        .expect("missing");
    assert_eq!(fetched.kind, MediaKind::Document);
    assert_eq!(fetched.download_status, MediaDownloadStatus::Pending);

    let msg = MessageRecord {
        key: MessageKey::new(100, 1),
        date: 1700000000,
        sender_id: Some(PeerId::new(100)),
        text: Some("Here is report".to_string()),
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

    let join = MessageMediaJoin {
        key: MessageKey::new(100, 1),
        media_id: "doc_12345".to_string(),
        role: MediaRole::Attachment,
        position: 0,
    };
    db.link_message_media(&join).expect("link failed");

    let refs = db
        .get_referencing_messages_for_media("doc_12345")
        .expect("refs failed");
    assert_eq!(refs, vec![(PeerId::new(100), MessageId::new(1))]);

    let claimed = db
        .claim_next_pending_media("worker_1")
        .expect("claim failed")
        .expect("should claim item");
    assert_eq!(claimed.media_id, "doc_12345");
    assert_eq!(claimed.download_status, MediaDownloadStatus::Downloading);

    db.update_media_progress("doc_12345", 524288)
        .expect("progress failed");

    db.update_media_completed("doc_12345", "abcdef123456", "media/ab/abcdef123456")
        .expect("complete failed");
    let completed = db.get_media("doc_12345").expect("get failed").unwrap();
    assert_eq!(completed.download_status, MediaDownloadStatus::Completed);
    assert_eq!(completed.sha256.as_deref(), Some("abcdef123456"));
    assert_eq!(
        completed.verification_status,
        MediaVerificationStatus::Verified
    );

    let stats = db.get_media_stats().expect("stats failed");
    assert_eq!(stats.total_count, 1);
    assert_eq!(stats.completed_count, 1);
    assert_eq!(stats.verified_count, 1);
}

#[test]
fn migration_0003_preserves_legacy_media_and_remaps_collisions() {
    let conn = Connection::open_in_memory().expect("open failed");

    let m1 = include_str!("../../../migrations/0001_initial_schema.sql");
    let m2 = include_str!("../../../migrations/0002_incremental_sync.sql");
    let m3 = include_str!("../../../migrations/0003_media_engine.sql");

    conn.execute_batch(m1).expect("m1 failed");
    conn.execute_batch(m2).expect("m2 failed");

    conn.execute(
        "INSERT INTO messages (peer_id, message_id, date, state) VALUES (100, 1, 1700000000, 'active');",
        [],
    ).expect("msg failed");

    conn.execute(
        "INSERT INTO media (media_key, mime_type, size_bytes, local_rel_path, sha256, download_status, downloaded_bytes, verified, created_at, updated_at)
         VALUES ('doc_123', 'image/jpeg', 1000, 'legacy/doc_123.bin', 'HASH_A', 'completed', 1000, 1, 1700000000, 1700000000);",
        [],
    ).expect("legacy doc_123 failed");

    conn.execute(
        "INSERT INTO media (media_key, mime_type, size_bytes, local_rel_path, sha256, download_status, downloaded_bytes, verified, created_at, updated_at)
         VALUES ('doc_456', 'application/pdf', 2000, NULL, NULL, 'pending', 0, 0, 1700000000, 1700000000);",
        [],
    ).expect("legacy doc_456 failed");

    conn.execute(
        "INSERT INTO message_media (peer_id, message_id, media_key, position) VALUES (100, 1, 'doc_123', 0);",
        [],
    ).expect("link doc_123 failed");
    conn.execute(
        "INSERT INTO message_media (peer_id, message_id, media_key, position) VALUES (100, 1, 'doc_456', 1);",
        [],
    ).expect("link doc_456 failed");

    conn.execute(
        "CREATE TABLE IF NOT EXISTS media_objects (
            media_id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            mime_type TEXT,
            size_bytes INTEGER,
            file_name TEXT,
            size_type TEXT,
            width INTEGER,
            height INTEGER,
            dc_id INTEGER NOT NULL DEFAULT 0,
            source_location_tl BLOB,
            file_reference BLOB,
            local_rel_path TEXT,
            sha256 TEXT,
            download_status TEXT NOT NULL DEFAULT 'pending',
            downloaded_bytes INTEGER NOT NULL DEFAULT 0,
            chunk_size INTEGER NOT NULL DEFAULT 524288,
            retry_count INTEGER NOT NULL DEFAULT 0,
            max_retries INTEGER NOT NULL DEFAULT 5,
            next_retry_at INTEGER,
            claimed_at INTEGER,
            worker_id TEXT,
            last_error TEXT,
            filter_decision TEXT,
            filter_reason TEXT,
            policy_version INTEGER NOT NULL DEFAULT 1,
            verification_status TEXT NOT NULL DEFAULT 'unverified',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );",
        [],
    )
    .expect("create mo failed");

    conn.execute(
        "INSERT INTO media_objects (media_id, kind, mime_type, size_bytes, local_rel_path, sha256, download_status, verification_status, created_at, updated_at)
         VALUES ('doc_123', 'document', 'application/zip', 5000, 'media/ha/hash_b.zip', 'HASH_B', 'completed', 'verified', 1700000000, 1700000000);",
        [],
    ).expect("insert conflicting mo failed");

    conn.execute_batch(m3).expect("m3 execution failed");

    let mo_123_sha: String = conn
        .query_row(
            "SELECT sha256 FROM media_objects WHERE media_id = 'doc_123'",
            [],
            |r| r.get(0),
        )
        .expect("query doc_123");
    assert_eq!(mo_123_sha, "HASH_B");

    let mo_conflict_sha: String = conn
        .query_row(
            "SELECT sha256 FROM media_objects WHERE media_id = 'doc_123_legacy_conflict'",
            [],
            |r| r.get(0),
        )
        .expect("query doc_123_legacy_conflict");
    assert_eq!(mo_conflict_sha, "HASH_A");

    let mo_456_status: String = conn
        .query_row(
            "SELECT download_status FROM media_objects WHERE media_id = 'doc_456'",
            [],
            |r| r.get(0),
        )
        .expect("query doc_456");
    assert_eq!(mo_456_status, "pending");

    let linked_media_for_pos0: String = conn.query_row(
        "SELECT media_id FROM message_media WHERE peer_id = 100 AND message_id = 1 AND position = 0",
        [],
        |r| r.get(0),
    ).expect("query pos 0");
    assert_eq!(linked_media_for_pos0, "doc_123_legacy_conflict");

    let linked_media_for_pos1: String = conn.query_row(
        "SELECT media_id FROM message_media WHERE peer_id = 100 AND message_id = 1 AND position = 1",
        [],
        |r| r.get(0),
    ).expect("query pos 1");
    assert_eq!(linked_media_for_pos1, "doc_456");

    let fk_violations: Vec<String> = conn
        .prepare("PRAGMA foreign_key_check;")
        .expect("prep fk check")
        .query_map([], |row| {
            let table: String = row.get(0)?;
            let rowid: i64 = row.get(1)?;
            let parent: String = row.get(2)?;
            let fkid: i64 = row.get(3)?;
            Ok(format!("{table}({rowid}) -> {parent}({fkid})"))
        })
        .expect("fk query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect fk violations");
    assert!(
        fk_violations.is_empty(),
        "Expected 0 foreign key violations after migration, got: {:?}",
        fk_violations
    );

    let orphan_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM message_media mm WHERE NOT EXISTS (SELECT 1 FROM media_objects mo WHERE mo.media_id = mm.media_id)",
        [],
        |r| r.get(0),
    ).expect("orphan count query");
    assert_eq!(
        orphan_count, 0,
        "No orphan message_media foreign keys after migration"
    );

    let backup_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM legacy_media_backup", [], |r| r.get(0))
        .expect("backup count query");
    assert_eq!(backup_count, 2);
}

#[test]
fn fresh_database_applies_all_migrations_cleanly() {
    let db = ArchiveDb::open_in_memory().expect("open db failed");

    db.with_conn(|conn| {
        let fk_violations: Vec<String> = conn
            .prepare("PRAGMA foreign_key_check;")?
            .query_map([], |row| {
                let table: String = row.get(0)?;
                let rowid: i64 = row.get(1)?;
                let parent: String = row.get(2)?;
                let fkid: i64 = row.get(3)?;
                Ok(format!("{table}({rowid}) -> {parent}({fkid})"))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        assert!(
            fk_violations.is_empty(),
            "Expected 0 foreign key violations on fresh database, got: {:?}",
            fk_violations
        );
        Ok(())
    })
    .expect("fk check failed");
}
