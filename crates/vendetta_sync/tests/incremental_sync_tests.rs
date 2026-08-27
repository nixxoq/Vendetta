use std::{collections::HashSet, sync::Arc};
use tempfile::{TempDir, tempdir};

use vendetta_model::{
    AccountSyncState, ChannelQueueItem, ChannelQueueStatus, DialogFilterRecord, DialogInfo,
    MessageId, MessageKey, MessageRecord, MessageState, NormalizedUpdate, PeerId, PeerRecord,
    PeerType, VerificationObservation,
};
use vendetta_storage::ArchiveDb;
use vendetta_sync::{
    ChannelQueueWorker, CoordinatedSyncPipeline, IncrementalSyncEngine, SyncError,
};
use vendetta_tg_adapter::{
    ChannelDifferenceResult, CommonDifferenceResult, DialogsPage, FakeTelegramAdapter,
    TelegramAdapter,
};

fn create_test_db() -> (TempDir, Arc<ArchiveDb>) {
    let dir = tempdir().expect("Failed to create tempdir");
    let db_path = dir.path().join("archive.db");
    let db = ArchiveDb::open(&db_path).expect("Failed to open test database");
    (dir, Arc::new(db))
}

fn create_dummy_message(peer_id: PeerId, msg_id: i64, text: &str) -> MessageRecord {
    MessageRecord {
        key: MessageKey::new(peer_id.raw(), msg_id),
        date: 1700000000 + msg_id,
        sender_id: Some(peer_id),
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
        raw_tl: Some(vec![1, 2, 3]),
    }
}

#[tokio::test]
async fn baseline_state_is_immutable() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let initial = AccountSyncState {
        account_id: "default".to_string(),
        pts: 500,
        qts: 50,
        date: 1700000000,
        seq: 10,
        sync_uncertain: false,
        last_synced_at: 1700000000,
    };
    adapter.set_account_state(initial.clone());

    let pipeline = CoordinatedSyncPipeline::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = pipeline.run_full_sync(&[]).await.unwrap();

    assert_eq!(summary.baseline_pts, 500);
    let state = storage.get_account_sync_state("default").unwrap().unwrap();
    assert_eq!(state.pts, 500);
}

#[tokio::test]
async fn history_difference_reconciles_cleanly() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let peer_a = PeerId::new(1001);

    adapter.set_account_state(AccountSyncState {
        account_id: "default".to_string(),
        pts: 1000,
        qts: 10,
        date: 1700000000,
        seq: 1,
        sync_uncertain: false,
        last_synced_at: 1700000000,
    });

    adapter.add_message(create_dummy_message(peer_a, 1, "Message 1"));

    let new_msg = create_dummy_message(peer_a, 2, "Message 2 (during scan)");
    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![new_msg.clone()],
        other_updates: vec![],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 1002,
            qts: 10,
            date: 1700000100,
            seq: 2,
            sync_uncertain: false,
            last_synced_at: 1700000100,
        },
    });

    let pipeline = CoordinatedSyncPipeline::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = pipeline.run_full_sync(&[peer_a]).await.unwrap();

    assert_eq!(summary.history_messages_ingested, 1);
    assert_eq!(summary.delta_messages_ingested, 1);
    assert_eq!(summary.final_pts, 1002);

    let count = storage.count_messages_by_peer(peer_a).unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn contiguous_new_message_updates_advance_pts() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let peer_id = PeerId::new(2001);

    let msg = create_dummy_message(peer_id, 42, "Live Message");
    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![NormalizedUpdate::NewMessage {
            message: msg.clone(),
            pts: Some(101),
            pts_count: 1,
        }],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 101,
            qts: 10,
            date: 1700000050,
            seq: 2,
            sync_uncertain: false,
            last_synced_at: 1700000050,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = engine.sync_common().await.unwrap();

    assert_eq!(summary.new_messages_ingested, 1);
    assert_eq!(summary.final_pts, 101);
    let stored = storage.get_message(msg.key).unwrap().unwrap();
    assert_eq!(stored.text, Some("Live Message".to_string()));
}

#[tokio::test]
async fn contiguous_edit_updates_advance_pts_and_record_revision() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let peer_id = PeerId::new(2001);

    let initial_msg = create_dummy_message(peer_id, 50, "Original Text");
    storage.insert_or_update_message(&initial_msg).unwrap();

    let mut edited_msg = initial_msg.clone();
    edited_msg.text = Some("Edited Text".to_string());
    edited_msg.edit_date = Some(1700000500);

    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![NormalizedUpdate::EditedMessage {
            message: edited_msg.clone(),
            pts: Some(102),
            pts_count: 1,
        }],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 102,
            qts: 10,
            date: 1700000500,
            seq: 3,
            sync_uncertain: false,
            last_synced_at: 1700000500,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = engine.sync_common().await.unwrap();

    assert_eq!(summary.edits_applied, 1);
    let stored = storage.get_message(initial_msg.key).unwrap().unwrap();
    assert_eq!(stored.text, Some("Edited Text".to_string()));

    let revs = storage.list_message_revisions(initial_msg.key).unwrap();
    assert_eq!(revs.len(), 1);
    assert_eq!(revs[0].text, Some("Original Text".to_string()));
}

#[tokio::test]
async fn contiguous_delete_marks_tombstone_and_advances_pts() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let peer_id = PeerId::new(2001);

    storage
        .upsert_peer(&PeerRecord {
            peer_id,
            peer_type: PeerType::User,
            name: Some("User 2001".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    let msg = create_dummy_message(peer_id, 60, "To be deleted");
    storage.insert_or_update_message(&msg).unwrap();

    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![NormalizedUpdate::CommonDeletedMessages {
            message_ids: vec![MessageId::new(60)],
            pts: Some(103),
            pts_count: 1,
        }],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 103,
            qts: 10,
            date: 1700000600,
            seq: 4,
            sync_uncertain: false,
            last_synced_at: 1700000600,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = engine.sync_common().await.unwrap();

    assert_eq!(summary.deletes_applied, 1);
    let stored = storage.get_message(msg.key).unwrap().unwrap();
    assert_eq!(stored.state, MessageState::Deleted);

    let peer0_msg = storage.get_message(MessageKey::new(0, 60)).unwrap();
    assert!(peer0_msg.is_none(), "Peer ID 0 must never be fabricated");
}

#[tokio::test]
async fn multi_id_delete_event_marks_all_tombstones() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let peer_id = PeerId::new(2001);

    storage
        .upsert_peer(&PeerRecord {
            peer_id,
            peer_type: PeerType::User,
            name: Some("User 2001".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    for id in 1..=5 {
        let msg = create_dummy_message(peer_id, id, &format!("Msg {id}"));
        storage.insert_or_update_message(&msg).unwrap();
    }

    let delete_ids: Vec<MessageId> = (1..=5).map(MessageId::new).collect();
    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![NormalizedUpdate::CommonDeletedMessages {
            message_ids: delete_ids,
            pts: Some(110),
            pts_count: 5,
        }],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 110,
            qts: 10,
            date: 1700000700,
            seq: 5,
            sync_uncertain: false,
            last_synced_at: 1700000700,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = engine.sync_common().await.unwrap();

    assert_eq!(summary.deletes_applied, 5);
    for id in 1..=5 {
        let stored = storage
            .get_message(MessageKey::new(peer_id.raw(), id))
            .unwrap()
            .unwrap();
        assert_eq!(stored.state, MessageState::Deleted);
    }
}

#[tokio::test]
async fn channel_new_message_advances_channel_pts() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let channel_id = PeerId::new(-1_000_000_123_456);

    let post = create_dummy_message(channel_id, 10, "Channel Post");
    adapter.enqueue_channel_difference(
        channel_id,
        ChannelDifferenceResult::Difference {
            final_state: true,
            pts: 25,
            timeout: Some(60),
            new_messages: vec![post.clone()],
            other_updates: vec![],
            auxiliary_peers: vec![],
        },
    );

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let res = engine.sync_channel(channel_id, Some(20)).await.unwrap();
    let state = res.state;

    assert_eq!(state.pts, Some(25));
    assert_eq!(res.new_messages_ingested, 1);
    let stored = storage.get_message(post.key).unwrap().unwrap();
    assert_eq!(stored.text, Some("Channel Post".to_string()));
}

#[tokio::test]
async fn channel_edit_and_delete_update_channel_state() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let channel_id = PeerId::new(-1_000_000_123_456);

    let initial = create_dummy_message(channel_id, 15, "Post 15");
    storage.insert_or_update_message(&initial).unwrap();

    let mut edited = initial.clone();
    edited.text = Some("Edited Post 15".to_string());

    adapter.enqueue_channel_difference(
        channel_id,
        ChannelDifferenceResult::Difference {
            final_state: true,
            pts: 30,
            timeout: Some(60),
            new_messages: vec![],
            other_updates: vec![
                NormalizedUpdate::EditedMessage {
                    message: edited,
                    pts: Some(29),
                    pts_count: 1,
                },
                NormalizedUpdate::ChannelDeletedMessages {
                    channel_id,
                    message_ids: vec![MessageId::new(15)],
                    pts: Some(30),
                    pts_count: 1,
                },
            ],
            auxiliary_peers: vec![],
        },
    );

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let res = engine.sync_channel(channel_id, Some(28)).await.unwrap();
    let state = res.state;

    assert_eq!(state.pts, Some(30));
    assert_eq!(res.edits_applied, 1);
    assert_eq!(res.deletes_applied, 1);
    let stored = storage.get_message(initial.key).unwrap().unwrap();
    assert_eq!(stored.state, MessageState::Deleted);
}

#[tokio::test]
async fn multi_slice_common_difference_pages_to_final_state() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let peer_id = PeerId::new(3001);

    adapter.enqueue_common_difference(CommonDifferenceResult::Slice {
        new_messages: vec![create_dummy_message(peer_id, 1, "Slice 1 Msg")],
        other_updates: vec![],
        auxiliary_peers: vec![],
        intermediate_state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 110,
            qts: 10,
            date: 1700000100,
            seq: 2,
            sync_uncertain: false,
            last_synced_at: 1700000100,
        },
    });

    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![create_dummy_message(peer_id, 2, "Slice 2 Msg")],
        other_updates: vec![],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 120,
            qts: 10,
            date: 1700000200,
            seq: 3,
            sync_uncertain: false,
            last_synced_at: 1700000200,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = engine.sync_common().await.unwrap();

    assert_eq!(summary.slices_processed, 2);
    assert_eq!(summary.new_messages_ingested, 2);
    assert_eq!(summary.final_pts, 120);

    let state = storage.get_account_sync_state("default").unwrap().unwrap();
    assert_eq!(state.pts, 120);
}

