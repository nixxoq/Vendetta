use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
};

use grammers_tl_types::{self as tl, Deserializable};
use sha2::{Digest, Sha256};
use vendetta_core::now_unix_secs;
use vendetta_storage::CURRENT_SCHEMA_VERSION;

use crate::{
    auditors::{DatabaseAuditContext, fs::audit_relative_path},
    error::VerificationResult,
    model::{FindingCategory, FindingSeverity, MediaAuditMetrics, VerificationFinding},
};

pub fn audit_schema(ctx: &DatabaseAuditContext) -> VerificationResult<Vec<VerificationFinding>> {
    let mut findings = Vec::new();
    let conn = ctx.conn;

    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='schema_migrations'",
    )?;
    let has_migrations = stmt.exists([])?;
    if !has_migrations {
        findings.push(VerificationFinding {
            code: "SCHEMA_MIGRATIONS_TABLE_MISSING".to_string(),
            severity: FindingSeverity::Fatal,
            category: FindingCategory::Schema,
            peer_id: None,
            message_id: None,
            media_id: None,
            path: None,
            description: "The schema_migrations table is missing from the database.".to_string(),
            evidence: None,
            recommendation: Some(
                "Ensure the database was initialized with Vendetta schema migrations.".to_string(),
            ),
        });
        return Ok(findings);
    }

    let latest_version: Option<i64> = conn
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap_or(None);

    match latest_version {
        Some(v) if v < CURRENT_SCHEMA_VERSION => {
            findings.push(VerificationFinding {
                code: "SCHEMA_VERSION_OUTDATED".to_string(),
                severity: FindingSeverity::Error,
                category: FindingCategory::Schema,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: None,
                description: format!(
                    "Schema migration version is {v}, expected at least {CURRENT_SCHEMA_VERSION}."
                ),
                evidence: Some(serde_json::json!({
                    "current_version": v,
                    "expected_min_version": CURRENT_SCHEMA_VERSION
                })),
                recommendation: Some(
                    "Apply pending schema migrations to bring archive up to date.".to_string(),
                ),
            });
        }
        None => {
            findings.push(VerificationFinding {
                code: "SCHEMA_VERSION_EMPTY".to_string(),
                severity: FindingSeverity::Fatal,
                category: FindingCategory::Schema,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: None,
                description: "No schema migrations have been recorded in schema_migrations table."
                    .to_string(),
                evidence: None,
                recommendation: Some("Initialize database schema.".to_string()),
            });
        }
        _ => {}
    }

    let fk_enabled: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap_or(0);
    if fk_enabled == 0 {
        findings.push(VerificationFinding {
            code: "DB_FOREIGN_KEYS_DISABLED".to_string(),
            severity: FindingSeverity::Info,
            category: FindingCategory::Schema,
            peer_id: None,
            message_id: None,
            media_id: None,
            path: None,
            description:
                "SQLite foreign key constraint enforcement is disabled in the current session."
                    .to_string(),
            evidence: Some(serde_json::json!({ "foreign_keys_pragma": 0 })),
            recommendation: Some(
                "Ensure active write connections execute PRAGMA foreign_keys = ON.".to_string(),
            ),
        });
    }

    let required_tables = [
        "peers",
        "messages",
        "message_revisions",
        "message_media",
        "media_objects",
        "account_sync_state",
        "sync_state",
        "channel_sync_queue",
        "unsupported_events",
    ];

    let mut existing_tables = HashSet::new();
    let mut table_stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
    let table_rows = table_stmt.query_map([], |row| row.get::<_, String>(0))?;
    for name in table_rows.flatten() {
        existing_tables.insert(name);
    }

    for table in required_tables {
        if !existing_tables.contains(table) {
            findings.push(VerificationFinding {
                code: "REQUIRED_TABLE_MISSING".to_string(),
                severity: FindingSeverity::Fatal,
                category: FindingCategory::Schema,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: None,
                description: format!("Required table '{table}' is missing from database schema."),
                evidence: Some(serde_json::json!({ "table": table })),
                recommendation: Some(format!("Run migrations to create '{table}' table.")),
            });
        }
    }

    let mut fk_stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let fk_rows = fk_stmt.query_map([], |row| {
        let table: String = row.get(0)?;
        let rowid: i64 = row.get(1)?;
        let parent_table: String = row.get(2)?;
        let fk_idx: i64 = row.get(3)?;
        Ok((table, rowid, parent_table, fk_idx))
    })?;

    for (table, rowid, parent, fk_idx) in fk_rows.flatten() {
        findings.push(VerificationFinding {
            code: "DB_FOREIGN_KEY_VIOLATION".to_string(),
            severity: FindingSeverity::Error,
            category: FindingCategory::ReferentialIntegrity,
            peer_id: None,
            message_id: None,
            media_id: None,
            path: None,
            description: format!(
                "Foreign key constraint violation in table '{table}' at rowid {rowid} referencing '{parent}' (fkid {fk_idx})."
            ),
            evidence: Some(serde_json::json!({
                "table": table,
                "rowid": rowid,
                "parent_table": parent,
                "fk_index": fk_idx
            })),
            recommendation: Some(
                "Repair broken relation or purge orphaned child record.".to_string(),
            ),
        });
    }

    let mut check_stmt = conn.prepare("PRAGMA quick_check")?;
    let check_results = check_stmt.query_map([], |row| row.get::<_, String>(0))?;
    for msg in check_results.flatten() {
        if msg != "ok" {
            findings.push(VerificationFinding {
                code: "SQLITE_CORRUPTION_DETECTED".to_string(),
                severity: FindingSeverity::Fatal,
                category: FindingCategory::Schema,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: None,
                description: format!("SQLite quick_check reported corruption: {msg}"),
                evidence: Some(serde_json::json!({ "error": msg })),
                recommendation: Some(
                    "Recover SQLite database using backup or .dump utility.".to_string(),
                ),
            });
        }
    }

    let critical_indexes = [
        "idx_messages_peer_date",
        "idx_messages_grouped_id",
        "idx_messages_sender",
        "idx_media_objects_sha256",
    ];

    let mut existing_indexes = HashSet::new();
    let mut idx_stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='index'")?;
    let idx_rows = idx_stmt.query_map([], |row| row.get::<_, String>(0))?;
    for name in idx_rows.flatten() {
        existing_indexes.insert(name);
    }

    for idx_name in critical_indexes {
        if !existing_indexes.contains(idx_name) {
            findings.push(VerificationFinding {
                code: "CRITICAL_INDEX_MISSING".to_string(),
                severity: FindingSeverity::Warning,
                category: FindingCategory::Schema,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: None,
                description: format!("Recommended index '{idx_name}' is missing from schema."),
                evidence: Some(serde_json::json!({ "index": idx_name })),
                recommendation: Some(format!(
                    "Create index '{idx_name}' to maintain query performance."
                )),
            });
        }
    }

    Ok(findings)
}

