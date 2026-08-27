use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6},
    sync::Arc,
};
use tempfile::tempdir;

use grammers_session::{
    Session,
    types::{ChannelKind, DcOption, PeerAuth, PeerId as GrammersPeerId, PeerInfo},
};
use grammers_tl_types as tl;
use vendetta_model::{PeerId, PeerRecord, PeerType};
use vendetta_tg_adapter::{
    AdapterError, FakeTelegramAdapter, FileSession, GrammersTelegramAdapter, TelegramAdapter,
    normalize::{normalize_raw_chat, normalize_raw_user},
};

#[tokio::test]
async fn file_session_reloads_and_persists_atomically() {
    let dir = tempdir().expect("tempdir failed");
    let session_path = dir.path().join("session.json");

    let session = FileSession::open(&session_path).expect("open session failed");

    session.set_home_dc_id(4).await.expect("set home dc failed");

    let mut auth_key = [0u8; 256];
    auth_key[10] = 0x42;
    let custom_dc = DcOption {
        id: 4,
        ipv4: SocketAddrV4::new(Ipv4Addr::new(149, 154, 167, 91), 443),
        ipv6: SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 443, 0, 0),
        auth_key: Some(auth_key),
    };
    session
        .set_dc_option(&custom_dc)
        .await
        .expect("set dc option failed");

    let user_info = PeerInfo::User {
        id: 555666,
        auth: Some(PeerAuth::from_hash(11223344)),
        bot: Some(false),
        is_self: Some(true),
    };
    session
        .cache_peer(&user_info)
        .await
        .expect("cache peer failed");

    session.save().expect("save session failed");

    drop(session);

    let reloaded = FileSession::open(&session_path).expect("reopen failed");
    let state = reloaded
        .get_session_state()
        .expect("get session state failed");

    assert_eq!(state.home_dc, 4);
    assert!(
        state
            .dc_options
            .iter()
            .any(|dc| dc.id == 4 && dc.auth_key == Some(auth_key))
    );
    assert!(
        state
            .peer_infos
            .iter()
            .any(|p| matches!(p, PeerInfo::User { id: 555666, .. }))
    );

    let peer_ref = reloaded
        .peer_ref(GrammersPeerId::user_unchecked(555666))
        .await
        .expect("peer_ref failed")
        .expect("peer_ref missing");

    assert_eq!(peer_ref.auth.hash(), 11223344);
}

