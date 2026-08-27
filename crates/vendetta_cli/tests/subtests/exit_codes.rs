//! Exit code

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;
use vendetta_model::{
    MediaDownloadStatus, MediaKind, MediaRecord, MediaVerificationStatus, MessageId, MessageKey,
    MessageRecord, MessageState, PeerId, PeerRecord, PeerType,
};
use vendetta_storage::ArchiveDb;

#[test]
fn auth_status_returns_warning_or_json_unauthorized() {
    let tmp = tempdir().unwrap();
    let session_path = tmp.path().join("empty_session.json");

    // Human mode without session -> Exit code 1 (EXIT_WARNING)
    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.args([
        "auth",
        "status",
        "--session",
        session_path.to_str().unwrap(),
    ])
    .assert()
    .code(1);

    // JSON mode without session -> Exit code 0 with status: "unauthorized"
    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.args([
        "auth",
        "status",
        "--session",
        session_path.to_str().unwrap(),
        "--json",
    ])
    .assert()
    .code(0)
    .stdout(predicate::str::contains("\"status\": \"unauthorized\""));
}

#[test]
fn invalid_cli_arguments_returns_error_code() {
    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.args(["--non-existent-flag"]).assert().code(2);
}

#[test]
fn missing_archive_database_returns_fatal_code() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("non_existent_archive.db");

    // verify-archive on missing database -> Exit code 3 (EXIT_FATAL)
    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.args(["verify-archive", "--archive", db_path.to_str().unwrap()])
        .assert()
        .code(3);
}

#[test]
fn verify_html_on_missing_dir_returns_fatal_code() {
    let tmp = tempdir().unwrap();
    let html_path = tmp.path().join("non_existent_html_dir");

    // verify-html on missing directory -> Exit code 3 (EXIT_FATAL)
    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.args(["verify-html", "--html-dir", html_path.to_str().unwrap()])
        .assert()
        .code(3);
}

#[test]
fn verify_media_on_missing_file_returns_error_code() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("archive.db");
    let media_dir = tmp.path().join("media");
    std::fs::create_dir_all(&media_dir).unwrap();

    let db = ArchiveDb::open(&db_path).unwrap();
    let media_rec = MediaRecord {
        media_id: "media_1".to_string(),
        kind: MediaKind::Photo,
        mime_type: Some("image/jpeg".to_string()),
        size_bytes: Some(1024),
        file_name: Some("test.jpg".to_string()),
        size_type: None,
        width: Some(100),
        height: Some(100),
        dc_id: 2,
        source_location_tl: Some(vec![]),
        file_reference: None,
        local_rel_path: Some("photos/100_1.jpg".to_string()),
        sha256: Some(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
        ),
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
        created_at: 1000,
        updated_at: 1000,
    };
    db.insert_or_update_media(&media_rec).unwrap();

    // verify-media when file is missing from disk -> Exit code 2 (EXIT_ERROR)
    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.args([
        "verify-media",
        "--archive",
        db_path.to_str().unwrap(),
        "--media-dir",
        media_dir.to_str().unwrap(),
    ])
    .assert()
    .code(2);
}

#[test]
fn verify_archive_distinguishes_warning_from_strict_error() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("archive.db");

    let db = ArchiveDb::open(&db_path).unwrap();
    let peer = PeerRecord {
        peer_id: PeerId::new(100),
        peer_type: PeerType::User,
        name: Some("Test User".to_string()),
        username: None,
        phone: None,
        raw_tl: None,
        updated_at: 1000,
    };
    db.upsert_peer(&peer).unwrap();

    // Insert message replying to non-existent message ID -> causes REPLY_TARGET_MISSING (warning)
    let msg = MessageRecord {
        key: MessageKey::new(PeerId::new(100), MessageId::new(2)),
        date: 1700000000,
        sender_id: Some(PeerId::new(100)),
        text: Some("Reply msg".to_string()),
        entities_json: None,
        edit_date: None,
        state: MessageState::Active,
        reply_to_msg_id: Some(MessageId::new(999)),
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

    // Non-strict verify-archive with warning -> Exit code 1 (EXIT_WARNING)
    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.args([
        "verify-archive",
        "--archive",
        db_path.to_str().unwrap(),
        "--replies",
    ])
    .assert()
    .code(1);

    // Strict verify-archive with warning -> Exit code 2 (EXIT_ERROR)
    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.args([
        "verify-archive",
        "--archive",
        db_path.to_str().unwrap(),
        "--replies",
        "--strict",
    ])
    .assert()
    .code(2);
}

#[test]
fn sync_explicit_peers_override_peer_type_filter() {
    if !cfg!(feature = "test-mock") {
        eprintln!("SKIPPED: run with --features test-mock to execute mock sync test");
        return;
    }

    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("archive.db");
    let session_path = tmp.path().join("session.json");
    std::fs::write(&session_path, b"{}").unwrap();

    // Mock has user=1001, group=2001, channel=3001.
    // If we specify --peers 2001, 3001 with --peer-type user, --peers must override and sync 2001 and 3001!
    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    let assert = cmd
        .env("VENDETTA_OFFLINE_MOCK", "1")
        .args([
            "sync",
            "--archive",
            db_path.to_str().unwrap(),
            "--session",
            session_path.to_str().unwrap(),
            "--peers",
            "2001,3001",
            "--peer-type",
            "user", // Filter says user, but explicit --peers must override
            "--limit",
            "1",
            "--json",
        ])
        .assert()
        .success();

    let out_str = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let val: serde_json::Value = serde_json::from_str(&out_str).unwrap();
    assert_eq!(val["schema_version"], 1);
    assert_eq!(val["status"], "completed");

    let db = ArchiveDb::open(&db_path).unwrap();
    assert!(
        db.get_message(MessageKey::new(PeerId::new(2001), MessageId::new(1)))
            .unwrap()
            .is_some()
    );
    assert!(
        db.get_message(MessageKey::new(PeerId::new(3001), MessageId::new(1)))
            .unwrap()
            .is_some()
    );
    assert!(
        db.get_message(MessageKey::new(PeerId::new(1001), MessageId::new(1)))
            .unwrap()
            .is_none()
    );
}