pub fn audit_identity(ctx: &DatabaseAuditContext) -> VerificationResult<Vec<VerificationFinding>> {
    let mut findings = Vec::new();
    let conn = ctx.conn;

    let mut invalid_msg_id_stmt =
        conn.prepare("SELECT peer_id, message_id FROM messages WHERE message_id <= 0 LIMIT 100")?;
    let rows = invalid_msg_id_stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let mid: i64 = row.get(1)?;
        Ok((pid, mid))
    })?;

    for (pid, mid) in rows.flatten() {
        findings.push(VerificationFinding {
            code: "MESSAGE_ID_INVALID_RANGE".to_string(),
            severity: FindingSeverity::Error,
            category: FindingCategory::Identity,
            peer_id: Some(pid),
            message_id: Some(mid),
            media_id: None,
            path: None,
            description: format!("Message has non-positive message_id: {mid}."),
            evidence: Some(serde_json::json!({ "peer_id": pid, "message_id": mid })),
            recommendation: Some(
                "Ensure message_id adheres to Telegram protocol (> 0).".to_string(),
            ),
        });
    }

    let mut zero_peer_stmt =
        conn.prepare("SELECT peer_id, message_id FROM messages WHERE peer_id = 0 LIMIT 100")?;
    let zero_peer_rows = zero_peer_stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let mid: i64 = row.get(1)?;
        Ok((pid, mid))
    })?;

    for (pid, mid) in zero_peer_rows.flatten() {
        findings.push(VerificationFinding {
            code: "PEER_ID_ZERO".to_string(),
            severity: FindingSeverity::Error,
            category: FindingCategory::Identity,
            peer_id: Some(pid),
            message_id: Some(mid),
            media_id: None,
            path: None,
            description: "Message record contains invalid peer_id = 0.".to_string(),
            evidence: Some(serde_json::json!({ "message_id": mid })),
            recommendation: Some(
                "Reconcile peer identity with canonical Telegram peer ID.".to_string(),
            ),
        });
    }

    let mut missing_peer_stmt = conn.prepare(
        "SELECT DISTINCT m.peer_id FROM messages m LEFT JOIN peers p ON m.peer_id = p.peer_id WHERE p.peer_id IS NULL",
    )?;
    let missing_peer_rows = missing_peer_stmt.query_map([], |row| row.get::<_, i64>(0))?;
    for pid in missing_peer_rows.flatten() {
        findings.push(VerificationFinding {
            code: "MESSAGE_WITHOUT_PEER_RECORD".to_string(),
            severity: FindingSeverity::Error,
            category: FindingCategory::ReferentialIntegrity,
            peer_id: Some(pid),
            message_id: None,
            media_id: None,
            path: None,
            description: format!("Messages exist for peer_id {pid} but no corresponding record exists in peers table."),
            evidence: Some(serde_json::json!({ "peer_id": pid })),
            recommendation: Some("Backfill peer metadata from Telegram API or channel discovery.".to_string()),
        });
    }

    let mut peer_type_stmt = conn.prepare(
        "SELECT peer_id, peer_type FROM peers WHERE peer_type NOT IN ('user', 'chat', 'channel', 'group')",
    )?;
    let peer_type_rows = peer_type_stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let pt: String = row.get(1)?;
        Ok((pid, pt))
    })?;

    for (pid, pt) in peer_type_rows.flatten() {
        findings.push(VerificationFinding {
            code: "UNKNOWN_PEER_TYPE".to_string(),
            severity: FindingSeverity::Warning,
            category: FindingCategory::Identity,
            peer_id: Some(pid),
            message_id: None,
            media_id: None,
            path: None,
            description: format!("Peer {pid} has unknown peer_type: '{pt}'."),
            evidence: Some(serde_json::json!({ "peer_id": pid, "peer_type": pt })),
            recommendation: Some(
                "Normalize peer_type to 'user', 'chat', or 'channel'.".to_string(),
            ),
        });
    }

    Ok(findings)
}

pub fn audit_chronology(
    ctx: &DatabaseAuditContext,
) -> VerificationResult<Vec<VerificationFinding>> {
    let mut findings = Vec::new();
    let conn = ctx.conn;

    let min_ts: i64 = 1375315200;
    let max_ts: i64 = 2524608000;

    let mut ts_stmt = conn.prepare(
        "SELECT peer_id, message_id, date FROM messages WHERE date < ?1 OR date > ?2 LIMIT 100",
    )?;
    let ts_rows = ts_stmt.query_map([min_ts, max_ts], |row| {
        let pid: i64 = row.get(0)?;
        let mid: i64 = row.get(1)?;
        let date: i64 = row.get(2)?;
        Ok((pid, mid, date))
    })?;

    for (pid, mid, date) in ts_rows.flatten() {
        findings.push(VerificationFinding {
            code: "MESSAGE_DATE_IMPOSSIBLE".to_string(),
            severity: FindingSeverity::Error,
            category: FindingCategory::Chronology,
            peer_id: Some(pid),
            message_id: Some(mid),
            media_id: None,
            path: None,
            description: format!(
                "Message has impossible timestamp ({date}) outside 2013-2050 range."
            ),
            evidence: Some(serde_json::json!({
                "peer_id": pid,
                "message_id": mid,
                "date": date
            })),
            recommendation: Some("Verify raw TL message date timestamp.".to_string()),
        });
    }

    if !ctx.is_fast_mode {
        let mut inv_stmt = conn.prepare(
            "SELECT m1.peer_id, m1.message_id, m1.date, m2.message_id, m2.date
             FROM messages m1
             JOIN messages m2 ON m1.peer_id = m2.peer_id AND m2.message_id > m1.message_id
             JOIN peers p ON p.peer_id = m1.peer_id AND (p.peer_type = 'channel' OR p.peer_type = 'group')
             WHERE m2.date < (m1.date - 86400)
             LIMIT 50",
        )?;

        let inv_rows = inv_stmt.query_map([], |row| {
            let pid: i64 = row.get(0)?;
            let m1_id: i64 = row.get(1)?;
            let m1_date: i64 = row.get(2)?;
            let m2_id: i64 = row.get(3)?;
            let m2_date: i64 = row.get(4)?;
            Ok((pid, m1_id, m1_date, m2_id, m2_date))
        })?;

        for (pid, m1_id, m1_date, m2_id, m2_date) in inv_rows.flatten() {
            findings.push(VerificationFinding {
                code: "CHRONOLOGY_SEVERE_INVERSION".to_string(),
                severity: FindingSeverity::Warning,
                category: FindingCategory::Chronology,
                peer_id: Some(pid),
                message_id: Some(m2_id),
                media_id: None,
                path: None,
                description: format!(
                    "Severe channel timestamp inversion: message {m2_id} (date {m2_date}) is earlier than older message {m1_id} (date {m1_date})."
                ),
                evidence: Some(serde_json::json!({
                    "peer_id": pid,
                    "earlier_msg_id": m1_id,
                    "earlier_date": m1_date,
                    "later_msg_id": m2_id,
                    "later_date": m2_date
                })),
                recommendation: Some("Check channel history import or scheduled post timestamps.".to_string()),
            });
        }
    }

    Ok(findings)
}