#[test]
fn peer_normalization_covers_all_telegram_types() {
    let raw_user = tl::enums::User::User(tl::types::User {
        is_self: false,
        contact: true,
        mutual_contact: false,
        deleted: false,
        bot: false,
        bot_chat_history: false,
        bot_nochats: false,
        verified: true,
        restricted: false,
        min: false,
        bot_inline_geo: false,
        support: false,
        scam: false,
        apply_min_photo: false,
        fake: false,
        bot_attach_menu: false,
        premium: true,
        attach_menu_enabled: false,
        bot_can_edit: false,
        close_friend: false,
        stories_hidden: false,
        stories_unavailable: false,
        contact_require_premium: false,
        bot_business: false,
        bot_has_main_app: false,
        bot_forum_view: false,
        bot_forum_can_manage_topics: false,
        bot_can_manage_bots: false,
        bot_guestchat: false,
        bot_guard: false,
        id: 123456789,
        access_hash: Some(987654321),
        first_name: Some("Alice".to_string()),
        last_name: Some("Smith".to_string()),
        username: Some("alice_smith".to_string()),
        phone: Some("+1234567890".to_string()),
        photo: None,
        status: None,
        bot_info_version: None,
        restriction_reason: None,
        bot_inline_placeholder: None,
        lang_code: None,
        emoji_status: None,
        usernames: None,
        stories_max_id: None,
        color: None,
        profile_color: None,
        bot_active_users: None,
        bot_verification_icon: None,
        send_paid_messages_stars: None,
        linked_community_id: None,
    });

    let norm_user = normalize_raw_user(&raw_user).expect("user norm failed");
    assert_eq!(norm_user.peer_id, PeerId::new(123456789));
    assert_eq!(norm_user.peer_type, PeerType::User);
    assert_eq!(norm_user.name.as_deref(), Some("Alice Smith"));
    assert_eq!(norm_user.username.as_deref(), Some("alice_smith"));
    assert_eq!(norm_user.phone.as_deref(), Some("+1234567890"));
    assert!(norm_user.raw_tl.is_some());

    let raw_empty_user = tl::enums::User::Empty(tl::types::UserEmpty { id: 99999 });
    let norm_deleted = normalize_raw_user(&raw_empty_user).expect("deleted norm failed");
    assert_eq!(norm_deleted.peer_id, PeerId::new(99999));
    assert_eq!(norm_deleted.peer_type, PeerType::User);
    assert_eq!(norm_deleted.name.as_deref(), Some("[Deleted Account]"));

    let raw_chat = tl::enums::Chat::Chat(tl::types::Chat {
        creator: false,
        left: false,
        deactivated: false,
        call_active: false,
        call_not_empty: false,
        noforwards: false,
        id: 456789,
        title: "Rust Enthusiasts".to_string(),
        photo: tl::enums::ChatPhoto::Empty,
        participants_count: 42,
        date: 1600000000,
        version: 1,
        migrated_to: None,
        admin_rights: None,
        default_banned_rights: None,
    });

    let norm_chat = normalize_raw_chat(&raw_chat).expect("chat norm failed");
    assert_eq!(norm_chat.peer_id, PeerId::new(-456789));
    assert_eq!(norm_chat.peer_type, PeerType::Group);
    assert_eq!(norm_chat.name.as_deref(), Some("Rust Enthusiasts"));

    let raw_channel = tl::enums::Chat::Channel(tl::types::Channel {
        creator: false,
        left: false,
        broadcast: true,
        verified: false,
        megagroup: false,
        restricted: false,
        signatures: false,
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
        id: 11223344,
        access_hash: Some(99887766),
        title: "Rust Announcements".to_string(),
        username: Some("rust_announcements".to_string()),
        photo: tl::enums::ChatPhoto::Empty,
        date: 1600000000,
        restriction_reason: None,
        admin_rights: None,
        banned_rights: None,
        default_banned_rights: None,
        participants_count: Some(50000),
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
    });

    let norm_channel = normalize_raw_chat(&raw_channel).expect("channel norm failed");
    assert_eq!(norm_channel.peer_type, PeerType::Channel);
    assert_eq!(norm_channel.name.as_deref(), Some("Rust Announcements"));
    assert_eq!(norm_channel.username.as_deref(), Some("rust_announcements"));

    let raw_forbidden = tl::enums::Chat::ChannelForbidden(tl::types::ChannelForbidden {
        broadcast: true,
        megagroup: false,
        monoforum: false,
        id: 888999,
        access_hash: 554433,
        title: "Secret Channel".to_string(),
        until_date: None,
    });

    let norm_forbidden = normalize_raw_chat(&raw_forbidden).expect("forbidden norm failed");
    assert_eq!(norm_forbidden.peer_type, PeerType::Channel);
    assert_eq!(
        norm_forbidden.name.as_deref(),
        Some("[Inaccessible] Secret Channel")
    );
}

