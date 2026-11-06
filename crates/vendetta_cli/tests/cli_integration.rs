use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::{TempDir, tempdir};
use vendetta_model::{
    MediaDownloadStatus, MediaKind, MediaRecord, MediaVerificationStatus, MessageId, MessageKey,
    MessageRecord, MessageState, PeerId, PeerRecord, PeerType,
};
use vendetta_storage::ArchiveDb;

fn create_fixture_db(dir: &TempDir) -> std::path::PathBuf {
    let db_path = dir.path().join("fixture_archive.db");
    let db = ArchiveDb::open(&db_path).unwrap();

    let peer = PeerRecord {
        peer_id: PeerId::new(1001),
        peer_type: PeerType::Group,
        name: Some("CLI Test Group".to_string()),
        username: Some("clitest".to_string()),
        phone: None,
        raw_tl: None,
        updated_at: 1700000000,
    };
    db.upsert_peer(&peer).unwrap();

    let mut msgs = Vec::new();
    for i in 1..=5 {
        msgs.push(MessageRecord {
            key: MessageKey::new(peer.peer_id, MessageId::new(i)),
            date: 1700000000 + i * 60,
            sender_id: Some(peer.peer_id),
            text: Some(format!("CLI test message {i}")),
            entities_json: None,
            edit_date: None,
            state: MessageState::Active,
            reply_to_msg_id: if i > 1 {
                Some(MessageId::new(i - 1))
            } else {
                None
            },
            reply_to_top_id: None,
            reply_to_peer_id: None,
            grouped_id: None,
            forward_json: None,
            reactions_json: None,
            views: Some(10),
            forwards_count: Some(1),
            raw_tl: None,
        });
    }
    db.insert_messages_batch(&msgs).unwrap();

    db_path
}

fn setup_test_archive(path: &std::path::Path) -> ArchiveDb {
    let db = ArchiveDb::open(path).expect("Failed to open test database");
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO peers (peer_id, peer_type, name, updated_at) VALUES (100, 'user', 'Alice', 1700000000)",
            [],
        )?;
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text)
             VALUES (100, 1, 100, 1700000001, 'active', 'Hello CLI')",
            [],
        )?;
        conn.execute(
            "INSERT INTO account_sync_state (account_id, pts, qts, date, seq, sync_uncertain, last_synced_at)
             VALUES ('main', 10, 10, 1700000000, 1, 0, 1700000000)",
            [],
        )?;
        Ok(())
    }).expect("Failed to seed database");
    db
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
fn export_html_replace_flag_and_filters() {
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

    let mut cmd2 = Command::cargo_bin("vendetta").unwrap();
    cmd2.args([
        "export-html",
        "--archive",
        db_path.to_str().unwrap(),
        "--output",
        out_dir.to_str().unwrap(),
        "--replace",
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
fn export_and_verify_handle_unicode_paths() {
    let tmp = tempdir().unwrap();
    let special_dir = tmp.path().join("Телеграм Архив (2026) 🚀");
    fs::create_dir_all(&special_dir).unwrap();

    let fixture_dir = tempfile::TempDir::new_in(&special_dir).unwrap();
    let db_path = create_fixture_db(&fixture_dir);
    let html_out = special_dir.join("экспорт html");

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
}

#[test]
fn cli_auth_status_and_invalid_arguments_exit_codes() {
    let tmp = tempdir().unwrap();
    let session_path = tmp.path().join("empty_session.json");

    let mut cmd_warn = Command::cargo_bin("vendetta").unwrap();
    cmd_warn
        .args([
            "auth",
            "status",
            "--session",
            session_path.to_str().unwrap(),
        ])
        .assert()
        .code(1);

    let mut cmd_json = Command::cargo_bin("vendetta").unwrap();
    cmd_json
        .args([
            "auth",
            "status",
            "--session",
            session_path.to_str().unwrap(),
            "--json",
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("\"status\": \"unauthorized\""));

    let mut cmd_invalid = Command::cargo_bin("vendetta").unwrap();
    cmd_invalid.args(["--non-existent-flag"]).assert().code(2);
}

#[test]
fn verify_archive_cli_options_and_flags() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let _db = setup_test_archive(&db_path);

    let mut cmd_err = Command::cargo_bin("vendetta").unwrap();
    cmd_err
        .args([
            "verify-archive",
            "--archive",
            db_path.to_str().unwrap(),
            "--search",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--search requires --html <EXPORT_DIR>",
        ));

    let mut cmd_clean = Command::cargo_bin("vendetta").unwrap();
    cmd_clean
        .args([
            "verify-archive",
            "--archive",
            db_path.to_str().unwrap(),
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"status\": \"passed\""));
}

#[test]
fn verify_archive_evaluates_exit_codes_and_strict_mode() {
    let dir = tempdir().unwrap();

    let mut fatal_cmd = Command::cargo_bin("vendetta").unwrap();
    fatal_cmd
        .args(["verify-archive", "--archive", "/non_existent/archive.db"])
        .assert()
        .code(3);

    let warn_path = dir.path().join("warn.db");
    let warn_db = setup_test_archive(&warn_path);
    warn_db
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO messages (peer_id, message_id, sender_id, date, state, edit_date, text)
                 VALUES (100, 2, 100, 1700000002, 'edited', 1700000010, 'Edited')",
                [],
            )?;
            Ok(())
        })
        .unwrap();

    let mut normal_cmd = Command::cargo_bin("vendetta").unwrap();
    normal_cmd
        .args(["verify-archive", "--archive", warn_path.to_str().unwrap()])
        .assert()
        .code(1);

    let mut strict_cmd = Command::cargo_bin("vendetta").unwrap();
    strict_cmd
        .args([
            "verify-archive",
            "--archive",
            warn_path.to_str().unwrap(),
            "--strict",
        ])
        .assert()
        .code(2);
}

#[test]
fn verify_media_detects_missing_and_corrupt_files() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("archive.db");
    let media_dir = tmp.path().join("media");
    fs::create_dir_all(&media_dir).unwrap();

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
        local_rel_path: Some("photos/test.jpg".to_string()),
        sha256: None,
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

    let mut cmd_missing = Command::cargo_bin("vendetta").unwrap();
    cmd_missing
        .args([
            "verify-media",
            "--archive",
            db_path.to_str().unwrap(),
            "--media-dir",
            media_dir.to_str().unwrap(),
        ])
        .assert()
        .code(2);

    let file_path = media_dir.join("photos/test.jpg");
    fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    fs::write(&file_path, b"short").unwrap();

    let mut cmd_corrupt = Command::cargo_bin("vendetta").unwrap();
    cmd_corrupt
        .args([
            "verify-media",
            "--archive",
            db_path.to_str().unwrap(),
            "--media-dir",
            media_dir.to_str().unwrap(),
        ])
        .assert()
        .code(2);
}