pub fn audit_message_states(
    ctx: &DatabaseAuditContext,
) -> VerificationResult<Vec<VerificationFinding>> {
    let mut findings = Vec::new();
    let conn = ctx.conn;

    let mut state_stmt = conn.prepare(
        "SELECT peer_id, message_id, state FROM messages WHERE state NOT IN ('active', 'edited', 'deleted', 'empty', 'inaccessible')",
    )?;
    let state_rows = state_stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let mid: i64 = row.get(1)?;
        let state: String = row.get(2)?;
        Ok((pid, mid, state))
    })?;

    for (pid, mid, state) in state_rows.flatten() {
        findings.push(VerificationFinding {
            code: "MESSAGE_STATE_INVALID".to_string(),
            severity: FindingSeverity::Error,
            category: FindingCategory::MessageState,
            peer_id: Some(pid),
            message_id: Some(mid),
            media_id: None,
            path: None,
            description: format!("Message has invalid state value: '{state}'."),
            evidence: Some(serde_json::json!({ "state": state })),
            recommendation: Some("Reconcile message state with valid enum variants.".to_string()),
        });
    }

    let mut edited_stmt = conn.prepare(
        "SELECT m.peer_id, m.message_id
         FROM messages m
         LEFT JOIN message_revisions r ON m.peer_id = r.peer_id AND m.message_id = r.message_id
         WHERE m.state = 'edited' AND r.revision_id IS NULL",
    )?;
    let edited_rows = edited_stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let mid: i64 = row.get(1)?;
        Ok((pid, mid))
    })?;

    for (pid, mid) in edited_rows.flatten() {
        findings.push(VerificationFinding {
            code: "EDITED_WITHOUT_REVISION_HISTORY".to_string(),
            severity: FindingSeverity::Warning,
            category: FindingCategory::MessageState,
            peer_id: Some(pid),
            message_id: Some(mid),
            media_id: None,
            path: None,
            description: format!("Message ({pid}, {mid}) is marked as edited, but has no captured revisions in message_revisions (historical edit prior to archive ingestion)."),
            evidence: Some(serde_json::json!({
                "peer_id": pid,
                "message_id": mid,
                "provenance": "historical_ingest_prior_to_archive"
            })),
            recommendation: Some("Normal if exporter observed the message only after it was already edited on Telegram servers.".to_string()),
        });
    }

    let mut empty_stmt = conn.prepare(
        "SELECT peer_id, message_id, state, raw_tl FROM messages WHERE state = 'empty' AND raw_tl IS NOT NULL",
    )?;
    let empty_rows = empty_stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let mid: i64 = row.get(1)?;
        let state: String = row.get(2)?;
        let raw_tl: Vec<u8> = row.get(3)?;
        Ok((pid, mid, state, raw_tl))
    })?;

    for (pid, mid, state, raw_tl) in empty_rows.flatten() {
        if !raw_tl.is_empty() {
            findings.push(VerificationFinding {
                code: "MESSAGE_EMPTY_CONFLATION".to_string(),
                severity: FindingSeverity::Info,
                category: FindingCategory::MessageState,
                peer_id: Some(pid),
                message_id: Some(mid),
                media_id: None,
                path: None,
                description: format!(
                    "Message ({pid}, {mid}) is marked state '{state}' with raw TL payload."
                ),
                evidence: Some(serde_json::json!({ "raw_tl_bytes": raw_tl.len() })),
                recommendation: None,
            });
        }
    }

    Ok(findings)
}

pub fn audit_revisions(ctx: &DatabaseAuditContext) -> VerificationResult<Vec<VerificationFinding>> {
    let mut findings = Vec::new();
    let conn = ctx.conn;

    let mut rev_stmt = conn.prepare(
        "SELECT peer_id, message_id, revision_id, edit_date, text, entities_json
         FROM message_revisions
         ORDER BY peer_id ASC, message_id ASC, revision_id ASC",
    )?;

    let rev_rows = rev_stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let mid: i64 = row.get(1)?;
        let rev_id: i64 = row.get(2)?;
        let edit_date: Option<i64> = row.get(3)?;
        let text: Option<String> = row.get(4)?;
        let entities_json: Option<String> = row.get(5)?;
        Ok((pid, mid, rev_id, edit_date, text, entities_json))
    })?;

    let mut current_key: Option<(i64, i64)> = None;
    let mut last_rev_id: i64 = 0;
    let mut last_edit_date: Option<i64> = None;

    for (pid, mid, rev_id, edit_date, _text, entities_json) in rev_rows.flatten() {
        let key = (pid, mid);
        if current_key != Some(key) {
            current_key = Some(key);
            last_rev_id = rev_id;
            last_edit_date = edit_date;
        } else {
            if rev_id <= last_rev_id {
                findings.push(VerificationFinding {
                    code: "REVISION_ORDER_INVALID".to_string(),
                    severity: FindingSeverity::Error,
                    category: FindingCategory::Revision,
                    peer_id: Some(pid),
                    message_id: Some(mid),
                    media_id: None,
                    path: None,
                    description: format!("Non-monotonic revision_id sequence ({last_rev_id} -> {rev_id}) for message ({pid}, {mid})."),
                    evidence: Some(serde_json::json!({
                        "peer_id": pid,
                        "message_id": mid,
                        "previous_rev_id": last_rev_id,
                        "current_rev_id": rev_id
                    })),
                    recommendation: Some("Ensure revision_id increments monotonically.".to_string()),
                });
            }

            if let (Some(l_date), Some(c_date)) = (last_edit_date, edit_date)
                && c_date < l_date {
                    findings.push(VerificationFinding {
                        code: "REVISION_ORDER_INVALID".to_string(),
                        severity: FindingSeverity::Error,
                        category: FindingCategory::Revision,
                        peer_id: Some(pid),
                        message_id: Some(mid),
                        media_id: None,
                        path: None,
                        description: format!("Revision edit_date inversion ({l_date} -> {c_date}) for message ({pid}, {mid})."),
                        evidence: Some(serde_json::json!({
                            "peer_id": pid,
                            "message_id": mid,
                            "previous_edit_date": l_date,
                            "current_edit_date": c_date
                        })),
                        recommendation: Some("Verify edit_date timestamps in message revision history.".to_string()),
                    });
                }

            last_rev_id = rev_id;
            last_edit_date = edit_date;
        }

        if let Some(json_str) = entities_json
            .as_deref()
            .filter(|s| !s.is_empty() && *s != "[]")
            && let Err(e) = serde_json::from_str::<serde_json::Value>(json_str) {
                findings.push(VerificationFinding {
                    code: "ENTITIES_JSON_MALFORMED".to_string(),
                    severity: FindingSeverity::Error,
                    category: FindingCategory::Entity,
                    peer_id: Some(pid),
                    message_id: Some(mid),
                    media_id: None,
                    path: None,
                    description: format!(
                        "Malformed entities_json in revision {rev_id} of ({pid}, {mid}): {e}"
                    ),
                    evidence: Some(serde_json::json!({
                        "peer_id": pid,
                        "message_id": mid,
                        "revision_id": rev_id,
                        "raw_json": json_str
                    })),
                    recommendation: Some("Sanitize JSON syntax in message_revisions.".to_string()),
                });
            }
    }

    Ok(findings)
}

