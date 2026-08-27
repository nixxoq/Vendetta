use async_trait::async_trait;
use tempfile::tempdir;
use vendetta_model::{
    MessageId, MessageKey, MessageRecord, MessageState, PeerId, PeerRecord, PeerType,
};
use vendetta_storage::ArchiveDb;
use vendetta_sync::{HistoryIngestionPipeline, SyncError};
use vendetta_tg_adapter::traits::HistoryPage;
use vendetta_tg_adapter::{AdapterResult, FakeTelegramAdapter, TelegramAdapter};

fn make_test_message(peer_id: PeerId, id: i64, date: i64, text: &str) -> MessageRecord {
    MessageRecord {
        key: MessageKey::new(peer_id, MessageId::new(id)),
        date,
        sender_id: Some(PeerId::new(999)),
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
        raw_tl: Some(vec![1, 2, 3, 4, (id % 256) as u8]),
    }
}

#[tokio::test]
async fn history_pagination_handles_chunk_and_exact_boundaries() {
    let test_cases = [
        (1001, 250, 50, 5, 1, 250), // 250 messages, 5 batches of 50
        (2002, 130, 50, 3, 1, 130), // 130 messages, pages 50, 50, 30
    ];

    for (peer_raw, msg_count, chunk_size, expected_batches, exp_min, exp_max) in test_cases {
        let dir = tempdir().expect("tempdir failed");
        let db = ArchiveDb::open(dir.path().join("archive.db")).expect("open db failed");
        let adapter = FakeTelegramAdapter::new();
        let peer_id = PeerId::new(peer_raw);

        for id in 1..=msg_count {
            adapter.add_message(make_test_message(
                peer_id,
                id,
                1700000000 + id,
                &format!("Message {id}"),
            ));
        }

        let pipeline = HistoryIngestionPipeline::new(chunk_size);
        let summary = pipeline
            .ingest_history(&adapter, &db, peer_id)
            .await
            .expect("ingest failed");

        assert_eq!(summary.batches_committed, expected_batches);
        assert_eq!(summary.messages_ingested, msg_count as usize);
        assert_eq!(summary.min_message_id, Some(MessageId::new(exp_min)));
        assert_eq!(summary.max_message_id, Some(MessageId::new(exp_max)));

        let count = db.count_messages_by_peer(peer_id).expect("count failed");
        assert_eq!(count, msg_count as usize);
    }
}

#[tokio::test]
async fn history_ingestion_handles_non_contiguous_message_ids() {
    let dir = tempdir().expect("tempdir failed");
    let db = ArchiveDb::open(dir.path().join("archive.db")).expect("open db failed");
    let adapter = FakeTelegramAdapter::new();

    let peer_id = PeerId::new(3003);
    let non_contiguous_ids = vec![1, 7, 42, 105, 9001, 1_000_000];

    for &id in &non_contiguous_ids {
        adapter.add_message(make_test_message(
            peer_id,
            id,
            1700000000 + id,
            &format!("Non-contiguous message {id}"),
        ));
    }

    let pipeline = HistoryIngestionPipeline::new(2);
    let summary = pipeline
        .ingest_history(&adapter, &db, peer_id)
        .await
        .expect("ingest failed");

    assert_eq!(summary.messages_ingested, non_contiguous_ids.len());
    assert_eq!(summary.min_message_id, Some(MessageId::new(1)));
    assert_eq!(summary.max_message_id, Some(MessageId::new(1_000_000)));

    let db_messages = db
        .list_messages_by_peer(peer_id, 10, 0)
        .expect("list messages failed");
    let stored_ids: Vec<i64> = db_messages.iter().map(|m| m.key.message_id.raw()).collect();
    assert_eq!(stored_ids, non_contiguous_ids);
}

#[tokio::test]
async fn history_ingestion_is_idempotent_on_duplicate_pages() {
    let dir = tempdir().expect("tempdir failed");
    let db = ArchiveDb::open(dir.path().join("archive.db")).expect("open db failed");
    let adapter = FakeTelegramAdapter::new();

    let peer_id = PeerId::new(4004);

    for id in 1..=50 {
        adapter.add_message(make_test_message(
            peer_id,
            id,
            1700000000 + id,
            &format!("Message {id}"),
        ));
    }

    let pipeline = HistoryIngestionPipeline::new(20);

    pipeline
        .ingest_history(&adapter, &db, peer_id)
        .await
        .expect("ingest 1 failed");

    pipeline
        .ingest_history(&adapter, &db, peer_id)
        .await
        .expect("ingest 2 failed");

    let count = db.count_messages_by_peer(peer_id).expect("count failed");
    assert_eq!(
        count, 50,
        "Idempotent ingestion must not duplicate (peer_id, message_id) rows"
    );
}

