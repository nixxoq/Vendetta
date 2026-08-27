use std::fs;
use std::path::Path;
use tempfile::tempdir;
use vendetta_render::manifest::DatasetFingerprint;
use vendetta_render::{ExportOptions, HtmlArchiveExporter, MediaMode, PresentationMode, ThemeMode};
use vendetta_storage::ArchiveDb;
use vendetta_verify::VerificationEngine;
use vendetta_verify::model::*;

fn setup_test_db(path: &Path) -> ArchiveDb {
    let db = ArchiveDb::open(path).expect("Failed to open test database");
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO peers (peer_id, peer_type, name, updated_at) VALUES (100, 'user', 'Alice', 1700000000)",
            [],
        )?;
        Ok(())
    }).expect("Failed to insert seed peer");
    db
}

fn compute_file_sha256(path: &Path) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    if let Ok(mut f) = fs::File::open(path) {
        let mut buf = vec![0u8; 65536];
        while let Ok(n) = std::io::Read::read(&mut f, &mut buf) {
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }
    format!("{:x}", hasher.finalize())
}

#[test]
fn verifier_is_strictly_read_only_zero_mutation() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = setup_test_db(&db_path);

    // Insert test messages and set journal_mode = DELETE
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, edit_date, text, entities_json, reply_to_msg_id, reply_to_peer_id, reply_to_top_id, grouped_id, raw_tl)
             VALUES (100, 1, 100, 1700000001, 'active', NULL, 'Hello world', '[]', NULL, NULL, NULL, NULL, NULL)",
            [],
        )?;
        conn.execute_batch("PRAGMA journal_mode=DELETE;")?;
        Ok(())
    }).unwrap();

    drop(db); // Close write handle

    // Record file length and sha256 before verification
    let pre_meta = fs::metadata(&db_path).unwrap();
    let pre_len = pre_meta.len();
    let pre_hash = compute_file_sha256(&db_path);

    // Run full verification engine
    let opts = VerificationOptions {
        archive_path: Some(db_path.clone()),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: true,
        scope_replies: true,
        scope_search: false,
        rehash_media: true,
        strict: false,
    };
    let engine = VerificationEngine::new(opts);
    let report = engine.run().unwrap();

    assert_eq!(report.summary.exit_code, 0);

    // Assert that file on disk is byte-for-byte identical after verification
    let post_meta = fs::metadata(&db_path).unwrap();
    let post_len = post_meta.len();
    let post_hash = compute_file_sha256(&db_path);

    assert_eq!(
        pre_len, post_len,
        "Database file length mutated during verification!"
    );
    assert_eq!(
        pre_hash, post_hash,
        "Database file content mutated during verification!"
    );

    // Assert that no temporary -wal or -shm or journal files were left behind by verifier
    let parent = db_path.parent().unwrap();
    for entry in fs::read_dir(parent).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        assert!(
            !name.ends_with("-wal") && !name.ends_with("-shm") && !name.ends_with("-journal"),
            "Verifier created persistent journal/WAL side effect: {name}"
        );
    }

    // Also test zero-mutation when source DB is in WAL mode
    let db_wal = ArchiveDb::open(&db_path).unwrap();
    db_wal
        .with_conn(|conn| {
            conn.execute_batch("PRAGMA journal_mode=WAL;")?;
            conn.execute(
                "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text) VALUES (100, 2, 100, 1700000002, 'active', 'WAL message')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    drop(db_wal);

    let wal_pre_hash = compute_file_sha256(&db_path);
    let wal_file_path = dir.path().join("archive.db-wal");
    let wal_file_pre_hash = compute_file_sha256(&wal_file_path);

    let engine_wal = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path.clone()),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: true,
        scope_replies: true,
        scope_search: false,
        rehash_media: true,
        strict: false,
    });
    let report_wal = engine_wal.run().unwrap();
    assert_eq!(report_wal.summary.exit_code, 0);

    let wal_post_hash = compute_file_sha256(&db_path);
    assert_eq!(
        wal_pre_hash, wal_post_hash,
        "WAL database file mutated during verification"
    );
    assert_eq!(
        wal_file_pre_hash,
        compute_file_sha256(&wal_file_path),
        "WAL sidecar mutated during verification"
    );
}

#[test]
fn dataset_fingerprints_are_compatible_across_components() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let export_dir = dir.path().join("export");
    let db = setup_test_db(&db_path);

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text)
             VALUES (100, 1, 100, 1700000001, 'active', 'Hello export')",
            [],
        )?;
        Ok(())
    })
    .unwrap();

    // 1. Export HTML using shared DatasetFingerprint
    let opts = ExportOptions {
        output_dir: export_dir.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        theme: ThemeMode::System,
        chunk_size: 100,
        media_mode: MediaMode::Copy,
        replace: true,
        media_src_dir: None,
        include_service_messages: true,
        include_deleted_messages: true,
        include_edit_history: true,
        build_search_index: true,
        build_date_index: true,
        target_peers: None,
    };
    let exporter = HtmlArchiveExporter::new(&db, opts);
    let summary = exporter.export().unwrap();
    assert_eq!(summary.messages_count, 1);

    // Ensure compute_from_db produces identical result
    let direct_fingerprint = DatasetFingerprint::compute_from_db(&db).unwrap();
    let manifest = vendetta_render::manifest::HtmlExportManifest::read_from_file(
        &export_dir.join("manifest.json"),
    )
    .unwrap();
    assert_eq!(
        manifest.source_fingerprint.source_digest,
        direct_fingerprint.source_digest
    );

    // Run verification with both --archive and --html
    let verify_opts = VerificationOptions {
        archive_path: Some(db_path.clone()),
        html_dir: Some(export_dir.clone()),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(verify_opts);
    let report = engine.run().unwrap();

    assert_eq!(report.summary.exit_code, 0);
    assert_eq!(report.summary.dimensions.html_export.status, "consistent");
    assert!(
        report
            .summary
            .dimensions
            .html_export
            .reason
            .contains("source fingerprint matches archive")
    );

    // 2. Modify message text in database -> Source fingerprint will mismatch
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE messages SET text = 'Modified text' WHERE message_id = 1",
            [],
        )?;
        Ok(())
    })
    .unwrap();

    let verify_opts2 = VerificationOptions {
        archive_path: Some(db_path),
        html_dir: Some(export_dir),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine2 = VerificationEngine::new(verify_opts2);
    let report2 = engine2.run().unwrap();
    assert_eq!(report2.summary.exit_code, 2); // Error exit
    assert!(
        report2
            .findings
            .iter()
            .any(|f| f.code == "HTML_SOURCE_FINGERPRINT_MISMATCH")
    );
    assert_eq!(report2.summary.dimensions.html_export.status, "mismatched");
}