pub fn audit_entities(ctx: &DatabaseAuditContext) -> VerificationResult<Vec<VerificationFinding>> {
    let mut findings = Vec::new();
    let conn = ctx.conn;

    let mut msg_stmt = conn.prepare(
        "SELECT peer_id, message_id, text, entities_json
         FROM messages
         WHERE entities_json IS NOT NULL AND entities_json != '' AND entities_json != '[]'",
    )?;

    let rows = msg_stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let mid: i64 = row.get(1)?;
        let text: Option<String> = row.get(2)?;
        let json_str: String = row.get(3)?;
        Ok((pid, mid, text, json_str))
    })?;

    for (pid, mid, text_opt, json_str) in rows.flatten() {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json_str);
        match parsed {
            Ok(serde_json::Value::Array(arr)) => {
                let text = text_opt.unwrap_or_default();
                let utf16_len = text.encode_utf16().count();

                for (idx, item) in arr.iter().enumerate() {
                    let (offset, length) = extract_bounds(item);

                    match (offset, length) {
                        (Some(off), Some(len)) => {
                            if off < 0 || len < 0 {
                                findings.push(VerificationFinding {
                                    code: "ENTITY_NEGATIVE_BOUNDS".to_string(),
                                    severity: FindingSeverity::Error,
                                    category: FindingCategory::Entity,
                                    peer_id: Some(pid),
                                    message_id: Some(mid),
                                    media_id: None,
                                    path: None,
                                    description: format!(
                                        "Negative entity offset ({off}) or length ({len}) in message ({pid}, {mid}) at index {idx}."
                                    ),
                                    evidence: Some(serde_json::json!({
                                        "entity_index": idx,
                                        "offset": off,
                                        "length": len
                                    })),
                                    recommendation: Some("Normalize entity bounds to non-negative integers.".to_string()),
                                });
                            } else {
                                let end_offset = (off as usize).saturating_add(len as usize);
                                if end_offset > utf16_len {
                                    findings.push(VerificationFinding {
                                        code: "ENTITY_UTF16_OUT_OF_BOUNDS".to_string(),
                                        severity: FindingSeverity::Error,
                                        category: FindingCategory::Entity,
                                        peer_id: Some(pid),
                                        message_id: Some(mid),
                                        media_id: None,
                                        path: None,
                                        description: format!(
                                            "Entity offset+length ({off}+{len}={end_offset}) exceeds UTF-16 code unit text length ({utf16_len}) in message ({pid}, {mid})."
                                        ),
                                        evidence: Some(serde_json::json!({
                                            "entity_index": idx,
                                            "offset": off,
                                            "length": len,
                                            "total_utf16_len": utf16_len
                                        })),
                                        recommendation: Some("Clamp entity offsets to text length.".to_string()),
                                    });
                                }
                            }
                        }
                        _ => {
                            findings.push(VerificationFinding {
                                code: "ENTITY_MISSING_BOUNDS".to_string(),
                                severity: FindingSeverity::Error,
                                category: FindingCategory::Entity,
                                peer_id: Some(pid),
                                message_id: Some(mid),
                                media_id: None,
                                path: None,
                                description: format!("Entity at index {idx} in message ({pid}, {mid}) lacks offset or length fields."),
                                evidence: Some(serde_json::json!({ "entity_item": item })),
                                recommendation: Some("Ensure all entity objects contain integer offset and length fields.".to_string()),
                            });
                        }
                    }
                }
            }
            Ok(_) => {
                findings.push(VerificationFinding {
                    code: "ENTITIES_NOT_ARRAY".to_string(),
                    severity: FindingSeverity::Error,
                    category: FindingCategory::Entity,
                    peer_id: Some(pid),
                    message_id: Some(mid),
                    media_id: None,
                    path: None,
                    description: format!(
                        "entities_json for message ({pid}, {mid}) is not a JSON array."
                    ),
                    evidence: Some(serde_json::json!({ "raw_json": json_str })),
                    recommendation: Some(
                        "Ensure entities_json serializes as a JSON array.".to_string(),
                    ),
                });
            }
            Err(e) => {
                findings.push(VerificationFinding {
                    code: "ENTITIES_JSON_MALFORMED".to_string(),
                    severity: FindingSeverity::Error,
                    category: FindingCategory::Entity,
                    peer_id: Some(pid),
                    message_id: Some(mid),
                    media_id: None,
                    path: None,
                    description: format!("Malformed entities_json for message ({pid}, {mid}): {e}"),
                    evidence: Some(serde_json::json!({ "raw_json": json_str })),
                    recommendation: Some("Sanitize JSON formatting in messages table.".to_string()),
                });
            }
        }
    }

    Ok(findings)
}

fn extract_bounds(item: &serde_json::Value) -> (Option<i64>, Option<i64>) {
    if let Some(obj) = item.as_object() {
        if let (Some(off), Some(len)) = (
            obj.get("offset").and_then(|v| v.as_i64()),
            obj.get("length").and_then(|v| v.as_i64()),
        ) {
            return (Some(off), Some(len));
        }
        for val in obj.values() {
            if let Some(inner_obj) = val.as_object()
                && let (Some(off), Some(len)) = (
                    inner_obj.get("offset").and_then(|v| v.as_i64()),
                    inner_obj.get("length").and_then(|v| v.as_i64()),
                ) {
                    return (Some(off), Some(len));
                }
        }
    }
    (None, None)
}