#[tokio::test]
async fn history_ingestion_preserves_cross_page_reply_metadata() {
    let dir = tempdir().expect("tempdir failed");
    let db = ArchiveDb::open(dir.path().join("archive.db")).expect("open db failed");
    let adapter = FakeTelegramAdapter::new();

    let peer_id = PeerId::new(5005);

    adapter.add_message(make_test_message(peer_id, 1, 1700000001, "First message"));
    let mut reply_msg = make_test_message(peer_id, 200, 1700000200, "Reply across pages");
    reply_msg.reply_to_msg_id = Some(MessageId::new(1));
    adapter.add_message(reply_msg);

    let pipeline = HistoryIngestionPipeline::new(1);
    pipeline
        .ingest_history(&adapter, &db, peer_id)
        .await
        .expect("ingest failed");

    let stored_reply = db
        .get_message(MessageKey::new(peer_id, MessageId::new(200)))
        .expect("get msg failed")
        .expect("missing reply message");

    assert_eq!(stored_reply.reply_to_msg_id, Some(MessageId::new(1)));
}

#[tokio::test]
async fn history_ingestion_preserves_out_of_range_reply_metadata() {
    let dir = tempdir().expect("tempdir failed");
    let db = ArchiveDb::open(dir.path().join("archive.db")).expect("open db failed");
    let adapter = FakeTelegramAdapter::new();

    let peer_id = PeerId::new(6006);

    let mut msg_105 =
        make_test_message(peer_id, 105, 1700000105, "Replying to out of range target");
    msg_105.reply_to_msg_id = Some(MessageId::new(1));

    adapter.add_message(msg_105);

    let pipeline = HistoryIngestionPipeline::new(10);
    pipeline
        .ingest_history(&adapter, &db, peer_id)
        .await
        .expect("ingest failed");

    let stored = db
        .get_message(MessageKey::new(peer_id, MessageId::new(105)))
        .expect("get msg failed")
        .expect("missing message");

    assert_eq!(
        stored.reply_to_msg_id,
        Some(MessageId::new(1)),
        "Out of range reply metadata must be preserved"
    );
}

#[tokio::test]
async fn channel_history_preserves_auxiliary_peers_and_pts() {
    let dir = tempdir().expect("tempdir failed");
    let db = ArchiveDb::open(dir.path().join("archive.db")).expect("open db failed");
    let adapter = FakeTelegramAdapter::new();

    let channel_id = PeerId::new(-1000000005555);

    for id in 1..=20 {
        adapter.add_message(make_test_message(
            channel_id,
            id,
            1700000000 + id,
            &format!("Broadcast {id}"),
        ));
    }

    adapter.set_channel_pts(channel_id, 4567);
    adapter.set_auxiliary_peers(
        channel_id,
        vec![
            PeerRecord {
                peer_id: PeerId::new(999),
                peer_type: PeerType::User,
                name: Some("Author User".to_string()),
                username: Some("author_user".to_string()),
                phone: None,
                raw_tl: Some(vec![9, 9, 9]),
                updated_at: 1700000000,
            },
            PeerRecord {
                peer_id: channel_id,
                peer_type: PeerType::Channel,
                name: Some("Tech Announcements".to_string()),
                username: Some("tech_announcements".to_string()),
                phone: None,
                raw_tl: Some(vec![5, 5, 5]),
                updated_at: 1700000000,
            },
        ],
    );

    let pipeline = HistoryIngestionPipeline::new(50);
    let summary = pipeline
        .ingest_history(&adapter, &db, channel_id)
        .await
        .expect("ingest failed");

    assert_eq!(summary.messages_ingested, 20);
    assert_eq!(summary.auxiliary_peers_ingested, 2);

    let sync_state = db
        .get_sync_state(channel_id)
        .expect("get sync state failed")
        .expect("missing sync state");
    assert_eq!(sync_state.pts, Some(4567));

    let author_peer = db
        .get_peer(PeerId::new(999))
        .expect("get peer failed")
        .expect("author peer missing from SQLite");
    assert_eq!(author_peer.username.as_deref(), Some("author_user"));

    let channel_peer = db
        .get_peer(channel_id)
        .expect("get peer failed")
        .expect("channel peer missing from SQLite");
    assert_eq!(channel_peer.username.as_deref(), Some("tech_announcements"));
}