#[test]
fn html_only_verification_runs_without_archive_database() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let export_dir = dir.path().join("export");
    let db = setup_test_db(&db_path);

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text)
             VALUES (100, 1, 100, 1700000001, 'active', 'Hello standalone HTML test')",
            [],
        )?;
        Ok(())
    })
    .unwrap();

    let opts = ExportOptions {
        output_dir: export_dir.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        theme: ThemeMode::System,
        chunk_size: 100,
        media_mode: MediaMode::Copy,
        replace: true,
        media_src_dir: None,
        include_service_messages: true,
        include_deleted_messages: true,
        include_edit_history: true,
        build_search_index: true,
        build_date_index: true,
        target_peers: None,
    };
    let exporter = HtmlArchiveExporter::new(&db, opts);
    exporter.export().unwrap();

    // Verify HTML standalone with archive_path = None
    let verify_opts = VerificationOptions {
        archive_path: None,
        html_dir: Some(export_dir),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(verify_opts);
    let report = engine.run().unwrap();

    // Should succeed with exit code 0 and NOT emit source fingerprint mismatch
    assert_eq!(report.summary.exit_code, 0);
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == "HTML_SOURCE_FINGERPRINT_MISMATCH")
    );
    assert_eq!(report.summary.dimensions.html_export.status, "consistent");
    assert!(
        report
            .summary
            .dimensions
            .html_export
            .reason
            .contains("source archive equivalence was not checked")
    );
}

#[test]
fn reply_graph_resolves_targets_and_detects_cycles() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = setup_test_db(&db_path);

    db.with_conn(|conn| {
        // Add peer 200 (archived channel)
        conn.execute(
            "INSERT INTO peers (peer_id, peer_type, name, updated_at) VALUES (200, 'channel', 'Channel 200', 1700000000)",
            [],
        )?;

        // Message 1 in peer 100 (Normal root)
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
             VALUES (100, 1, 100, 1700000001, 'active', 'Root msg', NULL)",
            [],
        )?;

        // Message 1 in peer 200 (Normal target in peer 200)
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
             VALUES (200, 1, 200, 1700000001, 'active', 'Peer 200 Root', NULL)",
            [],
        )?;

        // Message in peer 300 (Peer record missing from peers table -> MESSAGE_WITHOUT_PEER_RECORD)
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text)
             VALUES (300, 1, 300, 1700000001, 'active', 'Unrecorded peer msg')",
            [],
        )?;

        // Message 2 replies to 1 in peer 100 (Resolved internal)
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
             VALUES (100, 2, 100, 1700000002, 'active', 'Reply to 1', 1)",
            [],
        )?;

        // Message 3 is Deleted in peer 100
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
             VALUES (100, 3, 100, 1700000003, 'deleted', 'Deleted msg', NULL)",
            [],
        )?;

        // Message 4 replies to 3 (Unavailable)
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
             VALUES (100, 4, 100, 1700000004, 'active', 'Reply to deleted', 3)",
            [],
        )?;

        // Message 5 replies to 999 in peer 100 (Missing in archived peer)
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
             VALUES (100, 5, 100, 1700000005, 'active', 'Reply to missing', 999)",
            [],
        )?;

        // Message 6 in peer 100 replies to message 1 in peer 200 (Archived cross-peer reply -> Resolved!)
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id, reply_to_peer_id)
             VALUES (100, 6, 100, 1700000006, 'active', 'Cross peer reply to archived peer 200', 1, 200)",
            [],
        )?;

        // Message 7 in peer 100 replies to peer 9999 (Unarchived missing peer)
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id, reply_to_peer_id)
             VALUES (100, 7, 100, 1700000007, 'active', 'Reply to missing peer 9999', 10, 9999)",
            [],
        )?;

        // Message 10 Self-Cycle (A -> A)
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
             VALUES (100, 10, 100, 1700000010, 'active', 'Self reply', 10)",
            [],
        )?;

        // Messages 20 & 21: 2-node cycle (20 -> 21 -> 20)
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
             VALUES (100, 20, 100, 1700000020, 'active', 'Cycle 20', 21)",
            [],
        )?;
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
             VALUES (100, 21, 100, 1700000021, 'active', 'Cycle 21', 20)",
            [],
        )?;

        // Generate 32-node cycle: 101 -> 102 -> ... -> 132 -> 101
        for i in 101..=132 {
            let next_target = if i == 132 { 101 } else { i + 1 };
            conn.execute(
                &format!(
                    "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
                     VALUES (100, {i}, 100, 1700000000 + {i}, 'active', 'Cycle {i}', {next_target})"
                ),
                [],
            )?;
        }

        // Generate 33-node cycle: 201 -> 202 -> ... -> 233 -> 201
        for i in 201..=233 {
            let next_target = if i == 233 { 201 } else { i + 1 };
            conn.execute(
                &format!(
                    "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
                     VALUES (100, {i}, 100, 1700000000 + {i}, 'active', 'Cycle {i}', {next_target})"
                ),
                [],
            )?;
        }

        // Generate 60 independent 2-node cycles to prove absence of LIMIT 50 truncation
        for k in 0..60 {
            let a = 1000 + k * 2;
            let b = 1000 + k * 2 + 1;
            conn.execute(
                &format!(
                    "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
                     VALUES (100, {a}, 100, 1700000000 + {a}, 'active', 'Indep cycle {a}', {b})"
                ),
                [],
            )?;
            conn.execute(
                &format!(
                    "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
                     VALUES (100, {b}, 100, 1700000000 + {b}, 'active', 'Indep cycle {b}', {a})"
                ),
                [],
            )?;
        }

        Ok(())
    }).unwrap();

    let opts = VerificationOptions {
        archive_path: Some(db_path),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: true,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(opts);
    let report = engine.run().unwrap();

    let rm = report.summary.reply_metrics.as_ref().unwrap();
    // Message 2 and Message 6 are resolved
    assert!(rm.resolved >= 2, "Expected at least 2 resolved replies");
    assert!(
        rm.unavailable >= 1,
        "Expected at least 1 unavailable reply target"
    );
    assert_eq!(
        rm.cross_peer, 2,
        "Expected exact 2 cross-peer replies (peer 200 and peer 9999)"
    );
    assert!(rm.missing >= 1, "Expected at least 1 missing reply target");
    assert_eq!(rm.self_cycles, 1, "Expected exact 1 self-cycle");
    // Exactly 63 distinct cycles (60 independent + 1 2-node + 1 32-node + 1 33-node)
    assert!(
        rm.cycles >= 63,
        "Expected at least 63 cycles, found {}",
        rm.cycles
    );

    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "REPLY_SELF_REFERENCE")
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "REPLY_CYCLE_DETECTED")
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "REPLY_TARGET_OUT_OF_SCOPE")
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "MESSAGE_WITHOUT_PEER_RECORD")
    );
}