pub fn audit_service_messages(
    ctx: &DatabaseAuditContext,
) -> VerificationResult<Vec<VerificationFinding>> {
    let mut findings = Vec::new();
    let conn = ctx.conn;

    let mut stmt =
        conn.prepare("SELECT peer_id, message_id, raw_tl FROM messages WHERE raw_tl IS NOT NULL")?;

    let rows = stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let mid: i64 = row.get(1)?;
        let raw_tl: Vec<u8> = row.get(2)?;
        Ok((pid, mid, raw_tl))
    })?;

    for (pid, mid, raw_tl) in rows.flatten() {
        if raw_tl.len() < 4 {
            findings.push(VerificationFinding {
                code: "TL_MALFORMED_BYTES".to_string(),
                severity: FindingSeverity::Error,
                category: FindingCategory::ServiceMessage,
                peer_id: Some(pid),
                message_id: Some(mid),
                media_id: None,
                path: None,
                description: format!(
                    "raw_tl payload for message ({pid}, {mid}) has fewer than 4 bytes ({})",
                    raw_tl.len()
                ),
                evidence: Some(serde_json::json!({ "byte_length": raw_tl.len() })),
                recommendation: Some(
                    "Verify MTProto message serialization in ingestion pipeline.".to_string(),
                ),
            });
            continue;
        }

        match tl::enums::Message::from_bytes(&raw_tl) {
            Ok(_) => {}
            Err(e) => {
                let constructor_id =
                    u32::from_le_bytes([raw_tl[0], raw_tl[1], raw_tl[2], raw_tl[3]]);
                let is_known = matches!(
                    constructor_id,
                    0x7614535      // messageEmpty#7614535
                    | 0x38114ee1   // message#38114ee1
                    | 0x2b085862 // messageService#2b085862
                );

                if !is_known {
                    findings.push(VerificationFinding {
                        code: "TL_UNKNOWN_CONSTRUCTOR".to_string(),
                        severity: FindingSeverity::Warning,
                        category: FindingCategory::ServiceMessage,
                        peer_id: Some(pid),
                        message_id: Some(mid),
                        media_id: None,
                        path: None,
                        description: format!(
                            "Unknown Telegram TL constructor ID: 0x{constructor_id:08x} for message ({pid}, {mid})."
                        ),
                        evidence: Some(serde_json::json!({
                            "constructor_id_hex": format!("0x{constructor_id:08x}"),
                            "raw_bytes_len": raw_tl.len()
                        })),
                        recommendation: Some("Update Telegram layer schema bindings when available.".to_string()),
                    });
                } else {
                    findings.push(VerificationFinding {
                        code: "TL_MALFORMED_BYTES".to_string(),
                        severity: FindingSeverity::Error,
                        category: FindingCategory::ServiceMessage,
                        peer_id: Some(pid),
                        message_id: Some(mid),
                        media_id: None,
                        path: None,
                        description: format!(
                            "Failed to deserialize known TL constructor 0x{constructor_id:08x} for message ({pid}, {mid}): {e}"
                        ),
                        evidence: Some(serde_json::json!({
                            "constructor_id_hex": format!("0x{constructor_id:08x}"),
                            "error": e.to_string()
                        })),
                        recommendation: Some("Check MTProto frame integrity and payload decoding.".to_string()),
                    });
                }
            }
        }
    }

    Ok(findings)
}

pub fn audit_sync_state(
    ctx: &DatabaseAuditContext,
) -> VerificationResult<Vec<VerificationFinding>> {
    let mut findings = Vec::new();
    let conn = ctx.conn;

    let mut acc_stmt = conn.prepare(
        "SELECT account_id, pts, qts, date, seq, sync_uncertain, last_synced_at FROM account_sync_state LIMIT 1",
    )?;
    let acc_rows = acc_stmt.query_map([], |row| {
        let acc_id: String = row.get(0)?;
        let pts: i64 = row.get(1)?;
        let qts: i64 = row.get(2)?;
        let date: i64 = row.get(3)?;
        let seq: i64 = row.get(4)?;
        let sync_uncertain: i64 = row.get(5)?;
        let last_synced: i64 = row.get(6)?;
        Ok((acc_id, pts, qts, date, seq, sync_uncertain > 0, last_synced))
    })?;

    let mut has_account_sync = false;
    for (acc_id, pts, qts, date, seq, uncertain, last_synced) in acc_rows.flatten() {
        has_account_sync = true;

        if uncertain {
            findings.push(VerificationFinding {
                code: "SYNC_UNCERTAIN".to_string(),
                severity: FindingSeverity::Warning,
                category: FindingCategory::SyncState,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: None,
                description: format!("AccountSyncState '{acc_id}' is marked as sync_uncertain (gap in update stream detected)."),
                evidence: Some(serde_json::json!({
                    "account_id": acc_id,
                    "pts": pts,
                    "qts": qts,
                    "date": date,
                    "seq": seq,
                    "last_synced_at": last_synced
                })),
                recommendation: Some("Trigger Telegram difference update sync to reconcile gaps.".to_string()),
            });
        }

        if pts < 0 || qts < 0 || seq < 0 {
            findings.push(VerificationFinding {
                code: "SYNC_STATE_NEGATIVE_SEQUENCE".to_string(),
                severity: FindingSeverity::Error,
                category: FindingCategory::SyncState,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: None,
                description: format!("AccountSyncState contains negative sequence values: pts={pts}, qts={qts}, seq={seq}."),
                evidence: Some(serde_json::json!({ "pts": pts, "qts": qts, "seq": seq })),
                recommendation: Some("Re-fetch state baseline from Telegram getDifference.".to_string()),
            });
        }
    }

    if !has_account_sync {
        findings.push(VerificationFinding {
            code: "SYNC_STATE_EMPTY".to_string(),
            severity: FindingSeverity::Info,
            category: FindingCategory::SyncState,
            peer_id: None,
            message_id: None,
            media_id: None,
            path: None,
            description: "No account sync state recorded in account_sync_state.".to_string(),
            evidence: None,
            recommendation: None,
        });
    }

    let mut peer_stmt = conn.prepare(
        "SELECT peer_id, pts, qts, sync_uncertain FROM sync_state WHERE sync_uncertain > 0",
    )?;
    let peer_rows = peer_stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let pts: i64 = row.get(1)?;
        let qts: i64 = row.get(2)?;
        Ok((pid, pts, qts))
    })?;

    for (pid, pts, qts) in peer_rows.flatten() {
        findings.push(VerificationFinding {
            code: "PEER_SYNC_UNCERTAIN".to_string(),
            severity: FindingSeverity::Warning,
            category: FindingCategory::SyncState,
            peer_id: Some(pid),
            message_id: None,
            media_id: None,
            path: None,
            description: format!("Peer {pid} sync state is marked as sync_uncertain."),
            evidence: Some(serde_json::json!({ "peer_id": pid, "pts": pts, "qts": qts })),
            recommendation: Some("Trigger channel difference sync for this peer.".to_string()),
        });
    }

    Ok(findings)
}

pub fn audit_unsupported_events(
    ctx: &DatabaseAuditContext,
) -> VerificationResult<Vec<VerificationFinding>> {
    let mut findings = Vec::new();
    let conn = ctx.conn;

    let mut stmt = conn.prepare(
        "SELECT event_id, constructor_id, affects_sync_state, peer_id, pts, pts_count, raw_tl FROM unsupported_events",
    )?;

    let rows = stmt.query_map([], |row| {
        let event_id: i64 = row.get(0)?;
        let constructor_id: i64 = row.get(1)?;
        let affects_state: i64 = row.get(2)?;
        let peer_id: Option<i64> = row.get(3)?;
        let pts: Option<i64> = row.get(4)?;
        let pts_count: Option<i64> = row.get(5)?;
        let raw_tl: Vec<u8> = row.get(6)?;
        Ok((
            event_id,
            constructor_id,
            affects_state > 0,
            peer_id,
            pts,
            pts_count,
            raw_tl,
        ))
    })?;

    for (event_id, constructor_id, affects_state, peer_id, pts, pts_count, raw_tl) in rows.flatten()
    {
        if affects_state {
            findings.push(VerificationFinding {
                code: "UNSUPPORTED_STATE_AFFECTING_UPDATE".to_string(),
                severity: FindingSeverity::Warning,
                category: FindingCategory::UnsupportedUpdate,
                peer_id,
                message_id: None,
                media_id: None,
                path: None,
                description: format!(
                    "Unsupported Telegram update (0x{constructor_id:08x}) was marked as affects_sync_state. Full state sync may be required."
                ),
                evidence: Some(serde_json::json!({
                    "event_id": event_id,
                    "constructor_id_hex": format!("0x{constructor_id:08x}"),
                    "pts": pts,
                    "pts_count": pts_count,
                    "raw_bytes_len": raw_tl.len()
                })),
                recommendation: Some("Trigger full channel or account resync to resolve potential state divergence.".to_string()),
            });
        } else {
            findings.push(VerificationFinding {
                code: "UNSUPPORTED_UPDATE_RECORDED".to_string(),
                severity: FindingSeverity::Info,
                category: FindingCategory::UnsupportedUpdate,
                peer_id,
                message_id: None,
                media_id: None,
                path: None,
                description: format!(
                    "Unsupported Telegram update (0x{constructor_id:08x}) preserved in unsupported_events."
                ),
                evidence: Some(serde_json::json!({
                    "event_id": event_id,
                    "constructor_id_hex": format!("0x{constructor_id:08x}"),
                    "raw_bytes_len": raw_tl.len()
                })),
                recommendation: None,
            });
        }
    }

    Ok(findings)
}

