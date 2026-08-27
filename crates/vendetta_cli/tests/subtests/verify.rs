//! Archive and HTML verification

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;
use vendetta_render::{ExportOptions, HtmlArchiveExporter};
use vendetta_storage::ArchiveDb;

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
fn verify_archive_search_requires_html_dir() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let _db = setup_test_archive(&db_path);

    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.arg("verify-archive")
        .arg("--archive")
        .arg(&db_path)
        .arg("--search");
    cmd.assert().failure().stderr(predicate::str::contains(
        "--search requires --html <EXPORT_DIR>",
    ));
}

#[test]
fn verify_archive_rehash_requires_archive_path() {
    let dir = tempdir().unwrap();
    let export_dir = dir.path().join("export");
    fs::create_dir_all(&export_dir).unwrap();

    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.arg("verify-archive")
        .arg("--html")
        .arg(&export_dir)
        .arg("--rehash");
    cmd.assert().failure().stderr(predicate::str::contains(
        "--rehash requires --archive <PATH>",
    ));
}

#[test]
fn verify_archive_passes_on_clean_db() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let _db = setup_test_archive(&db_path);

    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.arg("verify-archive").arg("--archive").arg(&db_path);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("PASSED"))
        .stdout(predicate::str::contains(
            "Overall Status : \u{1b}[32mPASSED\u{1b}[0m (Exit Code: 0)",
        ));
}

#[test]
fn verify_archive_supports_media_and_replies_flags() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let _db = setup_test_archive(&db_path);

    let mut cmd_media = Command::cargo_bin("vendetta").unwrap();
    cmd_media
        .arg("verify-archive")
        .arg("--archive")
        .arg(&db_path)
        .arg("--media");
    cmd_media.assert().success();

    let mut cmd_replies = Command::cargo_bin("vendetta").unwrap();
    cmd_replies
        .arg("verify-archive")
        .arg("--archive")
        .arg(&db_path)
        .arg("--replies");
    cmd_replies.assert().success();
}

#[test]
fn verify_archive_json_output_is_structured() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let _db = setup_test_archive(&db_path);

    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.arg("verify-archive")
        .arg("--archive")
        .arg(&db_path)
        .arg("--json");

    let output = cmd.assert().success();
    let stdout_str = String::from_utf8(output.get_output().stdout.clone()).unwrap();

    let json_val: serde_json::Value =
        serde_json::from_str(&stdout_str).expect("Output must be valid JSON");
    assert_eq!(json_val["schema_version"], 1);
    assert_eq!(json_val["summary"]["status"], "passed");
    assert_eq!(json_val["summary"]["exit_code"], 0);
    assert_eq!(json_val["summary"]["scope"]["mode"], "full");
    assert_eq!(json_val["summary"]["scope"]["search_scope_executed"], false);
}

#[test]
fn verify_archive_warning_distinguishes_normal_from_strict() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = setup_test_archive(&db_path);

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, edit_date, text)
             VALUES (100, 2, 100, 1700000002, 'edited', 1700000010, 'Edited without rev')",
            [],
        )?;
        Ok(())
    })
    .unwrap();

    let mut cmd_normal = Command::cargo_bin("vendetta").unwrap();
    cmd_normal
        .arg("verify-archive")
        .arg("--archive")
        .arg(&db_path);

    cmd_normal
        .assert()
        .code(1)
        .stdout(predicate::str::contains("WARNINGS"));

    let mut cmd_strict = Command::cargo_bin("vendetta").unwrap();
    cmd_strict
        .arg("verify-archive")
        .arg("--archive")
        .arg(&db_path)
        .arg("--strict");

    cmd_strict
        .assert()
        .code(2)
        .stdout(predicate::str::contains("WARNINGS"));
}

#[test]
fn verify_archive_audits_standalone_and_corrupted_html() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let export_dir = dir.path().join("export");
    let db = setup_test_archive(&db_path);

    let exp_opts = ExportOptions {
        output_dir: export_dir.clone(),
        presentation_mode: vendetta_render::PresentationMode::TelegramLike,
        media_mode: vendetta_render::MediaMode::Copy,
        theme: vendetta_render::ThemeMode::System,
        chunk_size: 100,
        replace: true,
        media_src_dir: None,
        include_service_messages: true,
        include_deleted_messages: true,
        include_edit_history: true,
        build_search_index: true,
        build_date_index: true,
        target_peers: None,
    };
    let exporter = HtmlArchiveExporter::new(&db, exp_opts);
    exporter.export().unwrap();

    let mut cmd_clean = Command::cargo_bin("vendetta").unwrap();
    cmd_clean
        .arg("verify-archive")
        .arg("--html")
        .arg(&export_dir);

    cmd_clean
        .assert()
        .success()
        .stdout(predicate::str::contains("PASSED"));

    let mut cmd_combined = Command::cargo_bin("vendetta").unwrap();
    cmd_combined
        .arg("verify-archive")
        .arg("--archive")
        .arg(&db_path)
        .arg("--html")
        .arg(&export_dir);

    cmd_combined
        .assert()
        .success()
        .stdout(predicate::str::contains("PASSED"));

    let css_file = export_dir.join("assets/css/main.css");
    fs::remove_file(css_file).unwrap();

    let mut cmd_broken = Command::cargo_bin("vendetta").unwrap();
    cmd_broken
        .arg("verify-archive")
        .arg("--html")
        .arg(&export_dir);

    cmd_broken
        .assert()
        .code(2)
        .stdout(predicate::str::contains("ERRORS"));
}

#[test]
fn verify_archive_evaluates_exit_codes_0_through_3() {
    let dir = tempdir().unwrap();

    let clean_path = dir.path().join("clean.db");
    let _clean_db = setup_test_archive(&clean_path);
    let mut cmd0 = Command::cargo_bin("vendetta").unwrap();
    cmd0.arg("verify-archive").arg("--archive").arg(&clean_path);
    cmd0.assert().code(0);

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
    let mut cmd1 = Command::cargo_bin("vendetta").unwrap();
    cmd1.arg("verify-archive").arg("--archive").arg(&warn_path);
    cmd1.assert().code(1);

    let err_path = dir.path().join("err.db");
    let err_db = setup_test_archive(&err_path);
    err_db
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, entities_json)
                 VALUES (100, 10, 100, 1700000010, 'active', 'Short', '[{\"type\":\"bold\",\"offset\":100,\"length\":50}]')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let mut cmd2 = Command::cargo_bin("vendetta").unwrap();
    cmd2.arg("verify-archive").arg("--archive").arg(&err_path);
    cmd2.assert().code(2);

    let fatal_path = dir.path().join("non_existent.db");
    let mut cmd3 = Command::cargo_bin("vendetta").unwrap();
    cmd3.arg("verify-archive").arg("--archive").arg(&fatal_path);
    cmd3.assert().code(3);
}