#[tokio::test]
async fn non_final_channel_difference_enqueues_continuation() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let channel_id = PeerId::new(-1_000_000_555_666);

    adapter.enqueue_channel_difference(
        channel_id,
        ChannelDifferenceResult::Difference {
            final_state: false,
            pts: 50,
            timeout: Some(30),
            new_messages: vec![create_dummy_message(channel_id, 1, "Page 1")],
            other_updates: vec![],
            auxiliary_peers: vec![],
        },
    );

    adapter.enqueue_channel_difference(
        channel_id,
        ChannelDifferenceResult::Difference {
            final_state: true,
            pts: 60,
            timeout: Some(30),
            new_messages: vec![create_dummy_message(channel_id, 2, "Page 2")],
            other_updates: vec![],
            auxiliary_peers: vec![],
        },
    );

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let res = engine.sync_channel(channel_id, Some(40)).await.unwrap();
    let state = res.state;

    assert_eq!(state.pts, Some(60));
    assert_eq!(storage.count_messages_by_peer(channel_id).unwrap(), 2);
}

#[tokio::test]
async fn difference_empty_preserves_pts_and_qts() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let initial = AccountSyncState {
        account_id: "default".to_string(),
        pts: 999,
        qts: 88,
        date: 1700000000,
        seq: 1,
        sync_uncertain: false,
        last_synced_at: 1700000000,
    };
    storage.upsert_account_sync_state(&initial).unwrap();
    adapter.set_account_state(initial);

    adapter.enqueue_common_difference(CommonDifferenceResult::Empty {
        date: 1700000999,
        seq: 5,
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = engine.sync_common().await.unwrap();

    assert_eq!(summary.final_pts, 999);
    let state = storage.get_account_sync_state("default").unwrap().unwrap();
    assert_eq!(state.pts, 999);
    assert_eq!(state.qts, 88);
    assert_eq!(state.date, 1700000999);
    assert_eq!(state.seq, 5);
}

#[tokio::test]
async fn channel_difference_empty_is_no_op() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let channel_id = PeerId::new(-1_000_000_777_888);

    adapter.enqueue_channel_difference(
        channel_id,
        ChannelDifferenceResult::Empty {
            final_state: true,
            pts: 100,
            timeout: Some(60),
        },
    );

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let res = engine.sync_channel(channel_id, Some(100)).await.unwrap();
    let state = res.state;

    assert_eq!(state.pts, Some(100));
}

#[tokio::test]
async fn unknown_state_affecting_common_update_triggers_recovery() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let peer_id = PeerId::new(2001);

    adapter.set_account_state(AccountSyncState {
        account_id: "default".to_string(),
        pts: 100,
        qts: 10,
        date: 1700000000,
        seq: 1,
        sync_uncertain: false,
        last_synced_at: 1700000000,
    });

    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![NormalizedUpdate::Unsupported {
            constructor_name: "updateUnknownFuture".to_string(),
            constructor_id: 0xdeadbeef,
            affects_sync_state: true,
            pts: Some(105),
            pts_count: 5,
            qts: None,
            qts_count: 0,
            diagnostic_info: Some("Unsupported future update".to_string()),
            raw_tl: vec![0xde, 0xad, 0xbe, 0xef],
        }],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 105,
            qts: 10,
            date: 1700000010,
            seq: 2,
            sync_uncertain: false,
            last_synced_at: 1700000010,
        },
    });

    let recovered_msg = create_dummy_message(peer_id, 102, "Clean recovered msg");
    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![recovered_msg.clone()],
        other_updates: vec![],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 102,
            qts: 10,
            date: 1700000020,
            seq: 3,
            sync_uncertain: false,
            last_synced_at: 1700000020,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = engine.sync_common().await.unwrap();

    assert_eq!(summary.final_pts, 102);
    assert!(!summary.had_buffer_overflow);
    assert_eq!(summary.new_messages_ingested, 1);

    let events = storage.list_unsupported_events().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].constructor_id, 0xdeadbeef);
    assert_eq!(events[0].pts, Some(105));
    assert_eq!(events[0].pts_count, Some(5));
    assert_eq!(events[0].raw_tl, vec![0xde, 0xad, 0xbe, 0xef]);

    let state = storage.get_account_sync_state("default").unwrap().unwrap();
    assert!(!state.sync_uncertain);
    assert_eq!(state.pts, 102);
}

#[tokio::test]
async fn unknown_state_affecting_channel_update_triggers_resync() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let channel_id = PeerId::new(-1_000_000_999_777);

    adapter.enqueue_channel_difference(
        channel_id,
        ChannelDifferenceResult::Difference {
            final_state: true,
            pts: 205,
            timeout: Some(30),
            new_messages: vec![],
            other_updates: vec![NormalizedUpdate::Unsupported {
                constructor_name: "updateChannelUnknown".to_string(),
                constructor_id: 0x44444444,
                affects_sync_state: true,
                pts: Some(205),
                pts_count: 5,
                qts: None,
                qts_count: 0,
                diagnostic_info: Some("Unknown channel update".to_string()),
                raw_tl: vec![4, 4, 4],
            }],
            auxiliary_peers: vec![],
        },
    );

    adapter.enqueue_channel_difference(
        channel_id,
        ChannelDifferenceResult::Difference {
            final_state: true,
            pts: 202,
            timeout: Some(30),
            new_messages: vec![create_dummy_message(channel_id, 202, "Channel msg 202")],
            other_updates: vec![],
            auxiliary_peers: vec![],
        },
    );

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let res = engine.sync_channel(channel_id, Some(200)).await.unwrap();
    let state = res.state;

    assert_eq!(state.pts, Some(202));
    assert!(!state.sync_uncertain);

    let events = storage.list_unsupported_events().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].constructor_id, 0x44444444);
    assert_eq!(events[0].peer_id, Some(channel_id));
}

