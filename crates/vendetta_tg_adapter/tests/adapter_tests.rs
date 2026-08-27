use vendetta_model::{
    MessageId, MessageKey, MessageRecord, MessageState, PeerId, PeerRecord, PeerType,
};
use vendetta_tg_adapter::{FakeTelegramAdapter, TelegramAdapter};

#[tokio::test]
async fn fake_adapter_simulates_history_and_replies() {
    let adapter = FakeTelegramAdapter::new();

    let peer = PeerRecord {
        peer_id: PeerId::new(100),
        peer_type: PeerType::Group,
        name: Some("Test Group".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    adapter.add_peer(peer.clone());

    let dialogs = adapter.get_dialogs().await.expect("failed to get dialogs");
    assert_eq!(dialogs.len(), 1);
    assert_eq!(dialogs[0].name.as_deref(), Some("Test Group"));

    for i in 1..=10 {
        let msg = MessageRecord {
            key: MessageKey::new(100, i),
            date: 1700000000 + i,
            sender_id: Some(PeerId::new(100)),
            text: Some(format!("Message {i}")),
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: if i > 1 { Some(MessageId::new(1)) } else { None },
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: None,
            forward_json: None,
            reactions_json: None,
            views: None,
            forwards_count: None,
            raw_tl: None,
        };
        adapter.add_message(msg);
    }

    let page1 = adapter
        .get_history(PeerId::new(100), 5, None)
        .await
        .expect("get_history page 1 failed");
    assert_eq!(page1.len(), 5);
    assert_eq!(page1[0].key.message_id, MessageId::new(10));
    assert_eq!(page1[4].key.message_id, MessageId::new(6));

    let page2 = adapter
        .get_history(PeerId::new(100), 5, Some(MessageId::new(6)))
        .await
        .expect("get_history page 2 failed");
    assert_eq!(page2.len(), 5);
    assert_eq!(page2[0].key.message_id, MessageId::new(5));
    assert_eq!(page2[4].key.message_id, MessageId::new(1));

    let target = adapter
        .resolve_reply_target(
            PeerId::new(100),
            Some(PeerType::Group),
            None,
            None,
            MessageId::new(1),
        )
        .await
        .expect("reply resolve failed")
        .expect("target message not found");
    assert_eq!(target.text.as_deref(), Some("Message 1"));
}

#[tokio::test]
async fn get_messages_routes_correctly_by_canonical_peer_type() {
    use vendetta_tg_adapter::AdapterError;

    let adapter = FakeTelegramAdapter::new();

    let positive_channel_id = PeerId::new(55555);
    adapter.add_peer(PeerRecord {
        peer_id: positive_channel_id,
        peer_type: PeerType::Channel,
        name: Some("Positive Channel".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    });
    adapter.add_message(MessageRecord {
        key: MessageKey::new(positive_channel_id.raw(), 1),
        date: 1700000001,
        sender_id: Some(positive_channel_id),
        text: Some("Channel Message".to_string()),
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

    let channel_like_user_id = PeerId::new(-1_000_000_123_456);
    adapter.add_peer(PeerRecord {
        peer_id: channel_like_user_id,
        peer_type: PeerType::User,
        name: Some("Channel-Looking User".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    });
    adapter.add_message(MessageRecord {
        key: MessageKey::new(channel_like_user_id.raw(), 1),
        date: 1700000001,
        sender_id: Some(channel_like_user_id),
        text: Some("User Message".to_string()),
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

    let msgs = adapter
        .get_messages(positive_channel_id, None, &[MessageId::new(1)])
        .await
        .expect("lookup should succeed");
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].text.as_deref(), Some("Channel Message"));

    let channel_calls = adapter.channels_get_messages_calls.lock().unwrap().clone();
    assert_eq!(channel_calls.len(), 1);
    assert_eq!(channel_calls[0].0, positive_channel_id);

    let user_calls = adapter.messages_get_messages_calls.lock().unwrap().clone();
    assert_eq!(user_calls.len(), 0);

    let msgs_user = adapter
        .get_messages(channel_like_user_id, None, &[MessageId::new(1)])
        .await
        .expect("user lookup should succeed");
    assert_eq!(msgs_user.len(), 1);
    assert_eq!(msgs_user[0].text.as_deref(), Some("User Message"));

    let user_calls2 = adapter.messages_get_messages_calls.lock().unwrap().clone();
    assert_eq!(user_calls2.len(), 1);
    assert_eq!(user_calls2[0].0, channel_like_user_id);

    let channel_calls2 = adapter.channels_get_messages_calls.lock().unwrap().clone();
    assert_eq!(channel_calls2.len(), 1);

    adapter
        .get_messages(
            positive_channel_id,
            Some(PeerType::Channel),
            &[MessageId::new(1)],
        )
        .await
        .unwrap();
    assert_eq!(adapter.channels_get_messages_calls.lock().unwrap().len(), 2);

    let unknown_peer = PeerId::new(888888);
    let err = adapter
        .get_messages(unknown_peer, None, &[MessageId::new(1)])
        .await;
    assert!(err.is_err());
    match err.unwrap_err() {
        AdapterError::UnknownPeerType(p) => assert_eq!(p, unknown_peer),
        other => panic!("Expected UnknownPeerType, got {other:?}"),
    }
}

use vendetta_tg_adapter::AdapterError;

#[tokio::test]
async fn media_transfer_propagates_dc_authorizations() {
    let adapter = FakeTelegramAdapter::new();
    let location_bytes = vec![1, 2, 3, 4];
    let file_data = vec![42u8; 1024];

    adapter.add_file(location_bytes.clone(), file_data.clone());

    assert!(adapter.is_authorized().await.unwrap());
    let chunk = adapter
        .download_file_chunk(&location_bytes, 4, 0, 1024)
        .await
        .expect("download should succeed when authenticated");
    assert_eq!(chunk, file_data);

    adapter.set_authorized(false);
    assert!(!adapter.is_authorized().await.unwrap());
    adapter.inject_download_error("AUTH_KEY_UNREGISTERED".to_string());
    let err = adapter
        .download_file_chunk(&location_bytes, 4, 0, 1024)
        .await;
    assert!(err.is_err());
    match err.unwrap_err() {
        AdapterError::Invocation(e) => assert_eq!(e, "AUTH_KEY_UNREGISTERED"),
        other => panic!("Expected Auth error, got {other:?}"),
    }
}