pub fn audit_channel_queue(
    ctx: &DatabaseAuditContext,
) -> VerificationResult<Vec<VerificationFinding>> {
    let mut findings = Vec::new();
    let conn = ctx.conn;

    let mut stmt = conn.prepare(
        "SELECT peer_id, discovered_pts, current_pts, status, attempts, last_error, updated_at
         FROM channel_sync_queue",
    )?;

    let rows = stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let disc_pts: i64 = row.get(1)?;
        let cur_pts: Option<i64> = row.get(2)?;
        let status: String = row.get(3)?;
        let attempts: i64 = row.get(4)?;
        let last_error: Option<String> = row.get(5)?;
        let updated_at: i64 = row.get(6)?;
        Ok((
            pid, disc_pts, cur_pts, status, attempts, last_error, updated_at,
        ))
    })?;

    for (pid, disc_pts, cur_pts, status, attempts, last_error, updated_at) in rows.flatten() {
        match status.as_str() {
            "failed" | "blocked" => {
                findings.push(VerificationFinding {
                    code: "QUEUE_CHANNEL_BLOCKED".to_string(),
                    severity: FindingSeverity::Warning,
                    category: FindingCategory::Queue,
                    peer_id: Some(pid),
                    message_id: None,
                    media_id: None,
                    path: None,
                    description: format!(
                        "Channel {pid} sync queue entry is '{status}' after {attempts} attempts. Error: {}",
                        last_error.as_deref().unwrap_or("none")
                    ),
                    evidence: Some(serde_json::json!({
                        "peer_id": pid,
                        "status": status,
                        "attempts": attempts,
                        "last_error": last_error,
                        "discovered_pts": disc_pts,
                        "current_pts": cur_pts,
                        "updated_at": updated_at
                    })),
                    recommendation: Some("Check channel membership or access permissions in Telegram client.".to_string()),
                });
            }
            "pending" => {
                findings.push(VerificationFinding {
                    code: "QUEUE_CHANNEL_PENDING".to_string(),
                    severity: FindingSeverity::Info,
                    category: FindingCategory::Queue,
                    peer_id: Some(pid),
                    message_id: None,
                    media_id: None,
                    path: None,
                    description: format!("Channel {pid} is pending synchronization."),
                    evidence: Some(
                        serde_json::json!({ "peer_id": pid, "discovered_pts": disc_pts }),
                    ),
                    recommendation: None,
                });
            }
            _ => {}
        }
    }

    Ok(findings)
}

pub fn audit_migrations(
    ctx: &DatabaseAuditContext,
) -> VerificationResult<Vec<VerificationFinding>> {
    let mut findings = Vec::new();
    let conn = ctx.conn;

    let mut temp_stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name IN ('_media_id_migration_map', '_legacy_migration_backup')",
    )?;
    let temp_rows = temp_stmt.query_map([], |row| row.get::<_, String>(0))?;
    for name in temp_rows.flatten() {
        findings.push(VerificationFinding {
            code: "MIGRATION_TEMP_TABLE_LEFTOVER".to_string(),
            severity: FindingSeverity::Warning,
            category: FindingCategory::Migration,
            peer_id: None,
            message_id: None,
            media_id: None,
            path: None,
            description: format!(
                "Temporary migration table '{name}' was left behind in archive database."
            ),
            evidence: Some(serde_json::json!({ "table": name })),
            recommendation: Some(format!(
                "Drop temporary table '{name}' after verifying data integrity."
            )),
        });
    }

    Ok(findings)
}