#[tokio::test]
async fn unknown_state_affecting_qts_update_defers_policy() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![NormalizedUpdate::Unsupported {
            constructor_name: "updateNewEncryptedMessage".to_string(),
            constructor_id: 0x12bcbd9a,
            affects_sync_state: true,
            pts: None,
            pts_count: 0,
            qts: Some(50),
            qts_count: 1,
            diagnostic_info: Some("Secret chat message update".to_string()),
            raw_tl: vec![0x12, 0xbc, 0xbd, 0x9a],
        }],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 300,
            qts: 50,
            date: 1700000030,
            seq: 4,
            sync_uncertain: false,
            last_synced_at: 1700000030,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let res = engine.sync_common().await;

    assert!(res.is_err());
    match res.unwrap_err() {
        SyncError::UnsupportedStateAffectingUpdate {
            constructor_id,
            pts,
            pts_count,
        } => {
            assert_eq!(constructor_id, 0x12bcbd9a);
            assert_eq!(pts, None);
            assert_eq!(pts_count, 0);
        }
        other => panic!("Expected UnsupportedStateAffectingUpdate, got {other:?}"),
    }

    let events = storage.list_unsupported_events().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].constructor_id, 0x12bcbd9a);
    assert_eq!(events[0].qts, Some(50));
    assert_eq!(events[0].qts_count, Some(1));

    let state = storage.get_account_sync_state("default").unwrap().unwrap();
    assert!(state.sync_uncertain);
}

#[tokio::test]
async fn repeated_unknown_state_affecting_updates_halt_terminally_and_flag_uncertainty() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![NormalizedUpdate::Unsupported {
            constructor_name: "updateUnknown1".to_string(),
            constructor_id: 0x11111111,
            affects_sync_state: true,
            pts: Some(105),
            pts_count: 5,
            qts: None,
            qts_count: 0,
            diagnostic_info: None,
            raw_tl: vec![1],
        }],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 105,
            qts: 10,
            date: 1700000020,
            seq: 3,
            sync_uncertain: false,
            last_synced_at: 1700000020,
        },
    });

    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![NormalizedUpdate::Unsupported {
            constructor_name: "updateUnknown2".to_string(),
            constructor_id: 0x22222222,
            affects_sync_state: true,
            pts: Some(110),
            pts_count: 5,
            qts: None,
            qts_count: 0,
            diagnostic_info: None,
            raw_tl: vec![2],
        }],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 110,
            qts: 10,
            date: 1700000020,
            seq: 3,
            sync_uncertain: false,
            last_synced_at: 1700000020,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let res = engine.sync_common().await;

    assert!(res.is_err());
    match res.unwrap_err() {
        SyncError::UnsupportedStateAffectingUpdate { constructor_id, .. } => {
            assert_eq!(constructor_id, 0x22222222);
        }
        other => panic!("Expected UnsupportedStateAffectingUpdate, got {other:?}"),
    }

    let state = storage.get_account_sync_state("default").unwrap().unwrap();
    assert!(state.sync_uncertain);
}

#[tokio::test]
async fn unknown_benign_updates_are_retained_in_unsupported_events() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![NormalizedUpdate::Unsupported {
            constructor_name: "updateUserWallpaper".to_string(),
            constructor_id: 0x33333333,
            affects_sync_state: false,
            pts: None,
            pts_count: 0,
            qts: None,
            qts_count: 0,
            diagnostic_info: Some("Wallpaper change".to_string()),
            raw_tl: vec![3, 3, 3],
        }],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 101,
            qts: 10,
            date: 1700000010,
            seq: 2,
            sync_uncertain: false,
            last_synced_at: 1700000010,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = engine.sync_common().await.unwrap();
    assert_eq!(summary.final_pts, 101);
}

#[tokio::test]
async fn message_empty_lookup_does_not_mark_message_deleted() {
    let (_dir, storage) = create_test_db();
    let peer_id = PeerId::new(4001);

    let msg = create_dummy_message(peer_id, 100, "Secret Text");
    storage.insert_or_update_message(&msg).unwrap();

    let observation = VerificationObservation::ObservedEmptyOrUnavailable;
    assert_ne!(observation, VerificationObservation::ConfirmedDeleted);

    let stored = storage.get_message(msg.key).unwrap().unwrap();
    assert_eq!(stored.state, MessageState::Active);
    assert_eq!(stored.text, Some("Secret Text".to_string()));
}