#[tokio::test]
async fn dialog_retrieval_deduplicates_across_folders() {
    let adapter = FakeTelegramAdapter::new();

    let peer1 = PeerRecord {
        peer_id: PeerId::new(1001),
        peer_type: PeerType::User,
        name: Some("Bob".to_string()),
        username: Some("bob".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    let peer2 = PeerRecord {
        peer_id: PeerId::new(-1002),
        peer_type: PeerType::Group,
        name: Some("Dev Team".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };

    adapter.add_raw_peer(peer1.clone());
    adapter.add_raw_peer(peer2.clone());
    adapter.add_raw_peer(peer1.clone());

    let dialogs = adapter.get_dialogs().await.expect("get_dialogs failed");
    assert_eq!(dialogs.len(), 2);
    assert_eq!(dialogs[0].peer_id, PeerId::new(1001));
    assert_eq!(dialogs[1].peer_id, PeerId::new(-1002));
}

#[tokio::test]
async fn adapter_propagates_cancellation_and_errors() {
    let adapter = FakeTelegramAdapter::new();

    adapter.inject_error("FLOOD_WAIT_42");
    let res = adapter.get_dialogs().await;
    match res {
        Err(AdapterError::FloodWait { seconds }) => assert_eq!(seconds, 42),
        other => panic!("Expected FloodWait error, got {other:?}"),
    }

    adapter.inject_error("UNAUTHORIZED");
    let res = adapter.get_dialogs().await;
    match res {
        Err(AdapterError::NotAuthenticated) => (),
        other => panic!("Expected NotAuthenticated, got {other:?}"),
    }

    adapter.clear_error();
    assert!(adapter.get_dialogs().await.is_ok());
}

#[tokio::test]
async fn peer_auth_rejects_zero_access_hash_fallbacks() {
    let dir = tempdir().expect("tempdir failed");
    let session_path = dir.path().join("session.json");
    let session = Arc::new(FileSession::open(&session_path).expect("open session failed"));

    session
        .cache_peer(&PeerInfo::User {
            id: 111222,
            auth: Some(PeerAuth::from_hash(998877)),
            bot: Some(false),
            is_self: Some(false),
        })
        .await
        .unwrap();

    session
        .cache_peer(&PeerInfo::Channel {
            id: 333444,
            auth: Some(PeerAuth::from_hash(556677)),
            kind: Some(ChannelKind::Megagroup),
        })
        .await
        .unwrap();

    let basic_chat_id = PeerId::new(-500);
    let raw_basic_id = basic_chat_id.raw();
    assert!(raw_basic_id < 0 && raw_basic_id > -1_000_000_000_000);

    let user_ref = session
        .peer_ref(GrammersPeerId::user_unchecked(111222))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user_ref.auth.hash(), 998877);

    let uncached_user = session
        .peer_ref(GrammersPeerId::user_unchecked(99999999))
        .await
        .unwrap();
    assert!(uncached_user.is_none());

    let channel_ref = session
        .peer_ref(GrammersPeerId::channel_unchecked(333444))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(channel_ref.auth.hash(), 556677);

    let uncached_channel = session
        .peer_ref(GrammersPeerId::channel_unchecked(88888888))
        .await
        .unwrap();
    assert!(uncached_channel.is_none());
}

#[tokio::test]
async fn peer_type_resolution_is_strict_without_numeric_heuristics() {
    let dir = tempdir().expect("tempdir failed");
    let session_path = dir.path().join("session.json");
    let session = Arc::new(FileSession::open(&session_path).expect("open session failed"));

    session
        .cache_peer(&PeerInfo::Channel {
            id: 777888,
            auth: Some(PeerAuth::from_hash(112233)),
            kind: Some(ChannelKind::Megagroup),
        })
        .await
        .unwrap();

    session
        .cache_peer(&PeerInfo::User {
            id: 1_000_000_555_666,
            auth: Some(PeerAuth::from_hash(445566)),
            bot: Some(false),
            is_self: Some(false),
        })
        .await
        .unwrap();

    let adapter = GrammersTelegramAdapter::new_with_session(Arc::clone(&session));

    let positive_channel_peer = PeerId::new(777888);
    let channel_looking_user_peer = PeerId::new(-1_000_000_555_666);
    let uncached_peer = PeerId::new(999999);

    let res = adapter.resolve_input_peer(uncached_peer).await;
    assert!(res.is_err());
    match res.unwrap_err() {
        AdapterError::UnknownPeerType(p) => assert_eq!(p, uncached_peer),
        other => panic!("Expected UnknownPeerType, got {other:?}"),
    }

    let res_ch = adapter.resolve_input_channel(uncached_peer).await;
    assert!(res_ch.is_err());
    match res_ch.unwrap_err() {
        AdapterError::UnknownPeerType(p) => assert_eq!(p, uncached_peer),
        other => panic!("Expected UnknownPeerType, got {other:?}"),
    }

    adapter.register_peer_type(positive_channel_peer, PeerType::Channel);
    let input_peer = adapter
        .resolve_input_peer(positive_channel_peer)
        .await
        .expect("positive channel resolve_input_peer should succeed");
    assert!(matches!(input_peer, tl::enums::InputPeer::Channel(_)));

    let input_channel = adapter
        .resolve_input_channel(positive_channel_peer)
        .await
        .expect("positive channel resolve_input_channel should succeed");
    assert!(matches!(input_channel, tl::enums::InputChannel::Channel(_)));

    adapter.register_peer_type(channel_looking_user_peer, PeerType::User);
    let input_user = adapter
        .resolve_input_peer(channel_looking_user_peer)
        .await
        .expect("channel-looking user resolve_input_peer should succeed");
    assert!(matches!(input_user, tl::enums::InputPeer::User(_)));

    let inv = adapter
        .resolve_input_channel(channel_looking_user_peer)
        .await;
    assert!(inv.is_err());
    match inv.unwrap_err() {
        AdapterError::InvalidPeerType { peer_id, .. } => {
            assert_eq!(peer_id, channel_looking_user_peer)
        }
        other => panic!("Expected InvalidPeerType, got {other:?}"),
    }

    assert_eq!(
        GrammersTelegramAdapter::decode_user_id(PeerId::new(12345)),
        12345
    );
    assert_eq!(
        GrammersTelegramAdapter::decode_user_id(PeerId::new(-12345)),
        12345
    );
    assert_eq!(
        GrammersTelegramAdapter::decode_group_id(PeerId::new(-500)),
        500
    );
    assert_eq!(
        GrammersTelegramAdapter::decode_group_id(PeerId::new(500)),
        500
    );
    assert_eq!(
        GrammersTelegramAdapter::decode_channel_id(PeerId::new(-1_000_000_777_888)),
        777888
    );
    assert_eq!(
        GrammersTelegramAdapter::decode_channel_id(PeerId::new(777888)),
        777888
    );
}

#[tokio::test]
async fn megagroup_peer_resolution_uses_input_channel() {
    let dir = tempdir().expect("tempdir failed");
    let session_path = dir.path().join("session.json");
    let session = Arc::new(FileSession::open(&session_path).expect("open session failed"));

    let bare_channel_id = 3563998964_i64;
    let access_hash = 9876543210123_i64;
    session
        .cache_peer(&PeerInfo::Channel {
            id: bare_channel_id,
            auth: Some(PeerAuth::from_hash(access_hash)),
            kind: Some(ChannelKind::Megagroup),
        })
        .await
        .unwrap();

    let adapter = GrammersTelegramAdapter::new_with_session(Arc::clone(&session));
    let megagroup_peer_id = PeerId::new(-1_000_000_000_000 - bare_channel_id);
    assert_eq!(megagroup_peer_id.raw(), -1003563998964);

    adapter.register_peer_type(megagroup_peer_id, PeerType::Group);

    let input_peer = adapter
        .resolve_input_peer(megagroup_peer_id)
        .await
        .expect("megagroup resolve_input_peer should succeed");

    match input_peer {
        tl::enums::InputPeer::Channel(ch) => {
            assert_eq!(ch.channel_id, bare_channel_id);
            assert_eq!(ch.access_hash, access_hash);
        }
        other => panic!("Expected InputPeer::Channel for megagroup/supergroup, got {other:?}"),
    }

    let input_channel = adapter
        .resolve_input_channel(megagroup_peer_id)
        .await
        .expect("megagroup resolve_input_channel should succeed");

    match input_channel {
        tl::enums::InputChannel::Channel(ch) => {
            assert_eq!(ch.channel_id, bare_channel_id);
            assert_eq!(ch.access_hash, access_hash);
        }
        other => panic!("Expected InputChannel::Channel for megagroup/supergroup, got {other:?}"),
    }

    let basic_group_peer_id = PeerId::new(-54321);
    adapter.register_peer_type(basic_group_peer_id, PeerType::Group);

    let basic_input_peer = adapter
        .resolve_input_peer(basic_group_peer_id)
        .await
        .expect("basic group resolve_input_peer should succeed");

    match basic_input_peer {
        tl::enums::InputPeer::Chat(chat) => {
            assert_eq!(chat.chat_id, 54321);
        }
        other => panic!("Expected InputPeer::Chat for basic chat, got {other:?}"),
    }
}
