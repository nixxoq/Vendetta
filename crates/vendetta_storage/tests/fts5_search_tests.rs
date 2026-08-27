use vendetta_model::{
    MessageId, MessageKey, MessageRecord, MessageState, PeerId, PeerRecord, PeerType,
};
use vendetta_storage::{ArchiveDb, FtsSearchParams};

fn make_msg(peer: i64, id: i64, date: i64, text: &str, state: MessageState) -> MessageRecord {
    MessageRecord {
        key: MessageKey::new(PeerId::new(peer), MessageId::new(id)),
        date,
        sender_id: Some(PeerId::new(peer)),
        text: Some(text.to_string()),
        entities_json: None,
        edit_date: None,
        state,
        reply_to_msg_id: None,
        reply_to_top_id: None,
        reply_to_peer_id: None,
        grouped_id: None,
        forward_json: None,
        reactions_json: None,
        views: None,
        forwards_count: None,
        raw_tl: None,
    }
}

#[test]
fn fts5_trigger_lifecycle_indexes_active_and_removes_deleted() {
    let db = ArchiveDb::open_in_memory().unwrap();

    let peer = PeerRecord {
        peer_id: PeerId::new(1001),
        peer_type: PeerType::User,
        name: Some("Test User".to_string()),
        username: Some("testuser".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let msg1 = make_msg(
        1001,
        1,
        1700000010,
        "Hello world! This is a secret message.",
        MessageState::Active,
    );
    db.insert_messages_batch(&[msg1]).unwrap();

    let res = db
        .search_fts(&FtsSearchParams {
            query: "secret".to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].key.message_id, MessageId::new(1));
    assert!(res[0].snippet.as_ref().unwrap().contains("<b>secret</b>"));

    let msg2 = make_msg(
        1001,
        2,
        1700000020,
        "This deleted confidential text should never match",
        MessageState::Deleted,
    );
    db.insert_messages_batch(&[msg2]).unwrap();

    let res = db
        .search_fts(&FtsSearchParams {
            query: "confidential".to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res.len(), 0);

    let msg3 = make_msg(
        1001,
        3,
        1700000030,
        "Inaccessible restricted text",
        MessageState::Inaccessible,
    );
    let msg4 = make_msg(
        1001,
        4,
        1700000040,
        "Empty placeholder text",
        MessageState::Empty,
    );
    db.insert_messages_batch(&[msg3, msg4]).unwrap();

    let res_inacc = db
        .search_fts(&FtsSearchParams {
            query: "restricted".to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res_inacc.len(), 0);

    let res_empty = db
        .search_fts(&FtsSearchParams {
            query: "placeholder".to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res_empty.len(), 0);

    let mut msg1_edited = make_msg(
        1001,
        1,
        1700000010,
        "Hello world! This is an updated message.",
        MessageState::Edited,
    );
    msg1_edited.edit_date = Some(1700000050);
    db.insert_messages_batch(&[msg1_edited]).unwrap();

    let res_old = db
        .search_fts(&FtsSearchParams {
            query: "secret".to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res_old.len(), 0);

    let res_new = db
        .search_fts(&FtsSearchParams {
            query: "updated".to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res_new.len(), 1);
    assert_eq!(res_new[0].key.message_id, MessageId::new(1));

    let msg1_deleted = make_msg(
        1001,
        1,
        1700000010,
        "Hello world! This is an updated message.",
        MessageState::Deleted,
    );
    db.insert_messages_batch(&[msg1_deleted]).unwrap();

    let res_deleted = db
        .search_fts(&FtsSearchParams {
            query: "updated".to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res_deleted.len(), 0);

    let msg1_restored = make_msg(
        1001,
        1,
        1700000010,
        "Hello world! This is a restored message.",
        MessageState::Active,
    );
    db.insert_messages_batch(&[msg1_restored]).unwrap();

    let res_restored = db
        .search_fts(&FtsSearchParams {
            query: "restored".to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res_restored.len(), 1);
    assert_eq!(res_restored[0].key.message_id, MessageId::new(1));
}

#[test]
fn fts5_search_handles_multilingual_cyrillic_and_filters() {
    let db = ArchiveDb::open_in_memory().unwrap();

    let peer1 = PeerRecord {
        peer_id: PeerId::new(100),
        peer_type: PeerType::User,
        name: Some("User 1".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    let peer2 = PeerRecord {
        peer_id: PeerId::new(200),
        peer_type: PeerType::Channel,
        name: Some("Channel 2".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer1).unwrap();
    db.upsert_peer(&peer2).unwrap();

    let msgs = vec![
        make_msg(
            100,
            1,
            1700000100,
            "Привіт, це тестове повідомлення українською мовою.",
            MessageState::Active,
        ),
        make_msg(
            100,
            2,
            1700000200,
            "Привет, это тестовое сообщение на русском языке.",
            MessageState::Active,
        ),
        make_msg(
            200,
            1,
            1700000300,
            "Channel broadcast with важное повідомлення and critical announcements.",
            MessageState::Active,
        ),
        make_msg(
            200,
            2,
            1700000400,
            "German and French text: Übergröße und naïve façade.",
            MessageState::Active,
        ),
    ];
    db.insert_messages_batch(&msgs).unwrap();

    let res_ua = db
        .search_fts(&FtsSearchParams {
            query: "повідомлення".to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res_ua.len(), 2);

    let res_ru = db
        .search_fts(&FtsSearchParams {
            query: "сообщение".to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res_ru.len(), 1);
    assert_eq!(
        res_ru[0].key,
        MessageKey::new(PeerId::new(100), MessageId::new(2))
    );

    let res_acc = db
        .search_fts(&FtsSearchParams {
            query: "facade".to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res_acc.len(), 1);
    assert_eq!(
        res_acc[0].key,
        MessageKey::new(PeerId::new(200), MessageId::new(2))
    );

    let res_peer = db
        .search_fts(&FtsSearchParams {
            query: "повідомлення".to_string(),
            peer_id: Some(PeerId::new(200)),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res_peer.len(), 1);
    assert_eq!(
        res_peer[0].key,
        MessageKey::new(PeerId::new(200), MessageId::new(1))
    );

    let res_date = db
        .search_fts(&FtsSearchParams {
            query: "повідомлення".to_string(),
            min_date: Some(1700000250),
            max_date: Some(1700000350),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(res_date.len(), 1);
    assert_eq!(
        res_date[0].key,
        MessageKey::new(PeerId::new(200), MessageId::new(1))
    );
}