#[tokio::test]
async fn dormant_archived_common_peer_is_included_in_too_long_recovery() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let chat_a = PeerId::new(5001);
    let chat_b = PeerId::new(5002);

    storage
        .upsert_peer(&PeerRecord {
            peer_id: chat_a,
            peer_type: PeerType::User,
            name: Some("User A".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    storage
        .upsert_peer(&PeerRecord {
            peer_id: chat_b,
            peer_type: PeerType::User,
            name: Some("Dormant B".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1600000000,
        })
        .unwrap();

    adapter.add_message(create_dummy_message(chat_b, 99, "New msg in dormant chat"));

    adapter.enqueue_common_difference(CommonDifferenceResult::TooLong { pts: 50000 });
    adapter.set_account_state(AccountSyncState {
        account_id: "default".to_string(),
        pts: 50000,
        qts: 10,
        date: 1700001000,
        seq: 50,
        sync_uncertain: false,
        last_synced_at: 1700001000,
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = engine.sync_common().await.unwrap();

    assert!(summary.had_buffer_overflow);
    assert_eq!(summary.final_pts, 50000);

    let stored = storage
        .get_message(MessageKey::new(chat_b.raw(), 99))
        .unwrap()
        .unwrap();
    assert_eq!(stored.text, Some("New msg in dormant chat".to_string()));
}

#[tokio::test]
async fn server_side_min_id_rescan_bounds_recovery_range() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let peer_id = PeerId::new(6001);

    storage
        .upsert_peer(&PeerRecord {
            peer_id,
            peer_type: PeerType::User,
            name: Some("Big Chat".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    storage
        .insert_or_update_message(&create_dummy_message(peer_id, 1_000_000, "Old max msg"))
        .unwrap();

    for id in 1_000_001..=1_000_150 {
        adapter.add_message(create_dummy_message(peer_id, id, &format!("New msg {id}")));
    }

    adapter.enqueue_common_difference(CommonDifferenceResult::TooLong { pts: 100_000 });
    adapter.set_account_state(AccountSyncState {
        account_id: "default".to_string(),
        pts: 100_000,
        qts: 10,
        date: 1700002000,
        seq: 100,
        sync_uncertain: false,
        last_synced_at: 1700002000,
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = engine.sync_common().await.unwrap();

    assert_eq!(summary.final_pts, 100_000);
    let count = storage.count_messages_by_peer(peer_id).unwrap();
    assert_eq!(count, 151);
}

#[tokio::test]
async fn per_peer_history_rescan_respects_state_boundaries() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let chat_a = PeerId::new(7001);
    let chat_b = PeerId::new(7002);

    storage
        .upsert_peer(&PeerRecord {
            peer_id: chat_a,
            peer_type: PeerType::User,
            name: Some("Chat A".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    storage
        .upsert_peer(&PeerRecord {
            peer_id: chat_b,
            peer_type: PeerType::User,
            name: Some("Chat B".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    storage
        .insert_or_update_message(&create_dummy_message(chat_a, 1000, "A max"))
        .unwrap();
    storage
        .insert_or_update_message(&create_dummy_message(chat_b, 100, "B max"))
        .unwrap();

    adapter.add_message(create_dummy_message(chat_b, 150, "B new msg 150"));

    adapter.enqueue_common_difference(CommonDifferenceResult::TooLong { pts: 80_000 });
    adapter.set_account_state(AccountSyncState {
        account_id: "default".to_string(),
        pts: 80_000,
        qts: 10,
        date: 1700003000,
        seq: 80,
        sync_uncertain: false,
        last_synced_at: 1700003000,
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    engine.sync_common().await.unwrap();

    let msg150 = storage
        .get_message(MessageKey::new(chat_b.raw(), 150))
        .unwrap();
    assert!(msg150.is_some());
}

#[tokio::test]
async fn dialog_pagination_streams_all_dialogs() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let channel_page4 = PeerId::new(-1_000_000_999_004);

    let page1 = DialogsPage {
        dialogs: vec![DialogInfo {
            peer_id: PeerId::new(-1_000_000_999_001),
            peer_type: Some(PeerType::Channel),
            pts: Some(10),
            top_message: None,
            unread_count: 0,
            is_pinned: false,
            folder_id: Some(0),
            is_unresolved: false,
        }],
        auxiliary_peers: vec![],
        is_last_page: false,
        next_offset_date: 1700000100,
        next_offset_id: 1,
        next_offset_peer: None,
    };

    let page2 = DialogsPage {
        dialogs: vec![DialogInfo {
            peer_id: PeerId::new(-1_000_000_999_002),
            peer_type: Some(PeerType::Channel),
            pts: Some(20),
            top_message: None,
            unread_count: 0,
            is_pinned: false,
            folder_id: Some(0),
            is_unresolved: false,
        }],
        auxiliary_peers: vec![],
        is_last_page: false,
        next_offset_date: 1700000200,
        next_offset_id: 2,
        next_offset_peer: None,
    };

    let page3 = DialogsPage {
        dialogs: vec![DialogInfo {
            peer_id: PeerId::new(-1_000_000_999_003),
            peer_type: Some(PeerType::Channel),
            pts: Some(30),
            top_message: None,
            unread_count: 0,
            is_pinned: false,
            folder_id: Some(0),
            is_unresolved: false,
        }],
        auxiliary_peers: vec![],
        is_last_page: false,
        next_offset_date: 1700000300,
        next_offset_id: 3,
        next_offset_peer: None,
    };

    let page4 = DialogsPage {
        dialogs: vec![DialogInfo {
            peer_id: channel_page4,
            peer_type: Some(PeerType::Channel),
            pts: Some(40),
            top_message: None,
            unread_count: 0,
            is_pinned: false,
            folder_id: Some(0),
            is_unresolved: false,
        }],
        auxiliary_peers: vec![],
        is_last_page: true,
        next_offset_date: 0,
        next_offset_id: 0,
        next_offset_peer: None,
    };

    adapter.set_dialog_pages(0, vec![page1, page2, page3, page4]);

    let sync_engine = Arc::new(IncrementalSyncEngine::new(
        Arc::clone(&adapter),
        Arc::clone(&storage),
    ));
    let worker =
        ChannelQueueWorker::new(Arc::clone(&adapter), Arc::clone(&storage), sync_engine, 1);

    let enqueued = worker.discover_and_enqueue_stale_channels().await.unwrap();
    assert_eq!(enqueued, 4);

    let pending = storage.list_pending_channels().unwrap();
    assert!(pending.iter().any(|c| c.peer_id == channel_page4));
}

#[tokio::test]
async fn dialog_discovery_traverses_folders_and_filters() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let custom_channel = PeerId::new(-1_000_000_888_111);

    adapter.set_dialog_filters(vec![DialogFilterRecord {
        id: 7,
        title: "Work Channels".to_string(),
        pinned_peers: vec![custom_channel],
        include_peers: vec![custom_channel],
        exclude_peers: vec![],
    }]);
    adapter.set_channel_pts(custom_channel, 95);

    let sync_engine = Arc::new(IncrementalSyncEngine::new(
        Arc::clone(&adapter),
        Arc::clone(&storage),
    ));
    let worker =
        ChannelQueueWorker::new(Arc::clone(&adapter), Arc::clone(&storage), sync_engine, 1);

    let enqueued = worker.discover_and_enqueue_stale_channels().await.unwrap();
    assert!(enqueued >= 1);

    let pending = storage.list_pending_channels().unwrap();
    assert!(pending.iter().any(|c| c.peer_id == custom_channel));
}

#[tokio::test]
async fn dialog_discovery_deduplicates_pinned_and_normal_dialogs() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let channel_id = PeerId::new(-1_000_000_777_111);

    adapter.set_dialog_pages(
        0,
        vec![DialogsPage {
            dialogs: vec![DialogInfo {
                peer_id: channel_id,
                peer_type: Some(PeerType::Channel),
                pts: Some(50),
                top_message: None,
                unread_count: 0,
                is_pinned: true,
                folder_id: Some(0),
                is_unresolved: false,
            }],
            auxiliary_peers: vec![],
            is_last_page: true,
            next_offset_date: 0,
            next_offset_id: 0,
            next_offset_peer: None,
        }],
    );

    adapter.set_dialog_filters(vec![DialogFilterRecord {
        id: 2,
        title: "Pinned Filter".to_string(),
        pinned_peers: vec![channel_id],
        include_peers: vec![channel_id],
        exclude_peers: vec![],
    }]);

    let sync_engine = Arc::new(IncrementalSyncEngine::new(
        Arc::clone(&adapter),
        Arc::clone(&storage),
    ));
    let worker =
        ChannelQueueWorker::new(Arc::clone(&adapter), Arc::clone(&storage), sync_engine, 1);

    let enqueued = worker.discover_and_enqueue_stale_channels().await.unwrap();
    assert_eq!(enqueued, 1);
}

#[tokio::test]
async fn dialog_discovery_falls_back_to_get_peer_dialogs() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let dormant_channel = PeerId::new(-1_000_000_999_888);

    storage
        .upsert_peer(&PeerRecord {
            peer_id: dormant_channel,
            peer_type: PeerType::Channel,
            name: Some("Dormant Channel".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1600000000,
        })
        .unwrap();

    adapter.set_dialog_pages(
        0,
        vec![DialogsPage {
            dialogs: vec![],
            auxiliary_peers: vec![],
            is_last_page: true,
            next_offset_date: 0,
            next_offset_id: 0,
            next_offset_peer: None,
        }],
    );

    adapter.set_peer_dialog(
        dormant_channel,
        DialogInfo {
            peer_id: dormant_channel,
            peer_type: Some(PeerType::Channel),
            pts: Some(60),
            top_message: None,
            unread_count: 0,
            is_pinned: false,
            folder_id: None,
            is_unresolved: false,
        },
    );

    let sync_engine = Arc::new(IncrementalSyncEngine::new(
        Arc::clone(&adapter),
        Arc::clone(&storage),
    ));
    let worker =
        ChannelQueueWorker::new(Arc::clone(&adapter), Arc::clone(&storage), sync_engine, 1);

    let enqueued = worker.discover_and_enqueue_stale_channels().await.unwrap();
    assert_eq!(enqueued, 1);

    let pending = storage.list_pending_channels().unwrap();
    assert_eq!(pending[0].peer_id, dormant_channel);
}

#[tokio::test]
async fn partial_dialog_enumeration_failure_reports_non_fatal_error() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    adapter.inject_error("RPC Timeout on getDialogs page 3");

    let sync_engine = Arc::new(IncrementalSyncEngine::new(
        Arc::clone(&adapter),
        Arc::clone(&storage),
    ));
    let worker =
        ChannelQueueWorker::new(Arc::clone(&adapter), Arc::clone(&storage), sync_engine, 1);

    let res = worker.discover_and_enqueue_stale_channels().await;
    assert!(res.is_err());
    match res.unwrap_err() {
        SyncError::ChannelDiscoveryIncomplete(_) => {}
        other => panic!("Expected ChannelDiscoveryIncomplete, got {other:?}"),
    }
}

#[tokio::test]
async fn common_too_long_establishes_definitive_state_lock() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let peer_id = PeerId::new(8001);

    storage
        .upsert_peer(&PeerRecord {
            peer_id,
            peer_type: PeerType::User,
            name: Some("User 8001".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    adapter.enqueue_common_difference(CommonDifferenceResult::TooLong { pts: 200_000 });
    adapter.set_account_state(AccountSyncState {
        account_id: "default".to_string(),
        pts: 200_000,
        qts: 15,
        date: 1700005000,
        seq: 300,
        sync_uncertain: false,
        last_synced_at: 1700005000,
    });

    adapter.enqueue_common_difference(CommonDifferenceResult::Slice {
        new_messages: vec![create_dummy_message(peer_id, 10, "Post-recovery Msg 1")],
        other_updates: vec![],
        auxiliary_peers: vec![],
        intermediate_state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 200_005,
            qts: 15,
            date: 1700005010,
            seq: 301,
            sync_uncertain: false,
            last_synced_at: 1700005010,
        },
    });
    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![create_dummy_message(peer_id, 11, "Post-recovery Msg 2")],
        other_updates: vec![],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 200_010,
            qts: 15,
            date: 1700005020,
            seq: 302,
            sync_uncertain: false,
            last_synced_at: 1700005020,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = engine.sync_common().await.unwrap();

    assert_eq!(summary.final_pts, 200_010);
    assert!(summary.had_buffer_overflow);
    let state = storage.get_account_sync_state("default").unwrap().unwrap();
    assert_eq!(state.pts, 200_010);
    assert_eq!(storage.count_messages_by_peer(peer_id).unwrap(), 2);
}

#[tokio::test]
async fn channel_too_long_establishes_definitive_state_lock() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let channel_id = PeerId::new(-1_000_000_444_333);

    let msg = create_dummy_message(channel_id, 200, "TooLong Batch Post");
    adapter.enqueue_channel_difference(
        channel_id,
        ChannelDifferenceResult::TooLong {
            final_state: true,
            timeout: Some(30),
            dialog_pts: 50_000,
            top_message: Some(MessageId::new(200)),
            messages: vec![msg.clone()],
            auxiliary_peers: vec![],
        },
    );

    let msg_during_rescan = create_dummy_message(channel_id, 201, "Posted During Rescan");
    adapter.enqueue_channel_difference(
        channel_id,
        ChannelDifferenceResult::Difference {
            final_state: true,
            pts: 50_005,
            timeout: Some(30),
            new_messages: vec![msg_during_rescan.clone()],
            other_updates: vec![],
            auxiliary_peers: vec![],
        },
    );

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let res = engine.sync_channel(channel_id, Some(10_000)).await.unwrap();
    let state = res.state;

    assert_eq!(state.pts, Some(50_005));
    let stored1 = storage.get_message(msg.key).unwrap().unwrap();
    assert_eq!(stored1.text, Some("TooLong Batch Post".to_string()));
    let stored2 = storage.get_message(msg_during_rescan.key).unwrap().unwrap();
    assert_eq!(stored2.text, Some("Posted During Rescan".to_string()));
}

#[tokio::test]
async fn channel_queue_persists_across_restart_and_flood_wait() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let channel_id = PeerId::new(-1_000_000_111_222);

    storage
        .enqueue_channel(&ChannelQueueItem {
            peer_id: channel_id,
            discovered_pts: 100,
            current_pts: Some(50),
            status: ChannelQueueStatus::InProgress,
            attempts: 1,
            poll_timeout: None,
            last_error: None,
            updated_at: 1700000000,
        })
        .unwrap();

    adapter.inject_flood_wait(channel_id, 1);

    adapter.enqueue_channel_difference(
        channel_id,
        ChannelDifferenceResult::Difference {
            final_state: true,
            pts: 100,
            timeout: Some(30),
            new_messages: vec![create_dummy_message(channel_id, 5, "Recovered Post")],
            other_updates: vec![],
            auxiliary_peers: vec![],
        },
    );

    let sync_engine = Arc::new(IncrementalSyncEngine::new(
        Arc::clone(&adapter),
        Arc::clone(&storage),
    ));
    let worker = ChannelQueueWorker::new_with_scale(
        Arc::clone(&adapter),
        Arc::clone(&storage),
        sync_engine,
        0.001,
    );

    let synced = worker.process_queue().await.unwrap();
    assert_eq!(synced, 1);

    let pending = storage.list_pending_channels().unwrap();
    assert!(pending.is_empty());
}

#[tokio::test]
async fn common_deletion_tombstones_never_use_peer_id_zero() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![NormalizedUpdate::CommonDeletedMessages {
            message_ids: vec![MessageId::new(999)],
            pts: Some(105),
            pts_count: 1,
        }],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 105,
            qts: 10,
            date: 1700000100,
            seq: 2,
            sync_uncertain: false,
            last_synced_at: 1700000100,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    engine.sync_common().await.unwrap();

    let peer0_msg = storage.get_message(MessageKey::new(0, 999)).unwrap();
    assert!(peer0_msg.is_none());

    let tombstones = storage.get_common_deletion_tombstones().unwrap();
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].message_id, MessageId::new(999));
}

#[tokio::test]
async fn channel_messages_are_isolated_from_common_delete_with_same_id() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let user_peer = PeerId::new(1001);
    let channel_peer = PeerId::new(-1_000_000_999_999);

    storage
        .upsert_peer(&PeerRecord {
            peer_id: user_peer,
            peer_type: PeerType::User,
            name: Some("User 1001".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    storage
        .upsert_peer(&PeerRecord {
            peer_id: channel_peer,
            peer_type: PeerType::Channel,
            name: Some("Channel 999999".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    let user_msg = create_dummy_message(user_peer, 500, "User msg 500");
    let channel_msg = create_dummy_message(channel_peer, 500, "Channel msg 500");
    storage.insert_or_update_message(&user_msg).unwrap();
    storage.insert_or_update_message(&channel_msg).unwrap();

    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![NormalizedUpdate::CommonDeletedMessages {
            message_ids: vec![MessageId::new(500)],
            pts: Some(105),
            pts_count: 1,
        }],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 105,
            qts: 10,
            date: 1700000100,
            seq: 2,
            sync_uncertain: false,
            last_synced_at: 1700000100,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    engine.sync_common().await.unwrap();

    let stored_user = storage.get_message(user_msg.key).unwrap().unwrap();
    assert_eq!(stored_user.state, MessageState::Deleted);

    let stored_chan = storage.get_message(channel_msg.key).unwrap().unwrap();
    assert_eq!(stored_chan.state, MessageState::Active);
}

#[tokio::test]
async fn custom_dialog_filters_honor_excluded_peers() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let channel_inc = PeerId::new(-1_000_000_111_001);
    let channel_exc = PeerId::new(-1_000_000_111_002);

    adapter.set_dialog_filters(vec![DialogFilterRecord {
        id: 5,
        title: "Filtered Work".to_string(),
        pinned_peers: vec![channel_inc],
        include_peers: vec![channel_inc, channel_exc],
        exclude_peers: vec![channel_exc],
    }]);

    adapter.set_channel_pts(channel_inc, 80);
    adapter.set_channel_pts(channel_exc, 90);

    let sync_engine = Arc::new(IncrementalSyncEngine::new(
        Arc::clone(&adapter),
        Arc::clone(&storage),
    ));
    let worker =
        ChannelQueueWorker::new(Arc::clone(&adapter), Arc::clone(&storage), sync_engine, 1);

    let enqueued = worker.discover_and_enqueue_stale_channels().await.unwrap();
    assert_eq!(enqueued, 1);

    let pending = storage.list_pending_channels().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].peer_id, channel_inc);
    assert!(!pending.iter().any(|c| c.peer_id == channel_exc));
}

#[tokio::test]
async fn production_workers_use_standard_flood_wait_defaults() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let sync_engine = Arc::new(IncrementalSyncEngine::new(
        Arc::clone(&adapter),
        Arc::clone(&storage),
    ));

    let prod_worker =
        ChannelQueueWorker::new(Arc::clone(&adapter), Arc::clone(&storage), sync_engine, 1);

    assert_eq!(prod_worker.max_concurrency(), 1);
    assert_eq!(prod_worker.flood_wait_scale(), 1.0);
}

#[tokio::test]
async fn baseline_ids_remain_distinct_across_multiple_runs() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    adapter.set_account_state(AccountSyncState {
        account_id: "default".to_string(),
        pts: 888,
        qts: 10,
        date: 1700000000,
        seq: 1,
        sync_uncertain: false,
        last_synced_at: 1700000000,
    });

    let pipeline = CoordinatedSyncPipeline::new(Arc::clone(&adapter), Arc::clone(&storage));

    let mut baseline_ids = HashSet::new();
    for _ in 0..5 {
        pipeline.run_full_sync(&[]).await.unwrap();
    }

    storage
        .with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT baseline_id FROM sync_baseline;")?;
            let ids: Vec<String> = stmt
                .query_map([], |row| row.get(0))?
                .filter_map(Result::ok)
                .collect();
            assert_eq!(ids.len(), 5);
            for id in ids {
                assert!(baseline_ids.insert(id));
            }
            Ok(())
        })
        .unwrap();
}

#[tokio::test]
async fn duplicate_local_common_id_anomaly_is_detected() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let user_a = PeerId::new(1001);
    let user_b = PeerId::new(1002);

    storage
        .upsert_peer(&PeerRecord {
            peer_id: user_a,
            peer_type: PeerType::User,
            name: Some("User A".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    storage
        .upsert_peer(&PeerRecord {
            peer_id: user_b,
            peer_type: PeerType::User,
            name: Some("User B".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    let msg_a = create_dummy_message(user_a, 777, "User A Msg 777");
    let msg_b = create_dummy_message(user_b, 777, "User B Msg 777");
    storage.insert_or_update_message(&msg_a).unwrap();
    storage.insert_or_update_message(&msg_b).unwrap();

    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![NormalizedUpdate::CommonDeletedMessages {
            message_ids: vec![MessageId::new(777)],
            pts: Some(105),
            pts_count: 1,
        }],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 105,
            qts: 10,
            date: 1700000100,
            seq: 2,
            sync_uncertain: false,
            last_synced_at: 1700000100,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    engine.sync_common().await.unwrap();

    let stored_a = storage.get_message(msg_a.key).unwrap().unwrap();
    let stored_b = storage.get_message(msg_b.key).unwrap().unwrap();
    assert_eq!(stored_a.state, MessageState::Active);
    assert_eq!(stored_b.state, MessageState::Active);

    let tombstones = storage.get_common_deletion_tombstones().unwrap();
    assert!(
        tombstones
            .iter()
            .any(|t| t.message_id == MessageId::new(777))
    );
}

#[tokio::test]
async fn missing_peer_metadata_common_delete_uses_tombstone() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let orphan_peer = PeerId::new(9999);
    let orphan_msg = create_dummy_message(orphan_peer, 888, "Orphan Message");
    storage.insert_or_update_message(&orphan_msg).unwrap();

    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![NormalizedUpdate::CommonDeletedMessages {
            message_ids: vec![MessageId::new(888)],
            pts: Some(105),
            pts_count: 1,
        }],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 105,
            qts: 10,
            date: 1700000100,
            seq: 2,
            sync_uncertain: false,
            last_synced_at: 1700000100,
        },
    });

    let engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    engine.sync_common().await.unwrap();

    let stored = storage.get_message(orphan_msg.key).unwrap().unwrap();
    assert_eq!(stored.state, MessageState::Active);

    let tombstones = storage.get_common_deletion_tombstones().unwrap();
    assert!(
        tombstones
            .iter()
            .any(|t| t.message_id == MessageId::new(888))
    );
}

#[tokio::test]
async fn filter_lookup_failure_marks_discovery_incomplete_and_enqueues_retry() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let channel_id = PeerId::new(-1_000_000_555_001);
    adapter.set_dialog_filters(vec![DialogFilterRecord {
        id: 1,
        title: "Filter".to_string(),
        pinned_peers: vec![channel_id],
        include_peers: vec![channel_id],
        exclude_peers: vec![],
    }]);

    adapter.inject_error("Network Timeout on getPeerDialogs");

    let sync_engine = Arc::new(IncrementalSyncEngine::new(
        Arc::clone(&adapter),
        Arc::clone(&storage),
    ));
    let worker =
        ChannelQueueWorker::new(Arc::clone(&adapter), Arc::clone(&storage), sync_engine, 1);

    let res = worker.discover_and_enqueue_stale_channels().await;
    assert!(res.is_err());

    let report = storage
        .get_latest_sync_integrity_report("channel_discovery")
        .unwrap()
        .expect("Integrity report for channel_discovery must exist");
    assert!(!report.channel_discovery_complete);
}

#[tokio::test]
async fn channel_sync_and_queue_commit_atomically() {
    let (_dir, storage) = create_test_db();
    let channel_id = PeerId::new(-1_000_000_333_222);

    storage
        .enqueue_channel(&ChannelQueueItem {
            peer_id: channel_id,
            discovered_pts: 500,
            current_pts: Some(100),
            status: ChannelQueueStatus::InProgress,
            attempts: 1,
            poll_timeout: None,
            last_error: None,
            updated_at: 1700000000,
        })
        .unwrap();

    storage
        .complete_channel_sync_and_queue(channel_id, 500, Some(30), 1700000100)
        .unwrap();

    let state = storage.get_peer_sync_state(channel_id).unwrap().unwrap();
    assert_eq!(state.pts, Some(500));
    assert!(!state.sync_uncertain);
    assert_eq!(state.poll_timeout_secs, Some(30));

    let pending = storage.list_pending_channels().unwrap();
    assert!(pending.is_empty());
}

#[tokio::test]
async fn dormant_channel_pass_checks_local_archive_despite_filter_exclude() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let channel_excluded_only = PeerId::new(-1_000_000_999_111);
    let channel_dormant_archived = PeerId::new(-1_000_000_999_222);

    storage
        .upsert_peer(&PeerRecord {
            peer_id: channel_dormant_archived,
            peer_type: PeerType::Channel,
            name: Some("Archived Dormant Channel".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1600000000,
        })
        .unwrap();

    adapter.set_dialog_filters(vec![DialogFilterRecord {
        id: 3,
        title: "Work".to_string(),
        pinned_peers: vec![],
        include_peers: vec![channel_excluded_only, channel_dormant_archived],
        exclude_peers: vec![channel_excluded_only, channel_dormant_archived],
    }]);

    adapter.set_peer_dialog(
        channel_dormant_archived,
        DialogInfo {
            peer_id: channel_dormant_archived,
            peer_type: Some(PeerType::Channel),
            pts: Some(50),
            top_message: None,
            unread_count: 0,
            is_pinned: false,
            folder_id: None,
            is_unresolved: false,
        },
    );

    let sync_engine = Arc::new(IncrementalSyncEngine::new(
        Arc::clone(&adapter),
        Arc::clone(&storage),
    ));
    let worker =
        ChannelQueueWorker::new(Arc::clone(&adapter), Arc::clone(&storage), sync_engine, 1);

    let enqueued = worker.discover_and_enqueue_stale_channels().await.unwrap();
    assert_eq!(enqueued, 1);

    let pending = storage.list_pending_channels().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].peer_id, channel_dormant_archived);
    assert!(!pending.iter().any(|c| c.peer_id == channel_excluded_only));
}

#[tokio::test]
async fn unsupported_events_and_uncertainty_commit_atomically() {
    let (_dir, storage) = create_test_db();
    let safe_state = AccountSyncState {
        account_id: "default".to_string(),
        pts: 100,
        qts: 10,
        date: 1700000000,
        seq: 5,
        sync_uncertain: false,
        last_synced_at: 1700000000,
    };
    storage.upsert_account_sync_state(&safe_state).unwrap();

    storage
        .persist_account_unsupported_event_and_mark_uncertain(
            0xabcdef01,
            Some(105),
            Some(5),
            None,
            None,
            Some("Unknown constructor"),
            &[1, 2, 3, 4],
            &safe_state,
            1700000010,
        )
        .unwrap();

    let events = storage.list_unsupported_events().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].constructor_id, 0xabcdef01);
    assert_eq!(events[0].pts, Some(105));
    assert!(events[0].affects_sync_state);

    let updated_account = storage.get_account_sync_state("default").unwrap().unwrap();
    assert_eq!(updated_account.pts, 100);
    assert!(updated_account.sync_uncertain);

    let channel_id = PeerId::new(-1_000_000_123_456);
    storage
        .persist_channel_unsupported_event_and_mark_uncertain(
            channel_id,
            0xabcdef02,
            Some(300),
            Some(10),
            None,
            None,
            Some("Channel unknown constructor"),
            &[5, 6, 7, 8],
            250,
            Some(60),
            1700000020,
        )
        .unwrap();

    let channel_state = storage.get_peer_sync_state(channel_id).unwrap().unwrap();
    assert_eq!(channel_state.pts, Some(250));
    assert!(channel_state.sync_uncertain);
}

#[tokio::test]
async fn unresolved_channel_is_marked_blocked_and_never_synced_with_pts_1() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let unresolved_channel = PeerId::new(-1_000_000_777_888);

    storage
        .upsert_peer(&PeerRecord {
            peer_id: unresolved_channel,
            peer_type: PeerType::Channel,
            name: Some("Unresolved Channel".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    adapter.set_dialog_pages(
        0,
        vec![DialogsPage {
            dialogs: vec![DialogInfo {
                peer_id: unresolved_channel,
                peer_type: Some(PeerType::Channel),
                pts: None,
                top_message: None,
                unread_count: 0,
                is_pinned: false,
                folder_id: None,
                is_unresolved: true,
            }],
            auxiliary_peers: vec![],
            is_last_page: true,
            next_offset_date: 0,
            next_offset_id: 0,
            next_offset_peer: None,
        }],
    );

    let sync_engine = Arc::new(IncrementalSyncEngine::new(
        Arc::clone(&adapter),
        Arc::clone(&storage),
    ));
    let worker =
        ChannelQueueWorker::new(Arc::clone(&adapter), Arc::clone(&storage), sync_engine, 1);

    let enqueued = worker.discover_and_enqueue_stale_channels().await.unwrap();
    assert_eq!(enqueued, 0);

    let blocked = storage.list_blocked_channels().unwrap();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].peer_id, unresolved_channel);
    assert_eq!(blocked[0].status, ChannelQueueStatus::Blocked);

    let pending = storage.list_pending_channels().unwrap();
    assert!(pending.is_empty());

    let processed = worker.process_queue().await.unwrap();
    assert_eq!(processed, 0);

    let state = storage.get_peer_sync_state(unresolved_channel).unwrap();
    assert!(state.is_none());
}

#[tokio::test]
async fn fresh_channel_discovery_establishes_baseline_without_pts_1() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());
    let fresh_channel = PeerId::new(-1_000_000_555_444);

    storage
        .upsert_peer(&PeerRecord {
            peer_id: fresh_channel,
            peer_type: PeerType::Channel,
            name: Some("Fresh Channel".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    adapter.set_channel_pts(fresh_channel, 500000);
    adapter.set_dialog_pages(
        0,
        vec![DialogsPage {
            dialogs: vec![DialogInfo {
                peer_id: fresh_channel,
                peer_type: Some(PeerType::Channel),
                pts: Some(500000),
                top_message: None,
                unread_count: 0,
                is_pinned: false,
                folder_id: None,
                is_unresolved: false,
            }],
            auxiliary_peers: vec![],
            is_last_page: true,
            next_offset_date: 0,
            next_offset_id: 0,
            next_offset_peer: None,
        }],
    );

    let sync_engine = Arc::new(IncrementalSyncEngine::new(
        Arc::clone(&adapter),
        Arc::clone(&storage),
    ));
    let worker =
        ChannelQueueWorker::new(Arc::clone(&adapter), Arc::clone(&storage), sync_engine, 1);

    let enqueued = worker.discover_and_enqueue_stale_channels().await.unwrap();
    assert_eq!(enqueued, 1);

    let pending = storage.list_pending_channels().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].peer_id, fresh_channel);
    assert_eq!(pending[0].discovered_pts, 500000);
    assert_eq!(pending[0].current_pts, Some(500000));

    let processed = worker.process_queue().await.unwrap();
    assert_eq!(processed, 1);

    let state = storage.get_peer_sync_state(fresh_channel).unwrap().unwrap();
    assert_eq!(state.pts, Some(500000));
}

#[tokio::test]
async fn channel_discovery_uses_canonical_peer_type_not_numeric_range() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let channel_with_positive_id = PeerId::new(999888);
    let user_with_negative_id = PeerId::new(-1_000_000_111_222);

    adapter.set_dialog_pages(
        0,
        vec![DialogsPage {
            dialogs: vec![
                DialogInfo {
                    peer_id: channel_with_positive_id,
                    peer_type: Some(PeerType::Channel),
                    pts: Some(200),
                    top_message: None,
                    unread_count: 0,
                    is_pinned: false,
                    folder_id: None,
                    is_unresolved: false,
                },
                DialogInfo {
                    peer_id: user_with_negative_id,
                    peer_type: Some(PeerType::User),
                    pts: Some(300),
                    top_message: None,
                    unread_count: 0,
                    is_pinned: false,
                    folder_id: None,
                    is_unresolved: false,
                },
            ],
            auxiliary_peers: vec![],
            is_last_page: true,
            next_offset_date: 0,
            next_offset_id: 0,
            next_offset_peer: None,
        }],
    );

    let sync_engine = Arc::new(IncrementalSyncEngine::new(
        Arc::clone(&adapter),
        Arc::clone(&storage),
    ));
    let worker =
        ChannelQueueWorker::new(Arc::clone(&adapter), Arc::clone(&storage), sync_engine, 1);

    let enqueued = worker.discover_and_enqueue_stale_channels().await.unwrap();
    assert_eq!(enqueued, 1);

    let pending = storage.list_pending_channels().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].peer_id, channel_with_positive_id);
    assert!(!pending.iter().any(|c| c.peer_id == user_with_negative_id));
}