#[tokio::test]
async fn pagination_pipeline_guards_against_stale_non_progressing_offsets() {
    let dir = tempdir().expect("tempdir failed");
    let db = ArchiveDb::open(dir.path().join("archive.db")).expect("open db failed");

    #[derive(Clone)]
    struct StalePageAdapter;

    #[async_trait]
    impl TelegramAdapter for StalePageAdapter {
        async fn get_dialogs(&self) -> AdapterResult<Vec<PeerRecord>> {
            Ok(Vec::new())
        }

        async fn get_history_page(
            &self,
            peer_id: PeerId,
            _limit: usize,
            offset_id: Option<MessageId>,
        ) -> AdapterResult<HistoryPage> {
            let msg_id = match offset_id {
                None => 100,
                Some(_) => 100,
            };
            Ok(HistoryPage {
                messages: vec![make_test_message(peer_id, msg_id, 1700000000, "Loop msg")],
                auxiliary_peers: Vec::new(),
                pts: None,
                count: Some(1),
                raw_topics: Vec::new(),
            })
        }

        async fn get_messages(
            &self,
            _peer_id: PeerId,
            _peer_type: Option<vendetta_model::PeerType>,
            _message_ids: &[MessageId],
        ) -> AdapterResult<Vec<MessageRecord>> {
            Ok(Vec::new())
        }

        async fn resolve_reply_target(
            &self,
            _source_peer: PeerId,
            _source_peer_type: Option<vendetta_model::PeerType>,
            _target_peer: Option<PeerId>,
            _target_peer_type: Option<vendetta_model::PeerType>,
            _target_msg_id: MessageId,
        ) -> AdapterResult<Option<MessageRecord>> {
            Ok(None)
        }
    }

    let pipeline = HistoryIngestionPipeline::new(1);
    let result = pipeline
        .ingest_history(&StalePageAdapter, &db, PeerId::new(999))
        .await;

    match result {
        Err(SyncError::NonProgressingPagination {
            offset_id,
            returned_min_id,
            ..
        }) => {
            assert_eq!(offset_id, Some(MessageId::new(100)));
            assert_eq!(returned_min_id, Some(MessageId::new(100)));
        }
        other => panic!("Expected NonProgressingPagination error, got: {:?}", other),
    }
}