#[test]
fn verifier_enforces_revision_and_message_state_invariants() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = setup_test_db(&db_path);

    db.with_conn(|conn| {
        // Message 1: state 'edited', but 0 revisions -> WARNING, not Error
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, edit_date)
             VALUES (100, 1, 100, 1700000001, 'edited', 'Edited text', 1700000010)",
            [],
        )?;

        // Message 2: Non-monotonic revision sequence (rev 2 edit_date earlier than rev 1)
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, edit_date)
             VALUES (100, 2, 100, 1700000002, 'edited', 'Current text', 1700000020)",
            [],
        )?;
        conn.execute(
            "INSERT INTO message_revisions (peer_id, message_id, revision_id, edit_date, text, entities_json, captured_at)
             VALUES (100, 2, 1, 1700000015, 'Rev 1', '[]', 1700000015)",
            [],
        )?;
        conn.execute(
            "INSERT INTO message_revisions (peer_id, message_id, revision_id, edit_date, text, entities_json, captured_at)
             VALUES (100, 2, 2, 1700000010, 'Rev 2 earlier date', '[]', 1700000010)",
            [],
        )?;

        // Message 3: Malformed entities JSON
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, entities_json)
             VALUES (100, 3, 100, 1700000003, 'active', 'Hello', '{broken_json')",
            [],
        )?;

        // Message 4: UTF-16 entity offset out of bounds
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, entities_json)
             VALUES (100, 4, 100, 1700000004, 'active', 'Short', '[{\"type\":\"bold\",\"offset\":10,\"length\":5}]')",
            [],
        )?;

        Ok(())
    }).unwrap();

    let opts = VerificationOptions {
        archive_path: Some(db_path),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(opts);
    let report = engine.run().unwrap();

    // Check warning on message 1
    let edited_warn = report
        .findings
        .iter()
        .find(|f| f.code == "EDITED_WITHOUT_REVISION_HISTORY")
        .unwrap();
    assert_eq!(edited_warn.severity, FindingSeverity::Warning);

    // Check errors
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "REVISION_ORDER_INVALID")
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "ENTITIES_JSON_MALFORMED")
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "ENTITY_UTF16_OUT_OF_BOUNDS")
    );
}

#[test]
fn forward_compatible_raw_tl_emits_warnings_not_errors() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = setup_test_db(&db_path);

    db.with_conn(|conn| {
        // Message 1: Unknown constructor (0xDEADBEEF) -> Should be classified as WARNING
        let unknown_tl = vec![0xEF, 0xBE, 0xAD, 0xDE, 0x01, 0x02, 0x03, 0x04];
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, raw_tl)
             VALUES (100, 1, 100, 1700000001, 'active', 'Unknown TL', ?1)",
            [&unknown_tl],
        )?;

        // Message 2: Known constructor (message#38114ee1) with truncated/corrupt bytes -> ERROR
        let corrupt_known_tl = vec![0xe1, 0x4e, 0x11, 0x38, 0x00];
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, raw_tl)
             VALUES (100, 2, 100, 1700000002, 'active', 'Corrupted known TL', ?1)",
            [&corrupt_known_tl],
        )?;

        Ok(())
    })
    .unwrap();

    let opts = VerificationOptions {
        archive_path: Some(db_path),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(opts);
    let report = engine.run().unwrap();

    let unknown_finding = report
        .findings
        .iter()
        .find(|f| f.code == "TL_UNKNOWN_CONSTRUCTOR")
        .unwrap();
    assert_eq!(unknown_finding.severity, FindingSeverity::Warning);

    let corrupt_finding = report
        .findings
        .iter()
        .find(|f| f.code == "TL_MALFORMED_BYTES")
        .unwrap();
    assert_eq!(corrupt_finding.severity, FindingSeverity::Error);
}

#[test]
fn media_verifier_supports_opt_in_rehashing_and_hardlinks() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let media_dir = dir.path().join("media");
    fs::create_dir_all(&media_dir).unwrap();
    let db = setup_test_db(&db_path);

    let media_file_path = media_dir.join("photo_1.jpg");
    fs::write(&media_file_path, b"IMAGE_BINARY_DATA_12345").unwrap();
    let real_hash = compute_file_sha256(&media_file_path);

    // Create a hardlink to verify inode deduplication acceptance
    let media_hardlink_path = media_dir.join("photo_2.jpg");
    fs::hard_link(&media_file_path, &media_hardlink_path).unwrap();

    db.with_conn(|conn| {
        // Record 1: Completed media with correct size and wrong recorded hash
        conn.execute(
            "INSERT INTO media_objects (media_id, kind, size_bytes, sha256, local_rel_path, download_status, verification_status, created_at, updated_at)
             VALUES ('m1', 'photo', 23, 'WRONG_HASH_00000000000000000000000000000000000000000000000000000000', 'photo_1.jpg', 'completed', 'verified', 1700000000, 1700000000)",
            [],
        )?;

        // Record 2: Hardlinked media object pointing to photo_2.jpg
        conn.execute(
            &format!(
                "INSERT INTO media_objects (media_id, kind, size_bytes, sha256, local_rel_path, download_status, verification_status, created_at, updated_at)
                 VALUES ('m2', 'photo', 23, '{real_hash}', 'photo_2.jpg', 'completed', 'verified', 1700000000, 1700000000)"
            ),
            [],
        )?;

        Ok(())
    })
    .unwrap();

    // 1. Without rehash (normal mode) -> Should only check size and existence (No hash mismatch finding)
    let opts_no_rehash = VerificationOptions {
        archive_path: Some(db_path.clone()),
        html_dir: None,
        media_dir: Some(media_dir.clone()),
        mode: VerificationMode::Full,
        scope_media: true,
        scope_replies: false,
        scope_search: false,
        rehash_media: false, // Opt-out (default)
        strict: false,
    };
    let engine1 = VerificationEngine::new(opts_no_rehash);
    let report1 = engine1.run().unwrap();
    assert_eq!(report1.summary.exit_code, 0);
    assert!(
        !report1
            .findings
            .iter()
            .any(|f| f.code == "MEDIA_HASH_MISMATCH")
    );

    // 2. With opt-in --rehash -> Should detect hash mismatch on m1
    let opts_rehash = VerificationOptions {
        archive_path: Some(db_path),
        html_dir: None,
        media_dir: Some(media_dir),
        mode: VerificationMode::Full,
        scope_media: true,
        scope_replies: false,
        scope_search: false,
        rehash_media: true, // Opt-in
        strict: false,
    };
    let engine2 = VerificationEngine::new(opts_rehash);
    let report2 = engine2.run().unwrap();
    assert_eq!(report2.summary.exit_code, 2);
    assert!(
        report2
            .findings
            .iter()
            .any(|f| f.code == "MEDIA_HASH_MISMATCH")
    );
}

#[test]
fn repair_plan_classifies_safety_actions() {
    let findings = vec![
        VerificationFinding {
            code: "ORPHAN_PART_FILE".to_string(),
            severity: FindingSeverity::Warning,
            category: FindingCategory::Filesystem,
            peer_id: None,
            message_id: None,
            media_id: None,
            path: Some("file.part".to_string()),
            description: "Stale part file".to_string(),
            evidence: Some(serde_json::json!({
                "no_active_lease": true,
                "no_matching_media_object": true,
            })),
            recommendation: Some("Delete part file".to_string()),
        },
        VerificationFinding {
            code: "SEARCH_SHARD_MISSING".to_string(),
            severity: FindingSeverity::Error,
            category: FindingCategory::Search,
            peer_id: None,
            message_id: None,
            media_id: None,
            path: Some("search/shards/shard_00000.js".to_string()),
            description: "Missing shard".to_string(),
            evidence: None,
            recommendation: Some("Rebuild search".to_string()),
        },
        VerificationFinding {
            code: "MEDIA_FILE_MISSING".to_string(),
            severity: FindingSeverity::Error,
            category: FindingCategory::Media,
            peer_id: None,
            message_id: None,
            media_id: Some("m1".to_string()),
            path: Some("photo.jpg".to_string()),
            description: "Missing media".to_string(),
            evidence: None,
            recommendation: Some("Redownload".to_string()),
        },
    ];

    let plan = vendetta_verify::repair::RepairPlanner::build_plan(&findings);
    assert_eq!(plan.safe_automation_count, 1);
    assert_eq!(plan.manual_review_count, 1);
    assert_eq!(plan.requires_resync_count, 1);

    let safe_rec = plan
        .recommendations
        .iter()
        .find(|r| r.category == RepairCategory::SafeAutomation)
        .unwrap();
    assert_eq!(safe_rec.action_code, "CLEANUP_ORPHAN_PART_FILES");
    assert!(!safe_rec.why_safe_or_risky.is_empty());

    let manual_rec = plan
        .recommendations
        .iter()
        .find(|r| r.category == RepairCategory::ManualReview)
        .unwrap();
    assert_eq!(manual_rec.action_code, "REBUILD_SEARCH_INDEX");

    let resync_rec = plan
        .recommendations
        .iter()
        .find(|r| r.category == RepairCategory::RequiresTelegramResync)
        .unwrap();
    assert_eq!(resync_rec.action_code, "REDOWNLOAD_MISSING_MEDIA");
}