#[tokio::test]
async fn common_delete_and_cursor_commit_atomically() {
    let (_dir, storage) = create_test_db();
    let user_id = PeerId::new(42);

    storage
        .upsert_peer(&PeerRecord {
            peer_id: user_id,
            peer_type: PeerType::User,
            name: Some("Alice".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    let msg = create_dummy_message(user_id, 100, "Active Message");
    storage
        .insert_messages_batch(std::slice::from_ref(&msg))
        .unwrap();

    let intermediate_state = AccountSyncState {
        account_id: "default".to_string(),
        pts: 200,
        qts: 10,
        date: 1700000000,
        seq: 5,
        sync_uncertain: false,
        last_synced_at: 1700000000,
    };

    let delete_update = NormalizedUpdate::CommonDeletedMessages {
        message_ids: vec![MessageId::new(100)],
        pts: Some(200),
        pts_count: 1,
    };

    storage
        .apply_common_difference_slice(&[], &[delete_update], &[], &intermediate_state)
        .unwrap();

    let stored_msg = storage.get_message(msg.key).unwrap().unwrap();
    assert_eq!(stored_msg.state, MessageState::Deleted);

    let account_state = storage.get_account_sync_state("default").unwrap().unwrap();
    assert_eq!(account_state.pts, 200);
}

#[tokio::test]
async fn qts_deferred_state_persists_across_restart_without_pts_advance() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let initial = AccountSyncState {
        account_id: "default".to_string(),
        pts: 100,
        qts: 10,
        date: 1700000000,
        seq: 1,
        sync_uncertain: false,
        last_synced_at: 1700000000,
    };
    storage.upsert_account_sync_state(&initial).unwrap();
    adapter.set_account_state(initial);

    let qts_update = NormalizedUpdate::Unsupported {
        constructor_name: "updateEncryptedMessagesRead".to_string(),
        constructor_id: 0x38fe25b6,
        affects_sync_state: true,
        pts: None,
        pts_count: 0,
        qts: Some(15),
        qts_count: 5,
        diagnostic_info: Some("Secret chat QTS update".to_string()),
        raw_tl: vec![0x11, 0x22],
    };

    adapter.enqueue_common_difference(CommonDifferenceResult::Difference {
        new_messages: vec![],
        other_updates: vec![qts_update],
        auxiliary_peers: vec![],
        state: AccountSyncState {
            account_id: "default".to_string(),
            pts: 100,
            qts: 15,
            date: 1700000000,
            seq: 2,
            sync_uncertain: false,
            last_synced_at: 1700000000,
        },
    });

    let sync_engine = IncrementalSyncEngine::new(Arc::clone(&adapter), Arc::clone(&storage));
    let res = sync_engine.sync_common().await;

    assert!(res.is_err());
    match res.unwrap_err() {
        SyncError::UnsupportedStateAffectingUpdate {
            constructor_id,
            pts,
            ..
        } => {
            assert_eq!(constructor_id, 0x38fe25b6);
            assert_eq!(pts, None);
        }
        other => panic!("Unexpected error: {other:?}"),
    }

    let state = storage.get_account_sync_state("default").unwrap().unwrap();
    assert_eq!(state.pts, 100);
    assert_eq!(state.qts, 10);
    assert!(state.sync_uncertain);

    let events = storage.list_unsupported_events().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].constructor_id, 0x38fe25b6);
    assert_eq!(events[0].qts, Some(15));
}

