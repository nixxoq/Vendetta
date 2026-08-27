//! HTML export and verification CLI command functional tests.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;
use vendetta_model::{
    MessageId, MessageKey, MessageRecord, MessageState, PeerId, PeerRecord, PeerType,
};
use vendetta_storage::ArchiveDb;

fn create_fixture_db(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let db_path = temp.path().join("test_archive.db");
    let db = ArchiveDb::open(&db_path).unwrap();

    let peer = PeerRecord {
        peer_id: PeerId::new(1001),
        peer_type: PeerType::Group,
        name: Some("Rust Architecture Group".to_string()),
        username: Some("rust_arch".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let msgs: Vec<MessageRecord> = (1..=5)
        .map(|i| MessageRecord {
            key: MessageKey::new(PeerId::new(1001), MessageId::new(i)),
            date: 1700000000 + i * 60,
            sender_id: Some(PeerId::new(1001)),
            text: Some(format!("Message number {i}")),
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: None,
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: None,
            forward_json: None,
            reactions_json: None,
            views: Some(10),
            forwards_count: Some(1),
            raw_tl: None,
        })
        .collect();
    db.insert_messages_batch(&msgs).unwrap();

    db_path
}

#[test]
fn export_html_rejects_invalid_mode_or_theme() {
    let temp = tempdir().unwrap();
    let db_path = create_fixture_db(&temp);
    let out_dir = temp.path().join("out");

    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.args([
        "export-html",
        "--archive",
        db_path.to_str().unwrap(),
        "--output",
        out_dir.to_str().unwrap(),
        "--mode",
        "invalid-mode",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("invalid value 'invalid-mode'"));
}

#[test]
fn export_html_and_verify_html_roundtrip_succeeds() {
    let temp = tempdir().unwrap();
    let db_path = create_fixture_db(&temp);
    let out_dir = temp.path().join("html_export");

    let mut export_cmd = Command::cargo_bin("vendetta").unwrap();
    export_cmd
        .args([
            "export-html",
            "--archive",
            db_path.to_str().unwrap(),
            "--output",
            out_dir.to_str().unwrap(),
            "--mode",
            "telegram-like",
            "--media",
            "copy",
            "--theme",
            "system",
            "--chunk-size",
            "2",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "HTML EXPORT COMPLETED SUCCESSFULLY",
        ))
        .stdout(predicate::str::contains("Messages:          5"));

    assert!(out_dir.join("index.html").exists());
    assert!(out_dir.join("manifest.json").exists());
    assert!(out_dir.join("chats/p_1001/page_00001.html").exists());
    assert!(out_dir.join("chats/p_1001/page_00002.html").exists());
    assert!(out_dir.join("chats/p_1001/page_00003.html").exists());

    let mut verify_cmd = Command::cargo_bin("vendetta").unwrap();
    verify_cmd
        .args(["verify-html", "--html-dir", out_dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "HTML ARCHIVE INTEGRITY VERIFICATION PASSED",
        ))
        .stdout(predicate::str::contains("Errors:            0"));

    fs::remove_file(out_dir.join("chats/p_1001/page_00002.html")).unwrap();

    let mut verify_corrupt_cmd = Command::cargo_bin("vendetta").unwrap();
    verify_corrupt_cmd
        .args(["verify-html", "--html-dir", out_dir.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn export_html_replace_flag_overwrites_existing_directory() {
    let temp = tempdir().unwrap();
    let db_path = create_fixture_db(&temp);
    let out_dir = temp.path().join("replace_test_out");
    fs::create_dir_all(&out_dir).unwrap();
    fs::write(out_dir.join("existing_file.txt"), "hello").unwrap();

    let mut cmd1 = Command::cargo_bin("vendetta").unwrap();
    cmd1.args([
        "export-html",
        "--archive",
        db_path.to_str().unwrap(),
        "--output",
        out_dir.to_str().unwrap(),
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("already exists"));

    assert!(out_dir.join("existing_file.txt").exists());

    let mut cmd2 = Command::cargo_bin("vendetta").unwrap();
    cmd2.args([
        "export-html",
        "--archive",
        db_path.to_str().unwrap(),
        "--output",
        out_dir.to_str().unwrap(),
        "--replace",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "HTML EXPORT COMPLETED SUCCESSFULLY",
    ));

    assert!(out_dir.join("manifest.json").exists());
}

#[test]
fn export_html_respects_filter_flags() {
    let temp = tempdir().unwrap();
    let db_path = create_fixture_db(&temp);
    let out_dir = temp.path().join("flag_test_out");

    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.args([
        "export-html",
        "--archive",
        db_path.to_str().unwrap(),
        "--output",
        out_dir.to_str().unwrap(),
        "--include-service-messages",
        "false",
        "--include-deleted-messages",
        "false",
        "--include-edit-history",
        "false",
        "--build-search-index",
        "false",
        "--build-date-index",
        "false",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains(
        "HTML EXPORT COMPLETED SUCCESSFULLY",
    ));

    assert!(out_dir.join("manifest.json").exists());
    assert!(out_dir.join("index.html").exists());
}

#[test]
fn verify_media_progress_isolates_json_and_supports_quiet() {
    let temp = tempdir().unwrap();
    let db_path = create_fixture_db(&temp);
    let media_dir = temp.path().join("media");
    fs::create_dir_all(&media_dir).unwrap();

    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    let assert = cmd
        .args([
            "verify-media",
            "--archive",
            db_path.to_str().unwrap(),
            "--media-dir",
            media_dir.to_str().unwrap(),
            "--json",
        ])
        .assert()
        .success();

    let stdout_str = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout_str).expect("stdout must be valid JSON");
    assert_eq!(parsed["command"], "verify-media");
    assert_eq!(parsed["status"], "passed");

    let mut cmd_quiet = Command::cargo_bin("vendetta").unwrap();
    cmd_quiet
        .args([
            "--quiet",
            "verify-media",
            "--archive",
            db_path.to_str().unwrap(),
            "--media-dir",
            media_dir.to_str().unwrap(),
            "--json",
        ])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
}