#[test]
fn completeness_report_emits_multidimensional_json() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = setup_test_db(&db_path);

    // Set sync_uncertain to true and mark a blocked channel in channel_sync_queue
    db.with_conn(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO account_sync_state (account_id, pts, qts, date, seq, sync_uncertain, last_synced_at)
             VALUES ('main', 100, 10, 1700000000, 1, 1, 1700000000)",
            [],
        )?;
        conn.execute(
            "INSERT INTO channel_sync_queue (peer_id, discovered_pts, status, last_error, updated_at)
             VALUES (999, 1000, 'blocked', 'CHANNEL_PRIVATE_ACCESS_DENIED', 1700000000)",
            [],
        )?;
        Ok(())
    }).unwrap();

    let opts = VerificationOptions {
        archive_path: Some(db_path),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(opts);
    let report = engine.run().unwrap();

    // Check separate dimensions
    assert_eq!(
        report.summary.dimensions.message_history.status,
        "uncertain"
    );
    assert_eq!(
        report.summary.dimensions.channel_discovery.status,
        "incomplete"
    );
    assert_eq!(
        report.summary.dimensions.sync_uncertainty.status,
        "uncertain"
    );
    assert_eq!(
        report.summary.dimensions.deletion_verification.status,
        "uncertain"
    );
    assert_eq!(
        report.summary.dimensions.media_binaries.status,
        "not_applicable"
    );
    assert_eq!(
        report.summary.dimensions.html_export.status,
        "not_applicable"
    );
    assert_eq!(
        report.summary.dimensions.search_index.status,
        "not_applicable"
    );

    // Check JSON serialization
    let json_str = vendetta_verify::format_json(&report).unwrap();
    assert!(json_str.contains("\"message_history\""));
    assert!(json_str.contains("\"channel_discovery\""));
    assert!(json_str.contains("\"sync_uncertainty\""));
    assert!(json_str.contains("\"schema_version\": 1"));
    assert!(json_str.contains("\"scope\""));
    assert!(json_str.contains("\"core_db_auditors_executed\""));
}

#[test]
fn migration_verifier_passes_in_absence_of_temp_tables() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let _db = setup_test_db(&db_path);

    let opts = VerificationOptions {
        archive_path: Some(db_path),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(opts);
    let report = engine.run().unwrap();

    // Clean database after migration 0003 should NOT have _media_id_migration_map
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == "MIGRATION_TEMP_TABLE_LEFTOVER")
    );
}

#[test]
fn auditor_respects_scope_matrix_filters() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let export_dir = dir.path().join("export");
    let db = setup_test_db(&db_path);

    let exp_opts = ExportOptions {
        output_dir: export_dir.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        theme: ThemeMode::System,
        chunk_size: 100,
        media_mode: MediaMode::Copy,
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

    // 1. Default / Full (DB only) -> reply_metrics and media_metrics are None
    let rep_full = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path.clone()),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(rep_full.summary.reply_metrics, None);
    assert_eq!(rep_full.summary.media_metrics, None);
    assert_eq!(
        rep_full.summary.dimensions.media_binaries.status,
        "not_applicable"
    );
    assert!(!rep_full.summary.scope.search_scope_executed);
    assert_eq!(rep_full.summary.scope.core_db_auditors_executed.len(), 11);

    // 2. Fast mode -> fast DB only
    let rep_fast = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path.clone()),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Fast,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(rep_fast.summary.reply_metrics, None);
    assert_eq!(rep_fast.summary.media_metrics, None);
    assert_eq!(
        rep_fast.summary.dimensions.channel_discovery.status,
        "not_applicable"
    );
    assert_eq!(rep_fast.summary.scope.core_db_auditors_executed.len(), 4);

    // 3. --media only -> media_metrics is Some, reply_metrics is None
    let rep_media = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path.clone()),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: true,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert!(rep_media.summary.media_metrics.is_some());
    assert_eq!(rep_media.summary.reply_metrics, None);
    assert_eq!(
        rep_media.summary.dimensions.media_binaries.status,
        "complete"
    );

    // 4. --replies only -> reply_metrics is Some, media_metrics is None
    let rep_replies = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path.clone()),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: true,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert!(rep_replies.summary.reply_metrics.is_some());
    assert_eq!(rep_replies.summary.media_metrics, None);

    // 5. --media --replies -> both Some
    let rep_both = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path.clone()),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: true,
        scope_replies: true,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert!(rep_both.summary.media_metrics.is_some());
    assert!(rep_both.summary.reply_metrics.is_some());

    // 6. --html only -> HTML + Search run
    let rep_html = VerificationEngine::new(VerificationOptions {
        archive_path: None,
        html_dir: Some(export_dir.clone()),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(rep_html.summary.dimensions.html_export.status, "consistent");
    assert_eq!(rep_html.summary.dimensions.search_index.status, "complete");
    assert_eq!(
        rep_html.summary.dimensions.message_history.status,
        "not_applicable"
    );
    assert!(rep_html.summary.scope.search_scope_executed);
    assert!(!rep_html.summary.scope.search_scope_requested);
    assert!(rep_html.summary.scope.core_db_auditors_executed.is_empty());

    // 7. --archive --html -> DB + HTML + Search run
    let rep_arch_html = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path),
        html_dir: Some(export_dir),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_arch_html.summary.dimensions.html_export.status,
        "consistent"
    );
    assert_eq!(
        rep_arch_html.summary.dimensions.search_index.status,
        "complete"
    );
    assert!(rep_arch_html.summary.scope.search_scope_executed);
}

#[test]
fn completeness_auditor_prevents_false_completeness_claims() {
    let dir = tempdir().unwrap();
    let export_dir = dir.path().join("export");
    fs::create_dir_all(export_dir.join("search")).unwrap();

    // Write a broken search manifest pointing to non-existent shards
    fs::write(
        export_dir.join("search/manifest.js"),
        "window.__VENDETTA_SEARCH_MANIFEST__ = {\"total_entries\":10,\"shards\":[{\"shard_id\":1,\"file_name\":\"shard_00001.js\",\"entries_count\":10,\"peer_ids\":[100],\"min_date\":1700000000,\"max_date\":1700000001}],\"peers\":[],\"prefix_index\":{\"test\":[1]}};",
    )
    .unwrap();

    let opts = VerificationOptions {
        archive_path: None,
        html_dir: Some(export_dir),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: true,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(opts);
    let report = engine.run().unwrap();

    // Since shard_00001.js is missing, search_index MUST report corrupted and NOT complete!
    assert_eq!(report.summary.dimensions.search_index.status, "corrupted");
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "SEARCH_SHARD_MISSING")
    );
}