#[tokio::test]
async fn full_sync_integrity_flags_false_when_unresolved_channels_exist() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let initial = AccountSyncState {
        account_id: "default".to_string(),
        pts: 100,
        qts: 10,
        date: 1700000000,
        seq: 1,
        sync_uncertain: false,
        last_synced_at: 1700000000,
    };
    adapter.set_account_state(initial);
    adapter.enqueue_common_difference(CommonDifferenceResult::Empty {
        date: 1700000000,
        seq: 1,
    });

    let channel_unresolved = PeerId::new(-1_000_000_999_888);
    storage
        .upsert_peer(&PeerRecord {
            peer_id: channel_unresolved,
            peer_type: PeerType::Channel,
            name: Some("Unresolved Channel".to_string()),
            username: None,
            phone: None,
            raw_tl: None,
            updated_at: 1700000000,
        })
        .unwrap();

    adapter.set_dialog_pages(
        0,
        vec![DialogsPage {
            dialogs: vec![DialogInfo {
                peer_id: channel_unresolved,
                peer_type: Some(PeerType::Channel),
                pts: None,
                top_message: None,
                unread_count: 0,
                is_pinned: false,
                folder_id: None,
                is_unresolved: true,
            }],
            auxiliary_peers: vec![],
            is_last_page: true,
            next_offset_date: 0,
            next_offset_id: 0,
            next_offset_peer: None,
        }],
    );

    let pipeline = CoordinatedSyncPipeline::new(Arc::clone(&adapter), Arc::clone(&storage));
    let summary = pipeline.run_full_sync(&[]).await.unwrap();

    let integrity = summary
        .integrity
        .expect("Integrity report must be generated");
    assert!(!integrity.fully_lossless_contiguous_sync);
    assert!(!integrity.channel_discovery_complete);
}

