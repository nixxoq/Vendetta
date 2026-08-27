use assert_cmd::Command;
use std::fs;
use tempfile::tempdir;
use vendetta_model::{
    MessageId, MessageKey, MessageRecord, MessageState, PeerId, PeerRecord, PeerType,
};
use vendetta_storage::ArchiveDb;

#[test]
fn export_and_verify_handle_unicode_spaces_and_parentheses_in_paths() {
    let tmp = tempdir().unwrap();
    let special_dir = tmp.path().join("Телеграм Архив (2026) 🚀");
    fs::create_dir_all(&special_dir).unwrap();

    let db_path = special_dir.join("база данных [test].db");
    let html_out = special_dir.join("экспорт html (offline)");

    let db = ArchiveDb::open(&db_path).unwrap();
    let peer = PeerRecord {
        peer_id: PeerId::new(1001),
        peer_type: PeerType::User,
        name: Some("Тестовый Пользователь".to_string()),
        username: Some("test_user".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let msg = MessageRecord {
        key: MessageKey::new(peer.peer_id, MessageId::new(1)),
        date: 1700000000,
        sender_id: Some(peer.peer_id),
        text: Some("Привет из Юникода!".to_string()),
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

    let mut export_cmd = Command::cargo_bin("vendetta").unwrap();
    export_cmd
        .args([
            "export-html",
            "--archive",
            db_path.to_str().unwrap(),
            "--output",
            html_out.to_str().unwrap(),
            "--json",
        ])
        .assert()
        .success();

    assert!(html_out.join("index.html").exists());

    let mut vhtml_cmd = Command::cargo_bin("vendetta").unwrap();
    vhtml_cmd
        .args(["verify-html", "--html-dir", html_out.to_str().unwrap()])
        .assert()
        .success();

    let mut varchive_cmd = Command::cargo_bin("vendetta").unwrap();
    varchive_cmd
        .args([
            "verify-archive",
            "--archive",
            db_path.to_str().unwrap(),
            "--html",
            html_out.to_str().unwrap(),
            "--json",
        ])
        .assert()
        .success();
}