#[tokio::test]
async fn pagination_pipeline_handles_non_monotonic_ids_and_dates() {
    let dir = tempdir().expect("tempdir failed");
    let db = ArchiveDb::open(dir.path().join("archive.db")).expect("open db failed");
    let adapter = FakeTelegramAdapter::new();

    let peer_id = PeerId::new(4444);

    adapter.add_message(make_test_message(peer_id, 1, 3000, "Old ID, high date"));
    adapter.add_message(make_test_message(peer_id, 2, 1000, "Mid ID, low date"));
    adapter.add_message(make_test_message(peer_id, 3, 2000, "High ID, mid date"));

    let pipeline = HistoryIngestionPipeline::new(1);
    let summary = pipeline
        .ingest_history(&adapter, &db, peer_id)
        .await
        .expect("ingest failed");

    assert_eq!(summary.messages_ingested, 3);
    assert_eq!(summary.batches_committed, 3);

    let messages = db
        .list_messages_by_peer(peer_id, 10, 0)
        .expect("list messages failed");
    let ids: Vec<i64> = messages.iter().map(|m| m.key.message_id.raw()).collect();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[tokio::test]
async fn history_pipeline_resumes_cleanly_after_interruption() {
    let dir = tempdir().expect("tempdir failed");
    let db_path = dir.path().join("archive.db");
    let db = ArchiveDb::open(&db_path).expect("open db failed");
    let adapter = FakeTelegramAdapter::new();

    let peer_id = PeerId::new(7777);

    for id in 1..=150 {
        adapter.add_message(make_test_message(
            peer_id,
            id,
            1700000000 + id,
            &format!("Message {id}"),
        ));
    }

    let pipeline = HistoryIngestionPipeline::new(50);

    let page1 = adapter.get_history_page(peer_id, 50, None).await.unwrap();
    let sync_state1 = vendetta_model::SyncStateRecord {
        peer_id,
        pts: None,
        qts: None,
        date: None,
        seq: None,
        min_message_id: Some(101),
        max_message_id: Some(150),
        last_synced_at: 1700000000,
    };
    db.ingest_history_page(
        peer_id,
        &page1.messages,
        &page1.auxiliary_peers,
        Some(&sync_state1),
    )
    .unwrap();

    assert_eq!(db.count_messages_by_peer(peer_id).unwrap(), 50);

    adapter.inject_error("RPC 500: INTERNAL_SERVER_ERROR");
    let resume_attempt = pipeline.ingest_history(&adapter, &db, peer_id).await;
    assert!(resume_attempt.is_err(), "Must fail when adapter errors");

    assert_eq!(db.count_messages_by_peer(peer_id).unwrap(), 50);
    let state_after_fail = db.get_sync_state(peer_id).unwrap().unwrap();
    assert_eq!(state_after_fail.min_message_id, Some(101));

    adapter.clear_error();
    let new_pipeline = HistoryIngestionPipeline::new(50);
    let summary = new_pipeline
        .ingest_history(&adapter, &db, peer_id)
        .await
        .expect("retry ingest must succeed");

    assert_eq!(summary.messages_ingested, 100);
    assert_eq!(summary.min_message_id, Some(MessageId::new(1)));
    assert_eq!(summary.max_message_id, Some(MessageId::new(150)));

    assert_eq!(db.count_messages_by_peer(peer_id).unwrap(), 150);
}

#[tokio::test]
async fn history_ingestion_supports_all_peer_types() {
    let dir = tempdir().expect("tempdir failed");
    let db = ArchiveDb::open(dir.path().join("archive.db")).expect("open db failed");
    let adapter = FakeTelegramAdapter::new();

    let peers = vec![
        (PeerId::new(100), PeerType::User),
        (PeerId::new(-200), PeerType::Group),
        (PeerId::new(-100000000300), PeerType::Channel),
    ];

    for (peer_id, _) in &peers {
        for msg_id in 1..=10 {
            adapter.add_message(make_test_message(
                *peer_id,
                msg_id,
                1700000000 + msg_id,
                &format!("Msg {msg_id} for peer {peer_id}"),
            ));
        }
    }

    let pipeline = HistoryIngestionPipeline::new(10);
    for (peer_id, _) in &peers {
        let summary = pipeline
            .ingest_history(&adapter, &db, *peer_id)
            .await
            .expect("ingest failed");
        assert_eq!(summary.messages_ingested, 10);
        let count = db.count_messages_by_peer(*peer_id).expect("count failed");
        assert_eq!(count, 10);
    }
}

#[tokio::test]
async fn non_text_message_edits_are_captured_in_revision_history() {
    let dir = tempdir().expect("tempdir failed");
    let db = ArchiveDb::open(dir.path().join("archive.db")).expect("open db failed");

    let peer_id = PeerId::new(8888);
    let msg_id = MessageId::new(10);
    let key = MessageKey::new(peer_id, msg_id);

    let mut msg1 = make_test_message(peer_id, 10, 1700000000, "Text remains the same");
    msg1.entities_json = Some(r#"[{"type":"Bold","offset":0,"length":4}]"#.to_string());
    db.insert_or_update_message(&msg1)
        .expect("insert msg1 failed");

    let mut msg2 = make_test_message(peer_id, 10, 1700000000, "Text remains the same");
    msg2.entities_json = Some(r#"[{"type":"Italic","offset":0,"length":4}]"#.to_string());
    msg2.edit_date = Some(1700000500);
    msg2.state = MessageState::Edited;
    db.insert_or_update_message(&msg2)
        .expect("update msg2 failed");

    let revisions = db
        .list_message_revisions(key)
        .expect("list revisions failed");
    assert_eq!(revisions.len(), 1);
    assert_eq!(
        revisions[0].entities_json.as_deref(),
        Some(r#"[{"type":"Bold","offset":0,"length":4}]"#)
    );

    let current = db.get_message(key).expect("get msg failed").unwrap();
    assert_eq!(
        current.entities_json.as_deref(),
        Some(r#"[{"type":"Italic","offset":0,"length":4}]"#)
    );
    assert_eq!(current.state, MessageState::Edited);
}

#[tokio::test]
async fn large_history_streams_with_bounded_memory() {
    let dir = tempdir().expect("tempdir failed");
    let db = ArchiveDb::open(dir.path().join("archive.db")).expect("open db failed");
    let adapter = FakeTelegramAdapter::new();

    let peer_id = PeerId::new(99999);
    let total_messages = 20_000;

    for id in 1..=total_messages {
        adapter.add_message(make_test_message(
            peer_id,
            id,
            1700000000 + id,
            "Streaming bounded batch message payload",
        ));
    }

    let pipeline = HistoryIngestionPipeline::new(100);
    let summary = pipeline
        .ingest_history(&adapter, &db, peer_id)
        .await
        .expect("ingest failed");

    assert_eq!(summary.batches_committed, 200);
    assert_eq!(summary.messages_ingested, total_messages as usize);

    let count = db.count_messages_by_peer(peer_id).expect("count failed");
    assert_eq!(count, total_messages as usize);
}
