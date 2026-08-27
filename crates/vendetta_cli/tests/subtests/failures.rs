//! Failure injection

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;
use vendetta_storage::ArchiveDb;

#[test]
fn verify_media_detects_missing_media_file() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("archive.db");
    let media_root = tmp.path().join("media");
    fs::create_dir_all(&media_root).unwrap();

    let db = ArchiveDb::open(&db_path).unwrap();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO media_objects (
                media_id, kind, mime_type, size_bytes, file_name, sha256,
                local_rel_path, download_status, downloaded_bytes, retry_count,
                verification_status, created_at, updated_at
             ) VALUES (
                'missing_media_1', 'photo', 'image/jpeg', 100, 'missing.jpg', 'abcdef123456',
                'photos/missing.jpg', 'completed', 100, 0, 'verified', 1700000000, 1700000000
             )",
            [],
        )?;
        Ok(())
    })
    .unwrap();

    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.args([
        "verify-media",
        "--archive",
        db_path.to_str().unwrap(),
        "--media-dir",
        media_root.to_str().unwrap(),
    ])
    .assert()
    .failure()
    .code(predicate::eq(2));
}

#[test]
fn verify_media_detects_corrupted_media_size() {
    let tmp = tempdir().unwrap();
    let db_path = tmp.path().join("archive.db");
    let media_root = tmp.path().join("media");
    let file_path = media_root.join("photos/bad_size.jpg");
    fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    fs::write(&file_path, b"short").unwrap(); // 5 bytes instead of 100

    let db = ArchiveDb::open(&db_path).unwrap();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO media_objects (
                media_id, kind, mime_type, size_bytes, file_name, sha256,
                local_rel_path, download_status, downloaded_bytes, retry_count,
                verification_status, created_at, updated_at
             ) VALUES (
                'corrupt_size_1', 'photo', 'image/jpeg', 100, 'bad_size.jpg', NULL,
                'photos/bad_size.jpg', 'completed', 100, 0, 'verified', 1700000000, 1700000000
             )",
            [],
        )?;
        Ok(())
    })
    .unwrap();

    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.args([
        "verify-media",
        "--archive",
        db_path.to_str().unwrap(),
        "--media-dir",
        media_root.to_str().unwrap(),
    ])
    .assert()
    .failure()
    .code(predicate::eq(2));
}

#[test]
fn verify_archive_fails_on_missing_archive_db() {
    let mut cmd = Command::cargo_bin("vendetta").unwrap();
    cmd.args([
        "verify-archive",
        "--archive",
        "/nonexistent/path/archive.db",
    ])
    .assert()
    .failure()
    .code(predicate::eq(3));
}