#[tokio::test]
async fn get_messages_and_reply_routing_uses_canonical_peer_type() {
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let channel_peer = PeerId::new(77777);
    adapter.add_peer(PeerRecord {
        peer_id: channel_peer,
        peer_type: PeerType::Channel,
        name: Some("Positive ID Channel".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    });
    adapter.add_message(create_dummy_message(channel_peer, 1, "Channel Reply Root"));

    let user_peer = PeerId::new(-1_000_000_888_999);
    adapter.add_peer(PeerRecord {
        peer_id: user_peer,
        peer_type: PeerType::User,
        name: Some("Negative ID User".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    });
    adapter.add_message(create_dummy_message(user_peer, 2, "User Reply Root"));

    let channel_target = adapter
        .resolve_reply_target(
            channel_peer,
            Some(PeerType::Channel),
            None,
            None,
            MessageId::new(1),
        )
        .await
        .unwrap()
        .expect("channel target message found");
    assert_eq!(channel_target.text.as_deref(), Some("Channel Reply Root"));
    assert_eq!(adapter.channels_get_messages_calls.lock().unwrap().len(), 1);
    assert_eq!(adapter.messages_get_messages_calls.lock().unwrap().len(), 0);

    let user_target = adapter
        .resolve_reply_target(
            user_peer,
            Some(PeerType::User),
            None,
            None,
            MessageId::new(2),
        )
        .await
        .unwrap()
        .expect("user target message found");
    assert_eq!(user_target.text.as_deref(), Some("User Reply Root"));
    assert_eq!(adapter.channels_get_messages_calls.lock().unwrap().len(), 1);
    assert_eq!(adapter.messages_get_messages_calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn explicit_single_peer_scope_ignores_unrelated_channels() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let target_channel = PeerId::new(-1003563998964);
    let other_channel_1 = PeerId::new(-1001766138888);
    let other_channel_2 = PeerId::new(-1001411910424);

    adapter.add_peer(PeerRecord {
        peer_id: target_channel,
        peer_type: PeerType::Channel,
        name: Some("Target Group".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    });
    adapter.add_peer(PeerRecord {
        peer_id: other_channel_1,
        peer_type: PeerType::Channel,
        name: Some("Unrelated Channel 1".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    });
    adapter.add_peer(PeerRecord {
        peer_id: other_channel_2,
        peer_type: PeerType::Channel,
        name: Some("Unrelated Channel 2".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    });

    adapter.set_dialog_pages(
        0,
        vec![DialogsPage {
            dialogs: vec![
                DialogInfo {
                    peer_id: target_channel,
                    peer_type: Some(PeerType::Channel),
                    pts: Some(100),
                    top_message: None,
                    unread_count: 0,
                    is_pinned: false,
                    folder_id: Some(0),
                    is_unresolved: false,
                },
                DialogInfo {
                    peer_id: other_channel_1,
                    peer_type: Some(PeerType::Channel),
                    pts: Some(200),
                    top_message: None,
                    unread_count: 0,
                    is_pinned: false,
                    folder_id: Some(0),
                    is_unresolved: false,
                },
                DialogInfo {
                    peer_id: other_channel_2,
                    peer_type: Some(PeerType::Channel),
                    pts: Some(300),
                    top_message: None,
                    unread_count: 0,
                    is_pinned: false,
                    folder_id: Some(0),
                    is_unresolved: false,
                },
            ],
            auxiliary_peers: vec![],
            is_last_page: true,
            next_offset_date: 0,
            next_offset_id: 0,
            next_offset_peer: None,
        }],
    );

    let pipeline = CoordinatedSyncPipeline::new(Arc::clone(&adapter), Arc::clone(&storage));

    let summary = pipeline
        .run_full_sync_with_scope(&[target_channel], true, |_| {})
        .await
        .unwrap();

    assert_eq!(summary.requested_peers_count, 1);
    assert_eq!(summary.channels_synchronized, 1);
    assert!(summary.failed_channels.is_empty());
    assert!(summary.is_clean());
    assert!(summary.is_requested_scope_clean());

    let state_1 = storage.get_peer_sync_state(other_channel_1).unwrap();
    assert!(state_1.is_none());
    let state_2 = storage.get_peer_sync_state(other_channel_2).unwrap();
    assert!(state_2.is_none());
}

#[tokio::test]
async fn explicit_multi_peer_scope_only_processes_requested_peers() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let target_user = PeerId::new(12345);
    let target_channel = PeerId::new(-100222333444);
    let out_of_scope_channel = PeerId::new(-100999888777);

    adapter.add_peer(PeerRecord {
        peer_id: target_user,
        peer_type: PeerType::User,
        name: Some("Target User".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    });
    adapter.add_peer(PeerRecord {
        peer_id: target_channel,
        peer_type: PeerType::Channel,
        name: Some("Target Channel".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    });
    adapter.add_peer(PeerRecord {
        peer_id: out_of_scope_channel,
        peer_type: PeerType::Channel,
        name: Some("Out of Scope Channel".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    });

    adapter.set_dialog_pages(
        0,
        vec![DialogsPage {
            dialogs: vec![
                DialogInfo {
                    peer_id: target_channel,
                    peer_type: Some(PeerType::Channel),
                    pts: Some(500),
                    top_message: None,
                    unread_count: 0,
                    is_pinned: false,
                    folder_id: Some(0),
                    is_unresolved: false,
                },
                DialogInfo {
                    peer_id: out_of_scope_channel,
                    peer_type: Some(PeerType::Channel),
                    pts: Some(600),
                    top_message: None,
                    unread_count: 0,
                    is_pinned: false,
                    folder_id: Some(0),
                    is_unresolved: false,
                },
            ],
            auxiliary_peers: vec![],
            is_last_page: true,
            next_offset_date: 0,
            next_offset_id: 0,
            next_offset_peer: None,
        }],
    );

    let pipeline = CoordinatedSyncPipeline::new(Arc::clone(&adapter), Arc::clone(&storage));

    let summary = pipeline
        .run_full_sync_with_scope(&[target_user, target_channel], true, |_| {})
        .await
        .unwrap();

    assert_eq!(summary.requested_peers_count, 2);
    assert_eq!(summary.channels_synchronized, 1);
    assert!(summary.failed_channels.is_empty());
    assert!(summary.is_clean());

    let out_of_scope_state = storage.get_peer_sync_state(out_of_scope_channel).unwrap();
    assert!(out_of_scope_state.is_none());
}

#[tokio::test]
async fn global_sync_discovers_and_processes_all_channels() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let channel_1 = PeerId::new(-100111111);
    let channel_2 = PeerId::new(-100222222);

    adapter.set_dialog_pages(
        0,
        vec![DialogsPage {
            dialogs: vec![
                DialogInfo {
                    peer_id: channel_1,
                    peer_type: Some(PeerType::Channel),
                    pts: Some(10),
                    top_message: None,
                    unread_count: 0,
                    is_pinned: false,
                    folder_id: Some(0),
                    is_unresolved: false,
                },
                DialogInfo {
                    peer_id: channel_2,
                    peer_type: Some(PeerType::Channel),
                    pts: Some(20),
                    top_message: None,
                    unread_count: 0,
                    is_pinned: false,
                    folder_id: Some(0),
                    is_unresolved: false,
                },
            ],
            auxiliary_peers: vec![],
            is_last_page: true,
            next_offset_date: 0,
            next_offset_id: 0,
            next_offset_peer: None,
        }],
    );

    let pipeline = CoordinatedSyncPipeline::new(Arc::clone(&adapter), Arc::clone(&storage));

    let summary = pipeline
        .run_full_sync_with_scope(&[], false, |_| {})
        .await
        .unwrap();

    assert_eq!(summary.channels_synchronized, 2);
    assert!(summary.failed_channels.is_empty());
    assert!(summary.is_clean());
    assert!(storage.get_peer_sync_state(channel_1).unwrap().is_some());
    assert!(storage.get_peer_sync_state(channel_2).unwrap().is_some());
}

#[tokio::test]
async fn channel_queue_reports_failures_and_flags_integrity_false() {
    let (_dir, storage) = create_test_db();
    let adapter = Arc::new(FakeTelegramAdapter::new());

    let failing_channel = PeerId::new(-100999999);

    adapter.set_dialog_pages(
        0,
        vec![DialogsPage {
            dialogs: vec![DialogInfo {
                peer_id: failing_channel,
                peer_type: Some(PeerType::Channel),
                pts: Some(50),
                top_message: None,
                unread_count: 0,
                is_pinned: false,
                folder_id: Some(0),
                is_unresolved: false,
            }],
            auxiliary_peers: vec![],
            is_last_page: true,
            next_offset_date: 0,
            next_offset_id: 0,
            next_offset_peer: None,
        }],
    );

    adapter.inject_channel_error(failing_channel, "Simulated channel RPC failure");

    let pipeline = CoordinatedSyncPipeline::new(Arc::clone(&adapter), Arc::clone(&storage));

    let summary = pipeline
        .run_full_sync_with_scope(&[failing_channel], true, |_| {})
        .await
        .unwrap();

    assert_eq!(summary.failed_channels.len(), 1);
    assert_eq!(summary.failed_channels[0].0, failing_channel);
    assert!(
        summary.failed_channels[0]
            .1
            .contains("Simulated channel RPC failure")
    );
    assert!(!summary.is_clean());
    assert!(!summary.is_requested_scope_clean());
}