#[test]
fn media_auditor_flags_unverified_and_stale_worker_leases() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let media_dir = dir.path().join("media");
    fs::create_dir_all(&media_dir).unwrap();
    let db = setup_test_db(&db_path);

    let media_file = media_dir.join("photo_unverified.jpg");
    fs::write(&media_file, b"BINARY_DATA").unwrap();

    db.with_conn(|conn| {
        // Record 1: completed + unverified
        conn.execute(
            "INSERT INTO media_objects (media_id, kind, size_bytes, local_rel_path, download_status, verification_status, created_at, updated_at)
             VALUES ('m_unver', 'photo', 11, 'photo_unverified.jpg', 'completed', 'unverified', 1700000000, 1700000000)",
            [],
        )?;

        // Record 2: downloading with stale claim (claimed 10000 seconds ago)
        conn.execute(
            "INSERT INTO media_objects (media_id, kind, size_bytes, local_rel_path, download_status, verification_status, worker_id, claimed_at, created_at, updated_at)
             VALUES ('m_stale', 'photo', 100, 'photo_stale.jpg', 'downloading', 'unverified', 'worker_1', 1000, 1000, 1000)",
            [],
        )?;

        // Record 3: retry_wait with invalid next_retry_at (NULL)
        conn.execute(
            "INSERT INTO media_objects (media_id, kind, size_bytes, download_status, verification_status, next_retry_at, created_at, updated_at)
             VALUES ('m_retry', 'photo', 100, 'retry_wait', 'unverified', NULL, 1700000000, 1700000000)",
            [],
        )?;

        Ok(())
    })
    .unwrap();

    let opts = VerificationOptions {
        archive_path: Some(db_path),
        html_dir: None,
        media_dir: Some(media_dir),
        mode: VerificationMode::Full,
        scope_media: true,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(opts);
    let report = engine.run().unwrap();

    assert!(report.findings.iter().any(|f| f.code == "MEDIA_UNVERIFIED"));
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "MEDIA_STALE_CLAIM")
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "MEDIA_RETRY_WAIT_INVALID")
    );

    let mm = report.summary.media_metrics.as_ref().unwrap();
    // completed but unverified is NOT counted as completed_verified_on_disk
    assert_eq!(mm.completed_verified_on_disk, 0);
}

#[test]
fn finding_category_sorting_is_stable_and_canonical() {
    use vendetta_verify::status::category_sort_order;

    assert_eq!(category_sort_order(FindingCategory::Schema), 1);
    assert_eq!(
        category_sort_order(FindingCategory::ReferentialIntegrity),
        2
    );
    assert_eq!(category_sort_order(FindingCategory::Identity), 3);
    assert_eq!(category_sort_order(FindingCategory::Completeness), 20);

    // Canonical schema version constant is exported and >= 5
    assert_eq!(vendetta_storage::CURRENT_SCHEMA_VERSION, 5);
}

#[test]
fn cycle_detector_outputs_deterministic_json_for_deep_cycles() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = setup_test_db(&db_path);

    db.with_conn(|conn| {
        // Insert 60 independent 2-node cycles
        for k in 0..60 {
            let a = 1000 + k * 2;
            let b = 1000 + k * 2 + 1;
            conn.execute(
                &format!(
                    "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
                     VALUES (100, {a}, 100, 1700000000 + {a}, 'active', 'Cycle {a}', {b})"
                ),
                [],
            )?;
            conn.execute(
                &format!(
                    "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, reply_to_msg_id)
                     VALUES (100, {b}, 100, 1700000000 + {b}, 'active', 'Cycle {b}', {a})"
                ),
                [],
            )?;
        }
        Ok(())
    })
    .unwrap();

    let opts = VerificationOptions {
        archive_path: Some(db_path),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: true,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };

    let engine = VerificationEngine::new(opts);
    let mut rep1 = engine.run().unwrap();
    let mut rep2 = engine.run().unwrap();

    assert_eq!(rep1.findings, rep2.findings);
    assert_eq!(rep1.summary.reply_metrics, rep2.summary.reply_metrics);
    assert_eq!(rep1.summary.exit_code, rep2.summary.exit_code);
    assert_eq!(rep1.summary.total_findings, rep2.summary.total_findings);
    assert_eq!(rep1.summary.category_counts, rep2.summary.category_counts);
    assert_eq!(rep1.summary.dimensions, rep2.summary.dimensions);
    assert_eq!(rep1.repair_plan, rep2.repair_plan);
    assert_eq!(rep1.summary.reply_metrics.as_ref().unwrap().cycles, 60);

    // Normalize duration_ms for exact byte-for-byte JSON equality test
    rep1.summary.duration_ms = 42;
    rep2.summary.duration_ms = 42;
    let json1 = vendetta_verify::format_json(&rep1).unwrap();
    let json2 = vendetta_verify::format_json(&rep2).unwrap();
    assert_eq!(
        json1, json2,
        "JSON reports across two verification runs are not byte-for-byte identical"
    );
}

#[test]
fn html_only_verifier_is_strictly_zero_mutation() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let export_dir = dir.path().join("export");
    let db = setup_test_db(&db_path);

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text) VALUES (100, 1, 100, 1700000001, 'active', 'HTML zero mutation test')",
            [],
        )?;
        Ok(())
    }).unwrap();

    let opts = ExportOptions {
        output_dir: export_dir.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        theme: ThemeMode::System,
        chunk_size: 100,
        media_mode: MediaMode::Copy,
        replace: true,
        media_src_dir: None,
        include_service_messages: true,
        include_deleted_messages: true,
        include_edit_history: true,
        build_search_index: true,
        build_date_index: true,
        target_peers: None,
    };
    let exporter = HtmlArchiveExporter::new(&db, opts);
    exporter.export().unwrap();

    // Snapshot all files in export_dir
    let mut file_hashes_pre = std::collections::BTreeMap::new();
    for entry in walkdir(&export_dir) {
        if entry.is_file() {
            let rel = entry.strip_prefix(&export_dir).unwrap().to_path_buf();
            file_hashes_pre.insert(rel, compute_file_sha256(&entry));
        }
    }

    // Run standalone HTML verification
    let v_opts = VerificationOptions {
        archive_path: None,
        html_dir: Some(export_dir.clone()),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(v_opts);
    let report = engine.run().unwrap();
    assert_eq!(report.summary.exit_code, 0);

    // Verify all files are byte-for-byte identical after verification
    for (rel, pre_hash) in &file_hashes_pre {
        let path = export_dir.join(rel);
        let post_hash = compute_file_sha256(&path);
        assert_eq!(
            pre_hash,
            &post_hash,
            "HTML file {} was mutated during verification",
            rel.display()
        );
    }
}

fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                files.extend(walkdir(&p));
            } else {
                files.push(p);
            }
        }
    }
    files
}