pub fn audit_media(
    ctx: &DatabaseAuditContext,
    media_root_dir: Option<&Path>,
    rehash: bool,
) -> VerificationResult<(Vec<VerificationFinding>, MediaAuditMetrics)> {
    let mut findings = Vec::new();
    let conn = ctx.conn;

    let mut metrics = MediaAuditMetrics::default();

    let mut unlinked_msg_stmt = conn.prepare(
        "SELECT mm.peer_id, mm.message_id, mm.media_id
         FROM message_media mm
         LEFT JOIN messages m ON mm.peer_id = m.peer_id AND mm.message_id = m.message_id
         WHERE m.message_id IS NULL",
    )?;
    let unlinked_msg_rows = unlinked_msg_stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let mid: i64 = row.get(1)?;
        let media_id: String = row.get(2)?;
        Ok((pid, mid, media_id))
    })?;

    for (pid, mid, media_id) in unlinked_msg_rows.flatten() {
        findings.push(VerificationFinding {
            code: "MESSAGE_MEDIA_ORPHAN_MESSAGE".to_string(),
            severity: FindingSeverity::Error,
            category: FindingCategory::ReferentialIntegrity,
            peer_id: Some(pid),
            message_id: Some(mid),
            media_id: Some(media_id),
            path: None,
            description: format!(
                "message_media record links to non-existent message ({pid}, {mid})."
            ),
            evidence: None,
            recommendation: Some("Purge unlinked message_media record.".to_string()),
        });
    }

    let mut unlinked_obj_stmt = conn.prepare(
        "SELECT mm.peer_id, mm.message_id, mm.media_id
         FROM message_media mm
         LEFT JOIN media_objects mo ON mm.media_id = mo.media_id
         WHERE mo.media_id IS NULL",
    )?;
    let unlinked_obj_rows = unlinked_obj_stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let mid: i64 = row.get(1)?;
        let media_id: String = row.get(2)?;
        Ok((pid, mid, media_id))
    })?;

    for (pid, mid, media_id) in unlinked_obj_rows.flatten() {
        findings.push(VerificationFinding {
            code: "MESSAGE_MEDIA_ORPHAN_OBJECT".to_string(),
            severity: FindingSeverity::Error,
            category: FindingCategory::ReferentialIntegrity,
            peer_id: Some(pid),
            message_id: Some(mid),
            media_id: Some(media_id.clone()),
            path: None,
            description: format!(
                "message_media references missing media_object record '{media_id}'."
            ),
            evidence: None,
            recommendation: Some("Backfill media_object record from message payload.".to_string()),
        });
    }

    let mut media_stmt = conn.prepare(
        "SELECT media_id, download_status, local_rel_path, size_bytes, sha256, verification_status, downloaded_bytes, worker_id, claimed_at, next_retry_at, filter_decision
         FROM media_objects",
    )?;

    let media_rows = media_stmt.query_map([], |row| {
        let media_id: String = row.get(0)?;
        let status: String = row.get(1)?;
        let rel_path: Option<String> = row.get(2)?;
        let size_bytes: Option<i64> = row.get(3)?;
        let sha256: Option<String> = row.get(4)?;
        let verif_status: String = row.get(5)?;
        let downloaded_bytes: i64 = row.get(6)?;
        let worker_id: Option<String> = row.get(7)?;
        let claimed_at: Option<i64> = row.get(8)?;
        let next_retry_at: Option<i64> = row.get(9)?;
        let filter_decision: Option<String> = row.get(10)?;
        Ok((
            media_id,
            status,
            rel_path,
            size_bytes,
            sha256,
            verif_status,
            downloaded_bytes,
            worker_id,
            claimed_at,
            next_retry_at,
            filter_decision,
        ))
    })?;

    let mut known_rel_paths = HashSet::new();
    let mut active_downloading_paths = HashMap::new();
    let now_ts = now_unix_secs();

    for (
        media_id,
        status,
        rel_path_opt,
        size_opt,
        sha_opt,
        verif_status,
        _dl_bytes,
        worker_id_opt,
        claimed_at_opt,
        next_retry_opt,
        filter_dec,
    ) in media_rows.flatten()
    {
        metrics.total_media_records += 1;

        if filter_dec.as_deref() == Some("skipped") {
            continue;
        }

        match status.as_str() {
            "completed" => {
                if let Some(rel_path) = &rel_path_opt {
                    known_rel_paths.insert(rel_path.clone());

                    let is_verified_state = verif_status == "verified";
                    if !is_verified_state {
                        if verif_status == "unverified" {
                            findings.push(VerificationFinding {
                                code: "MEDIA_UNVERIFIED".to_string(),
                                severity: FindingSeverity::Warning,
                                category: FindingCategory::Media,
                                peer_id: None,
                                message_id: None,
                                media_id: Some(media_id.clone()),
                                path: Some(rel_path.clone()),
                                description: format!(
                                    "Media {media_id} is marked as completed but verification_status is 'unverified'."
                                ),
                                evidence: Some(serde_json::json!({
                                    "media_id": media_id,
                                    "verification_status": verif_status
                                })),
                                recommendation: Some("Run media verification to confirm integrity.".to_string()),
                            });
                        } else {
                            findings.push(VerificationFinding {
                                code: "MEDIA_VERIFICATION_FAILED".to_string(),
                                severity: FindingSeverity::Error,
                                category: FindingCategory::Media,
                                peer_id: None,
                                message_id: None,
                                media_id: Some(media_id.clone()),
                                path: Some(rel_path.clone()),
                                description: format!(
                                    "Media {media_id} verification_status indicates corruption/failure: '{verif_status}'."
                                ),
                                evidence: Some(serde_json::json!({
                                    "media_id": media_id,
                                    "verification_status": verif_status
                                })),
                                recommendation: Some("Re-download corrupted media binary.".to_string()),
                            });
                        }
                    }

                    if let Some(root_dir) = media_root_dir {
                        if let Err(finding) =
                            audit_relative_path(root_dir, rel_path, FindingCategory::Filesystem)
                        {
                            findings.push(*finding);
                        }

                        let full_path = root_dir.join(rel_path);

                        if !full_path.exists() {
                            metrics.missing_files += 1;
                            findings.push(VerificationFinding {
                                code: "MEDIA_FILE_MISSING".to_string(),
                                severity: FindingSeverity::Error,
                                category: FindingCategory::Media,
                                peer_id: None,
                                message_id: None,
                                media_id: Some(media_id.clone()),
                                path: Some(rel_path.clone()),
                                description: format!(
                                    "Completed media binary missing on disk: {}",
                                    full_path.display()
                                ),
                                evidence: Some(serde_json::json!({
                                    "expected_path": full_path.display().to_string()
                                })),
                                recommendation: Some("Re-download missing media file.".to_string()),
                            });
                        } else if let Ok(meta) = full_path.metadata() {
                            let disk_len = meta.len() as i64;
                            if let Some(expected_size) = size_opt {
                                if expected_size > 0 && disk_len != expected_size {
                                    metrics.size_mismatches += 1;
                                    findings.push(VerificationFinding {
                                        code: "MEDIA_SIZE_MISMATCH".to_string(),
                                        severity: FindingSeverity::Error,
                                        category: FindingCategory::Media,
                                        peer_id: None,
                                        message_id: None,
                                        media_id: Some(media_id.clone()),
                                        path: Some(rel_path.clone()),
                                        description: format!(
                                            "Media file size mismatch for {media_id}: disk has {disk_len} bytes, DB expects {expected_size} bytes."
                                        ),
                                        evidence: Some(serde_json::json!({
                                            "disk_bytes": disk_len,
                                            "expected_bytes": expected_size
                                        })),
                                        recommendation: Some(
                                            "Re-download corrupted media file.".to_string(),
                                        ),
                                    });
                                } else if is_verified_state {
                                    if rehash {
                                        if let Some(expected_sha) = &sha_opt {
                                            match compute_sha256(&full_path) {
                                                Ok(actual_sha) => {
                                                    if actual_sha != *expected_sha {
                                                        metrics.hash_mismatches += 1;
                                                        findings.push(VerificationFinding {
                                                            code: "MEDIA_HASH_MISMATCH".to_string(),
                                                            severity: FindingSeverity::Error,
                                                            category: FindingCategory::Media,
                                                            peer_id: None,
                                                            message_id: None,
                                                            media_id: Some(media_id.clone()),
                                                            path: Some(rel_path.clone()),
                                                            description: format!(
                                                                "SHA-256 mismatch for {media_id}: disk has {actual_sha}, DB expects {expected_sha}."
                                                            ),
                                                            evidence: Some(serde_json::json!({
                                                                "disk_sha256": actual_sha,
                                                                "expected_sha256": expected_sha
                                                            })),
                                                            recommendation: Some(
                                                                "Re-download corrupted media binary.".to_string(),
                                                            ),
                                                        });
                                                    } else {
                                                        metrics.completed_verified_on_disk += 1;
                                                        metrics.total_bytes_verified +=
                                                            disk_len as u64;
                                                    }
                                                }
                                                Err(e) => {
                                                    findings.push(VerificationFinding {
                                                        code: "MEDIA_READ_FAILED".to_string(),
                                                        severity: FindingSeverity::Error,
                                                        category: FindingCategory::Media,
                                                        peer_id: None,
                                                        message_id: None,
                                                        media_id: Some(media_id.clone()),
                                                        path: Some(rel_path.clone()),
                                                        description: format!(
                                                            "Failed to read media binary for hashing: {e}"
                                                        ),
                                                        evidence: None,
                                                        recommendation: Some(
                                                            "Check filesystem read permissions.".to_string(),
                                                        ),
                                                    });
                                                }
                                            }
                                        } else {
                                            metrics.completed_verified_on_disk += 1;
                                            metrics.total_bytes_verified += disk_len as u64;
                                        }
                                    } else {
                                        metrics.completed_verified_on_disk += 1;
                                        metrics.total_bytes_verified += disk_len as u64;
                                    }
                                }
                            } else if is_verified_state {
                                metrics.completed_verified_on_disk += 1;
                                metrics.total_bytes_verified += disk_len as u64;
                            }
                        }
                    }
                } else {
                    findings.push(VerificationFinding {
                        code: "COMPLETED_MEDIA_WITHOUT_PATH".to_string(),
                        severity: FindingSeverity::Error,
                        category: FindingCategory::Media,
                        peer_id: None,
                        message_id: None,
                        media_id: Some(media_id.clone()),
                        path: None,
                        description: format!(
                            "Media {media_id} is marked as completed but lacks local_rel_path."
                        ),
                        evidence: None,
                        recommendation: Some("Re-verify media object download status.".to_string()),
                    });
                }
            }
            "downloading" => {
                if let Some(rel_path) = &rel_path_opt {
                    active_downloading_paths
                        .insert(rel_path.clone(), (worker_id_opt.clone(), claimed_at_opt));
                }

                match claimed_at_opt {
                    Some(claimed_at) if claimed_at > 0 => {
                        if now_ts > 0 && claimed_at < (now_ts - 3600) {
                            findings.push(VerificationFinding {
                                code: "MEDIA_STALE_CLAIM".to_string(),
                                severity: FindingSeverity::Warning,
                                category: FindingCategory::Media,
                                peer_id: None,
                                message_id: None,
                                media_id: Some(media_id.clone()),
                                path: rel_path_opt.clone(),
                                description: format!(
                                    "Media {media_id} has stale download claim from worker '{}' (claimed {claimed_at}, older than 1 hour).",
                                    worker_id_opt.as_deref().unwrap_or("unknown")
                                ),
                                evidence: Some(serde_json::json!({
                                    "media_id": media_id,
                                    "worker_id": worker_id_opt,
                                    "claimed_at": claimed_at,
                                    "current_timestamp": now_ts
                                })),
                                recommendation: Some("Release stale download claim and retry.".to_string()),
                            });
                        }
                    }
                    _ => {
                        findings.push(VerificationFinding {
                            code: "MEDIA_INVALID_CLAIM_STATE".to_string(),
                            severity: FindingSeverity::Warning,
                            category: FindingCategory::Media,
                            peer_id: None,
                            message_id: None,
                            media_id: Some(media_id.clone()),
                            path: rel_path_opt.clone(),
                            description: format!(
                                "Media {media_id} is in 'downloading' status but has no valid claimed_at timestamp."
                            ),
                            evidence: Some(serde_json::json!({ "media_id": media_id })),
                            recommendation: Some("Reset status to pending.".to_string()),
                        });
                    }
                }
            }
            "retry_wait" => match next_retry_opt {
                Some(retry_at) if retry_at > 0 => {}
                _ => {
                    findings.push(VerificationFinding {
                        code: "MEDIA_RETRY_WAIT_INVALID".to_string(),
                        severity: FindingSeverity::Warning,
                        category: FindingCategory::Media,
                        peer_id: None,
                        message_id: None,
                        media_id: Some(media_id.clone()),
                        path: rel_path_opt.clone(),
                        description: format!(
                            "Media {media_id} is in 'retry_wait' status but has no valid next_retry_at timestamp."
                        ),
                        evidence: Some(serde_json::json!({ "media_id": media_id })),
                        recommendation: Some("Reset retry_wait schedule or mark pending.".to_string()),
                    });
                }
            },
            _ => {}
        }
    }

    if let Some(root_dir) = media_root_dir
        && root_dir.exists() {
            scan_orphan_files(
                root_dir,
                root_dir,
                &known_rel_paths,
                &active_downloading_paths,
                &mut findings,
                &mut metrics,
            )?;
        }

    Ok((findings, metrics))
}

