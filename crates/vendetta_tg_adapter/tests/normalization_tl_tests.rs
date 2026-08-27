use grammers_tl_types::{self as tl, Serializable};
use vendetta_model::{MessageId, MessageKey, MessageState, PeerId};
use vendetta_tg_adapter::normalize_message;

#[test]
fn real_tl_normal_message_normalizes_cleanly() {
    let raw_msg = tl::types::Message {
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
        id: 42,
        from_id: Some(tl::enums::Peer::User(tl::types::PeerUser {
            user_id: 1234567,
        })),
        from_boosts_applied: None,
        from_rank: None,
        peer_id: tl::enums::Peer::Channel(tl::types::PeerChannel {
            channel_id: 9876543,
        }),
        saved_peer_id: None,
        fwd_from: None,
        via_bot_id: None,
        via_business_bot_id: None,
        guestchat_via_from: None,
        reply_to: Some(tl::enums::MessageReplyHeader::Header(
            tl::types::MessageReplyHeader {
                reply_to_scheduled: false,
                forum_topic: true,
                quote: false,
                reply_to_msg_id: Some(10),
                reply_to_peer_id: Some(tl::enums::Peer::Channel(tl::types::PeerChannel {
                    channel_id: 9876543,
                })),
                reply_from: None,
                reply_media: None,
                reply_to_top_id: Some(1),
                quote_text: None,
                quote_entities: None,
                quote_offset: None,
                todo_item_id: None,
                poll_option: None,
                reply_to_ephemeral: false,
            },
        )),
        date: 1700001000,
        message: "Hello world with bold and link".to_string(),
        media: None,
        reply_markup: None,
        entities: Some(vec![
            tl::enums::MessageEntity::Bold(tl::types::MessageEntityBold {
                offset: 17,
                length: 4,
            }),
            tl::enums::MessageEntity::TextUrl(tl::types::MessageEntityTextUrl {
                offset: 26,
                length: 4,
                url: "https://telegram.org".to_string(),
            }),
        ]),
        views: Some(1250),
        forwards: Some(8),
        replies: None,
        edit_date: Some(1700002000),
        post_author: Some("Editor".to_string()),
        grouped_id: Some(55555),
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
    };

    let tl_enum = tl::enums::Message::Message(raw_msg);
    let expected_raw_bytes = tl_enum.to_bytes();

    let record = normalize_message(&tl_enum, None);

    assert_eq!(
        record.key,
        MessageKey::new(PeerId::new(-1000000000000 - 9876543), MessageId::new(42))
    );
    assert_eq!(record.sender_id, Some(PeerId::new(1234567)));
    assert_eq!(record.date, 1700001000);
    assert_eq!(record.edit_date, Some(1700002000));
    assert_eq!(record.state, MessageState::Edited);
    assert_eq!(
        record.text.as_deref(),
        Some("Hello world with bold and link")
    );
    assert_eq!(record.reply_to_msg_id, Some(MessageId::new(10)));
    assert_eq!(record.reply_to_top_id, Some(MessageId::new(1)));
    assert_eq!(
        record.reply_to_peer_id,
        Some(PeerId::new(-1000000000000 - 9876543))
    );
    assert_eq!(record.grouped_id, Some(55555));
    assert_eq!(record.views, Some(1250));
    assert_eq!(record.forwards_count, Some(8));
    assert_eq!(
        record.raw_tl.as_deref(),
        Some(expected_raw_bytes.as_slice())
    );

    let entities_json = record.entities_json.expect("missing entities_json");
    assert!(entities_json.contains("Bold"));
    assert!(entities_json.contains("https://telegram.org"));
}