#[test]
fn deletion_provenance_audits_tombstones_and_uncertainty() {
    let dir = tempdir().unwrap();

    // Scenario A: Clean sync, but 0 deleted messages and no deletion provenance -> uncertain
    let db_path_a = dir.path().join("archive_a.db");
    let db_a = setup_test_db(&db_path_a);
    db_a.with_conn(|conn| {
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text) VALUES (100, 1, 100, 1700000001, 'active', 'No deletions')",
            [],
        )?;
        Ok(())
    }).unwrap();
    let rep_a = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path_a),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_a.summary.dimensions.deletion_verification.status,
        "uncertain"
    );

    // Scenario B: Deleted tombstones present but NO sync_integrity_reports -> still uncertain!
    let db_path_b = dir.path().join("archive_b.db");
    let db_b = setup_test_db(&db_path_b);
    db_b.with_conn(|conn| {
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text) VALUES (100, 1, 100, 1700000001, 'deleted', 'Deleted message')",
            [],
        )?;
        Ok(())
    }).unwrap();
    let rep_b = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path_b),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_b.summary.dimensions.deletion_verification.status, "uncertain",
        "Deleted tombstones alone must NOT make deletion_verification complete!"
    );

    // Scenario C: Legacy report from old schema (historical_deletions_complete=1, but provenance_version=1 / no reconciliation) -> uncertain!
    let db_path_c = dir.path().join("archive_c.db");
    let db_c = setup_test_db(&db_path_c);
    db_c.with_conn(|conn| {
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text) VALUES (100, 1, 100, 1700000001, 'active', 'Legacy clean sync')",
            [],
        )?;
        conn.execute(
            "INSERT INTO sync_integrity_reports (scope, fully_lossless_contiguous_sync, current_history_repaired, new_messages_recovered, current_content_reconciled, historical_edits_complete, historical_deletions_complete, event_window_lost, channel_discovery_complete, created_at, provenance_version, deletion_reconciliation_performed, deletion_reconciliation_complete, deletion_event_gap_count, deletion_tombstones_reconciled, historical_message_reconciliation_performed, historical_message_reconciliation_complete, historical_message_gap_count)
             VALUES ('full_sync_run', 1, 1, 1, 1, 1, 1, 0, 1, 1700000000, 1, 0, 0, 0, 0, 0, 0, 0)",
            [],
        )?;
        Ok(())
    }).unwrap();
    let rep_c = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path_c),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_c.summary.dimensions.deletion_verification.status, "uncertain",
        "Legacy historical_deletions_complete=1 without provenance_version >= 2 must be uncertain!"
    );
    assert_eq!(
        rep_c.summary.dimensions.message_history.status, "uncertain",
        "Legacy report without explicit message reconciliation provenance must be uncertain!"
    );

    // Scenario D: Current report (provenance_version = 2, fully_lossless_contiguous_sync = 1, but historical_message_reconciliation_performed = 0) -> uncertain
    let db_path_d = dir.path().join("archive_d.db");
    let db_d = setup_test_db(&db_path_d);
    db_d.with_conn(|conn| {
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text) VALUES (100, 1, 100, 1700000001, 'active', 'Current sync without deletion sweep')",
            [],
        )?;
        conn.execute(
            "INSERT INTO sync_integrity_reports (scope, fully_lossless_contiguous_sync, current_history_repaired, new_messages_recovered, current_content_reconciled, historical_edits_complete, historical_deletions_complete, event_window_lost, channel_discovery_complete, created_at, provenance_version, deletion_reconciliation_performed, deletion_reconciliation_complete, deletion_event_gap_count, deletion_tombstones_reconciled, historical_message_reconciliation_performed, historical_message_reconciliation_complete, historical_message_gap_count)
             VALUES ('full_sync_run', 1, 1, 1, 1, 1, 0, 0, 1, 1700000000, 2, 0, 0, 0, 0, 0, 0, 0)",
            [],
        )?;
        Ok(())
    }).unwrap();
    let rep_d = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path_d),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_d.summary.dimensions.deletion_verification.status,
        "uncertain"
    );
    assert_eq!(
        rep_d.summary.dimensions.message_history.status, "uncertain",
        "Clean sync with fully_lossless_contiguous_sync=1 must NOT claim message_history=complete without explicit message reconciliation sweep!"
    );

    // Scenario E: Explicit completed reconciliation (provenance_version = 2, deletion_reconciliation_performed = 1, historical_message_reconciliation_performed = 1, complete = 1, gaps = 0) -> complete
    let db_path_e = dir.path().join("archive_e.db");
    let db_e = setup_test_db(&db_path_e);
    db_e.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sync_integrity_reports (scope, fully_lossless_contiguous_sync, current_history_repaired, new_messages_recovered, current_content_reconciled, historical_edits_complete, historical_deletions_complete, event_window_lost, channel_discovery_complete, created_at, provenance_version, deletion_reconciliation_performed, deletion_reconciliation_complete, deletion_event_gap_count, deletion_tombstones_reconciled, historical_message_reconciliation_performed, historical_message_reconciliation_complete, historical_message_gap_count)
             VALUES ('full_sync_run', 1, 1, 1, 1, 1, 1, 0, 1, 1700000000, 2, 1, 1, 0, 5, 1, 1, 0)",
            [],
        )?;
        Ok(())
    }).unwrap();
    let rep_e = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path_e),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_e.summary.dimensions.deletion_verification.status,
        "complete"
    );
    assert_eq!(
        rep_e.summary.dimensions.message_history.status, "complete",
        "Explicit message reconciliation with complete=1 and gaps=0 must be complete!"
    );

    // Scenario F: Explicit incomplete reconciliation (historical_message_reconciliation_performed = 1, historical_message_gap_count = 3) -> incomplete
    let db_path_f = dir.path().join("archive_f.db");
    let db_f = setup_test_db(&db_path_f);
    db_f.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sync_integrity_reports (scope, fully_lossless_contiguous_sync, current_history_repaired, new_messages_recovered, current_content_reconciled, historical_edits_complete, historical_deletions_complete, event_window_lost, channel_discovery_complete, created_at, provenance_version, deletion_reconciliation_performed, deletion_reconciliation_complete, deletion_event_gap_count, deletion_tombstones_reconciled, historical_message_reconciliation_performed, historical_message_reconciliation_complete, historical_message_gap_count)
             VALUES ('full_sync_run', 0, 1, 1, 1, 0, 0, 0, 1, 1700000000, 2, 1, 0, 3, 0, 1, 0, 3)",
            [],
        )?;
        Ok(())
    }).unwrap();
    let rep_f = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path_f),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_f.summary.dimensions.deletion_verification.status,
        "incomplete"
    );
    assert_eq!(
        rep_f.summary.dimensions.message_history.status, "incomplete",
        "Explicit message reconciliation with gaps > 0 must be incomplete!"
    );

    // Scenario G: Newest report is peer-scoped, older report is full_sync_run (complete) -> Archive uses full_sync_run
    let db_path_g = dir.path().join("archive_g.db");
    let db_g = setup_test_db(&db_path_g);
    db_g.with_conn(|conn| {
        // Older global report (complete)
        conn.execute(
            "INSERT INTO sync_integrity_reports (scope, peer_id, fully_lossless_contiguous_sync, current_history_repaired, new_messages_recovered, current_content_reconciled, historical_edits_complete, historical_deletions_complete, event_window_lost, channel_discovery_complete, created_at, provenance_version, deletion_reconciliation_performed, deletion_reconciliation_complete, deletion_event_gap_count, deletion_tombstones_reconciled, historical_message_reconciliation_performed, historical_message_reconciliation_complete, historical_message_gap_count)
             VALUES ('full_sync_run', NULL, 1, 1, 1, 1, 1, 1, 0, 1, 1700000000, 2, 1, 1, 0, 10, 1, 1, 0)",
            [],
        )?;
        // Newer peer-specific report (peer 100)
        conn.execute(
            "INSERT INTO sync_integrity_reports (scope, peer_id, fully_lossless_contiguous_sync, current_history_repaired, new_messages_recovered, current_content_reconciled, historical_edits_complete, historical_deletions_complete, event_window_lost, channel_discovery_complete, created_at, provenance_version, deletion_reconciliation_performed, deletion_reconciliation_complete, deletion_event_gap_count, deletion_tombstones_reconciled, historical_message_reconciliation_performed, historical_message_reconciliation_complete, historical_message_gap_count)
             VALUES ('peer', 100, 0, 0, 0, 0, 0, 0, 1, 0, 1700000100, 2, 0, 0, 0, 0, 0, 0, 0)",
            [],
        )?;
        Ok(())
    }).unwrap();
    let rep_g = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path_g),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_g.summary.dimensions.deletion_verification.status, "complete",
        "Archive-wide evaluation must ignore newer peer-scoped reports and use full_sync_run"
    );
    assert_eq!(
        rep_g.summary.dimensions.message_history.status, "complete",
        "Archive-wide evaluation must use global full_sync_run for message_history completeness"
    );

    // Scenario H: Only channel-scoped report exists -> Archive-wide is NOT complete
    let db_path_h = dir.path().join("archive_h.db");
    let db_h = setup_test_db(&db_path_h);
    db_h.with_conn(|conn| {
        conn.execute(
            "INSERT INTO sync_integrity_reports (scope, peer_id, fully_lossless_contiguous_sync, current_history_repaired, new_messages_recovered, current_content_reconciled, historical_edits_complete, historical_deletions_complete, event_window_lost, channel_discovery_complete, created_at, provenance_version, deletion_reconciliation_performed, deletion_reconciliation_complete, deletion_event_gap_count, deletion_tombstones_reconciled, historical_message_reconciliation_performed, historical_message_reconciliation_complete, historical_message_gap_count)
             VALUES ('channel', 200, 1, 1, 1, 1, 1, 1, 0, 1, 1700000100, 2, 1, 1, 0, 5, 1, 1, 0)",
            [],
        )?;
        Ok(())
    }).unwrap();
    let rep_h = VerificationEngine::new(VerificationOptions {
        archive_path: Some(db_path_h),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_h.summary.dimensions.deletion_verification.status, "uncertain",
        "Channel-scoped report must not establish global archive completeness"
    );
    assert_eq!(
        rep_h.summary.dimensions.channel_discovery.status,
        "uncertain"
    );
    assert_eq!(rep_h.summary.dimensions.message_history.status, "uncertain");
}