fn compute_sha256(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn scan_orphan_files(
    root: &Path,
    current: &Path,
    known_paths: &HashSet<String>,
    active_downloading_paths: &HashMap<String, (Option<String>, Option<i64>)>,
    findings: &mut Vec<VerificationFinding>,
    metrics: &mut MediaAuditMetrics,
) -> std::io::Result<()> {
    if current.is_dir() {
        for entry in std::fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                scan_orphan_files(
                    root,
                    &path,
                    known_paths,
                    active_downloading_paths,
                    findings,
                    metrics,
                )?;
            } else if path.is_file() {
                let Ok(rel) = path.strip_prefix(root) else {
                    continue;
                };
                let rel_str = rel.to_string_lossy().to_string();

                if rel_str.ends_with(".part") {
                    metrics.part_files_checked += 1;

                    let base_rel = rel_str.trim_end_matches(".part").to_string();
                    let has_active_lease = active_downloading_paths.contains_key(&base_rel);

                    if !has_active_lease && !known_paths.contains(&base_rel) {
                        findings.push(VerificationFinding {
                            code: "ORPHAN_PART_FILE".to_string(),
                            severity: FindingSeverity::Warning,
                            category: FindingCategory::Filesystem,
                            peer_id: None,
                            message_id: None,
                            media_id: None,
                            path: Some(rel_str.clone()),
                            description: format!(
                                "Incomplete .part file found in media directory with no active worker lease: {rel_str}"
                            ),
                            evidence: Some(serde_json::json!({
                                "path": rel_str,
                                "no_matching_media_object": true,
                                "no_active_lease": true,
                                "reason": "No database record or active worker claim references this .part file"
                            })),
                            recommendation: Some("Resume download or delete stale .part file.".to_string()),
                        });
                    }
                } else if !known_paths.contains(&rel_str)
                    && !active_downloading_paths.contains_key(&rel_str)
                {
                    metrics.orphan_media_files += 1;
                    findings.push(VerificationFinding {
                        code: "ORPHAN_MEDIA_FILE".to_string(),
                        severity: FindingSeverity::Info,
                        category: FindingCategory::Media,
                        peer_id: None,
                        message_id: None,
                        media_id: None,
                        path: Some(rel_str.clone()),
                        description: format!(
                            "File on disk is not referenced by any active/completed media_object: {rel_str}"
                        ),
                        evidence: Some(serde_json::json!({ "path": rel_str })),
                        recommendation: Some(
                            "Investigate if file is from a previous or failed export.".to_string(),
                        ),
                    });
                }
            }
        }
    }
    Ok(())
}