#[test]
fn real_tl_service_messages_normalize_cleanly() {
    let base_service = |action: tl::enums::MessageAction, id: i32| tl::types::MessageService {
        out: false,
        mentioned: false,
        media_unread: false,
        silent: false,
        post: false,
        legacy: false,
        reactions_are_possible: false,
        id,
        from_id: Some(tl::enums::Peer::User(tl::types::PeerUser { user_id: 111 })),
        peer_id: tl::enums::Peer::Chat(tl::types::PeerChat { chat_id: 222 }),
        saved_peer_id: None,
        reply_to: Some(tl::enums::MessageReplyHeader::Header(
            tl::types::MessageReplyHeader {
                reply_to_scheduled: false,
                forum_topic: false,
                quote: false,
                reply_to_msg_id: Some(100),
                reply_to_peer_id: None,
                reply_from: None,
                reply_media: None,
                reply_to_top_id: None,
                quote_text: None,
                quote_entities: None,
                quote_offset: None,
                todo_item_id: None,
                poll_option: None,
                reply_to_ephemeral: false,
            },
        )),
        date: 1700003000,
        action,
        reactions: None,
        ttl_period: None,
    };

    let pin_tl = tl::enums::Message::Service(base_service(tl::enums::MessageAction::PinMessage, 1));
    let pin_rec = normalize_message(&pin_tl, None);
    assert_eq!(pin_rec.text.as_deref(), Some("Pinned a message"));
    assert_eq!(pin_rec.reply_to_msg_id, Some(MessageId::new(100)));
    assert_eq!(pin_rec.key.peer_id, PeerId::new(-222));
    assert_eq!(
        pin_rec.raw_tl.as_deref(),
        Some(pin_tl.to_bytes().as_slice())
    );

    let title_tl = tl::enums::Message::Service(base_service(
        tl::enums::MessageAction::ChatEditTitle(tl::types::MessageActionChatEditTitle {
            title: "Core Architecture Team".to_string(),
        }),
        2,
    ));
    let title_rec = normalize_message(&title_tl, None);
    assert_eq!(
        title_rec.text.as_deref(),
        Some("Changed group title to: \"Core Architecture Team\"")
    );

    let add_user_tl = tl::enums::Message::Service(base_service(
        tl::enums::MessageAction::ChatAddUser(tl::types::MessageActionChatAddUser {
            users: vec![101, 102, 103],
        }),
        3,
    ));
    let add_user_rec = normalize_message(&add_user_tl, None);
    assert_eq!(add_user_rec.text.as_deref(), Some("Added 3 user(s)"));

    let del_user_tl = tl::enums::Message::Service(base_service(
        tl::enums::MessageAction::ChatDeleteUser(tl::types::MessageActionChatDeleteUser {
            user_id: 999,
        }),
        4,
    ));
    let del_user_rec = normalize_message(&del_user_tl, None);
    assert_eq!(del_user_rec.text.as_deref(), Some("Removed user 999"));

    let chan_create_tl = tl::enums::Message::Service(base_service(
        tl::enums::MessageAction::ChannelCreate(tl::types::MessageActionChannelCreate {
            title: "Public Releases".to_string(),
        }),
        5,
    ));
    let chan_create_rec = normalize_message(&chan_create_tl, None);
    assert_eq!(
        chan_create_rec.text.as_deref(),
        Some("Created channel: \"Public Releases\"")
    );

    let migrate_tl = tl::enums::Message::Service(base_service(
        tl::enums::MessageAction::ChatMigrateTo(tl::types::MessageActionChatMigrateTo {
            channel_id: 123456789,
        }),
        6,
    ));
    let migrate_rec = normalize_message(&migrate_tl, None);
    assert_eq!(
        migrate_rec.text.as_deref(),
        Some("Group upgraded to supergroup 123456789")
    );
}

#[test]
fn real_tl_empty_message_normalizes_to_empty_state() {
    let empty_tl = tl::enums::Message::Empty(tl::types::MessageEmpty {
        id: 777,
        peer_id: Some(tl::enums::Peer::User(tl::types::PeerUser {
            user_id: 123456,
        })),
    });
    let rec = normalize_message(&empty_tl, None);

    assert_eq!(
        rec.key,
        MessageKey::new(PeerId::new(123456), MessageId::new(777))
    );
    assert_eq!(rec.state, MessageState::Empty);
    assert_eq!(rec.text.as_deref(), Some("[Empty / Unavailable Message]"));
    assert_eq!(rec.raw_tl.as_deref(), Some(empty_tl.to_bytes().as_slice()));
}