#[test]
fn search_completeness_matrix_validates_manifest_and_shards() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let export_dir = dir.path().join("export");
    let db = setup_test_db(&db_path);

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text) VALUES (100, 1, 100, 1700000001, 'active', 'Search matrix test')",
            [],
        )?;
        Ok(())
    }).unwrap();

    // 1. Valid export with search
    let opts = ExportOptions {
        output_dir: export_dir.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        theme: ThemeMode::System,
        chunk_size: 100,
        media_mode: MediaMode::Copy,
        replace: true,
        media_src_dir: None,
        include_service_messages: true,
        include_deleted_messages: true,
        include_edit_history: true,
        build_search_index: true,
        build_date_index: true,
        target_peers: None,
    };
    let exporter = HtmlArchiveExporter::new(&db, opts);
    exporter.export().unwrap();

    let rep_valid = VerificationEngine::new(VerificationOptions {
        archive_path: None,
        html_dir: Some(export_dir.clone()),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(rep_valid.summary.dimensions.search_index.status, "complete");

    // 2. Extra undeclared search shard -> search_index must NOT be complete!
    let undeclared_shard = export_dir.join("search/shards/shard_99999.js");
    fs::write(&undeclared_shard, "window.__VENDETTA_SEARCH_SHARDS__ = [];").unwrap();

    let rep_undeclared = VerificationEngine::new(VerificationOptions {
        archive_path: None,
        html_dir: Some(export_dir.clone()),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_undeclared.summary.dimensions.search_index.status,
        "incomplete"
    );
    assert!(
        rep_undeclared
            .findings
            .iter()
            .any(|f| f.code == "SEARCH_UNDECLARED_SHARD")
    );

    // Remove undeclared shard
    fs::remove_file(&undeclared_shard).unwrap();

    // 3. Intentionally disabled search -> search_index is not_applicable
    let disabled_dir = dir.path().join("export_disabled");
    let opts_disabled = ExportOptions {
        output_dir: disabled_dir.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        theme: ThemeMode::System,
        chunk_size: 100,
        media_mode: MediaMode::Copy,
        replace: true,
        media_src_dir: None,
        include_service_messages: true,
        include_deleted_messages: true,
        include_edit_history: true,
        build_search_index: false,
        build_date_index: true,
        target_peers: None,
    };
    let exporter_disabled = HtmlArchiveExporter::new(&db, opts_disabled);
    exporter_disabled.export().unwrap();

    let rep_disabled = VerificationEngine::new(VerificationOptions {
        archive_path: None,
        html_dir: Some(disabled_dir),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_disabled.summary.dimensions.search_index.status,
        "not_applicable"
    );
}

#[test]
fn html_manifest_and_fingerprint_error_matrix_detects_mismatches() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let export_dir = dir.path().join("export");
    let db = setup_test_db(&db_path);

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text) VALUES (100, 1, 100, 1700000001, 'active', 'HTML error matrix test')",
            [],
        )?;
        Ok(())
    }).unwrap();

    let opts = ExportOptions {
        output_dir: export_dir.clone(),
        presentation_mode: PresentationMode::TelegramLike,
        theme: ThemeMode::System,
        chunk_size: 100,
        media_mode: MediaMode::Copy,
        replace: true,
        media_src_dir: None,
        include_service_messages: true,
        include_deleted_messages: true,
        include_edit_history: true,
        build_search_index: true,
        build_date_index: true,
        target_peers: None,
    };
    let exporter = HtmlArchiveExporter::new(&db, opts);
    exporter.export().unwrap();

    let manifest_path = export_dir.join("manifest.json");
    let manifest_content = fs::read_to_string(&manifest_path).unwrap();

    // 1. Empty source fingerprint -> html_export must be corrupted
    let mut manifest_val: serde_json::Value = serde_json::from_str(&manifest_content).unwrap();
    manifest_val["source_fingerprint"]["source_digest"] = serde_json::json!("");
    fs::write(
        &manifest_path,
        serde_json::to_string(&manifest_val).unwrap(),
    )
    .unwrap();

    let rep_empty_src = VerificationEngine::new(VerificationOptions {
        archive_path: None,
        html_dir: Some(export_dir.clone()),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_empty_src.summary.dimensions.html_export.status,
        "corrupted"
    );
    assert!(
        rep_empty_src
            .findings
            .iter()
            .any(|f| f.code == "HTML_MANIFEST_EMPTY_SOURCE_FINGERPRINT")
    );

    // 2. Empty config fingerprint -> html_export must be corrupted
    manifest_val["source_fingerprint"]["source_digest"] = serde_json::json!("valid_digest_123");
    manifest_val["export_config_fingerprint"] = serde_json::json!("");
    fs::write(
        &manifest_path,
        serde_json::to_string(&manifest_val).unwrap(),
    )
    .unwrap();

    let rep_empty_cfg = VerificationEngine::new(VerificationOptions {
        archive_path: None,
        html_dir: Some(export_dir.clone()),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_empty_cfg.summary.dimensions.html_export.status,
        "corrupted"
    );
    assert!(
        rep_empty_cfg
            .findings
            .iter()
            .any(|f| f.code == "HTML_MANIFEST_EMPTY_CONFIG_FINGERPRINT")
    );

    // 3. Corrupted manifest JSON -> html_export must be corrupted
    fs::write(&manifest_path, "{broken_json").unwrap();

    let rep_corrupt = VerificationEngine::new(VerificationOptions {
        archive_path: None,
        html_dir: Some(export_dir.clone()),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_corrupt.summary.dimensions.html_export.status,
        "corrupted"
    );
    assert!(
        rep_corrupt
            .findings
            .iter()
            .any(|f| f.code == "HTML_MANIFEST_CORRUPTED")
    );

    // 4. Missing manifest -> html_export must be corrupted
    fs::remove_file(&manifest_path).unwrap();

    let rep_missing = VerificationEngine::new(VerificationOptions {
        archive_path: None,
        html_dir: Some(export_dir),
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    })
    .run()
    .unwrap();
    assert_eq!(
        rep_missing.summary.dimensions.html_export.status,
        "corrupted"
    );
    assert!(
        rep_missing
            .findings
            .iter()
            .any(|f| f.code == "HTML_MANIFEST_MISSING")
    );
}

#[test]
fn entity_auditor_supports_variant_tagged_and_flat_formats() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = setup_test_db(&db_path);

    // Insert messages with:
    // 1. Tagged TL entity format: [{"Mention": {"offset": 0, "length": 5}}]
    // 2. Multiple tagged entities: [{"Bold": {"offset": 0, "length": 4}}, {"Italic": {"offset": 5, "length": 6}}]
    // 3. Flat format: [{"offset": 0, "length": 4}]
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, entities_json)
             VALUES (100, 10, 100, 1700000001, 'active', 'Hello world', '[{\"Mention\":{\"offset\":0,\"length\":5}}]')",
            [],
        )?;
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, entities_json)
             VALUES (100, 11, 100, 1700000002, 'active', 'Bold italic text', '[{\"Bold\":{\"offset\":0,\"length\":4}},{\"Italic\":{\"offset\":5,\"length\":6}}]')",
            [],
        )?;
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, entities_json)
             VALUES (100, 12, 100, 1700000003, 'active', 'Flat format test', '[{\"offset\":0,\"length\":4}]')",
            [],
        )?;
        Ok(())
    }).unwrap();

    let opts = VerificationOptions {
        archive_path: Some(db_path),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(opts);
    let report = engine.run().unwrap();

    let entity_missing = report
        .findings
        .iter()
        .filter(|f| f.code == "ENTITY_MISSING_BOUNDS")
        .count();
    assert_eq!(
        entity_missing, 0,
        "Expected zero ENTITY_MISSING_BOUNDS findings for valid tagged & flat entities"
    );
}

#[test]
fn entity_auditor_detects_bounds_validation_errors() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = setup_test_db(&db_path);

    db.with_conn(|conn| {
        // Out of bounds: text length is 5, entity offset 0, length 10
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, entities_json)
             VALUES (100, 20, 100, 1700000001, 'active', 'Short', '[{\"Bold\":{\"offset\":0,\"length\":10}}]')",
            [],
        )?;
        // Negative bounds: offset -1
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, entities_json)
             VALUES (100, 21, 100, 1700000002, 'active', 'Negative', '[{\"Bold\":{\"offset\":-1,\"length\":5}}]')",
            [],
        )?;
        // Missing bounds: empty object
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, entities_json)
             VALUES (100, 22, 100, 1700000003, 'active', 'EmptyObj', '[{}]')",
            [],
        )?;
        Ok(())
    }).unwrap();

    let opts = VerificationOptions {
        archive_path: Some(db_path),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(opts);
    let report = engine.run().unwrap();

    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "ENTITY_UTF16_OUT_OF_BOUNDS")
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "ENTITY_NEGATIVE_BOUNDS")
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "ENTITY_MISSING_BOUNDS")
    );
}

#[test]
fn edit_history_provenance_audits_revisions_and_dates() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = setup_test_db(&db_path);

    db.with_conn(|conn| {
        // Historical message ingested with edit_date (no captured revisions) -> Warning
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, edit_date, text, entities_json)
             VALUES (100, 30, 100, 1700000001, 'edited', 1700000005, 'Edited historical message', '[]')",
            [],
        )?;
        // Incremental message with captured revision in message_revisions -> Clean
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, edit_date, text, entities_json)
             VALUES (100, 31, 100, 1700000002, 'edited', 1700000006, 'Current version', '[]')",
            [],
        )?;
        conn.execute(
            "INSERT INTO message_revisions (peer_id, message_id, revision_id, edit_date, text, entities_json, captured_at)
             VALUES (100, 31, 1, 1700000002, 'Initial version', '[]', 1700000006)",
            [],
        )?;
        Ok(())
    }).unwrap();

    let opts = VerificationOptions {
        archive_path: Some(db_path),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: false,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(opts);
    let report = engine.run().unwrap();

    let edit_warnings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == "EDITED_WITHOUT_REVISION_HISTORY")
        .collect();
    assert_eq!(edit_warnings.len(), 1);
    assert_eq!(edit_warnings[0].message_id, Some(30));
    assert_eq!(edit_warnings[0].severity, FindingSeverity::Warning);
}

#[test]
fn reply_graph_classifies_out_of_scope_peer_targets() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("archive.db");
    let db = setup_test_db(&db_path);

    db.with_conn(|conn| {
        // Message 40: In-scope reply to missing message in peer 100
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, entities_json, reply_to_msg_id, reply_to_peer_id)
             VALUES (100, 40, 100, 1700000001, 'active', 'In-scope reply', '[]', 999, 100)",
            [],
        )?;

        // Message 41: Cross-peer reply to an external unexported peer 200
        conn.execute(
            "INSERT INTO messages (peer_id, message_id, sender_id, date, state, text, entities_json, reply_to_msg_id, reply_to_peer_id)
             VALUES (100, 41, 100, 1700000002, 'active', 'Cross-peer reply', '[]', 555, 200)",
            [],
        )?;
        Ok(())
    }).unwrap();

    let opts = VerificationOptions {
        archive_path: Some(db_path),
        html_dir: None,
        media_dir: None,
        mode: VerificationMode::Full,
        scope_media: false,
        scope_replies: true,
        scope_search: false,
        rehash_media: false,
        strict: false,
    };
    let engine = VerificationEngine::new(opts);
    let report = engine.run().unwrap();

    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "REPLY_TARGET_MISSING" && f.message_id == Some(40))
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "REPLY_TARGET_OUT_OF_SCOPE" && f.message_id == Some(41))
    );
}
