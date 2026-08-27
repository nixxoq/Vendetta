use std::collections::HashSet;

use serde_json::json;

use crate::{
    auditors::DatabaseAuditContext,
    error::VerificationResult,
    model::{FindingCategory, FindingSeverity, ReplyGraphMetrics, VerificationFinding},
};

pub fn audit_reply_graph(
    ctx: &DatabaseAuditContext,
) -> VerificationResult<(Vec<VerificationFinding>, ReplyGraphMetrics)> {
    let mut findings = Vec::new();
    let conn = ctx.conn;

    let total_replies = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE reply_to_msg_id IS NOT NULL",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize;

    let mut metrics = ReplyGraphMetrics {
        total_replies,
        ..Default::default()
    };

    let mut self_stmt = conn.prepare(
        "SELECT peer_id, message_id, reply_to_msg_id, reply_to_peer_id, reply_to_top_id
         FROM messages
         WHERE reply_to_msg_id = message_id AND (reply_to_peer_id IS NULL OR reply_to_peer_id = peer_id)",
    )?;

    let self_rows = self_stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let mid: i64 = row.get(1)?;
        let r_mid: i64 = row.get(2)?;
        let r_pid: Option<i64> = row.get(3)?;
        let r_top: Option<i64> = row.get(4)?;
        Ok((pid, mid, r_mid, r_pid, r_top))
    })?;

    for (pid, mid, r_mid, r_pid, r_top) in self_rows.flatten() {
        metrics.self_cycles += 1;
        findings.push(VerificationFinding {
            code: "REPLY_SELF_REFERENCE".to_string(),
            severity: FindingSeverity::Error,
            category: FindingCategory::ReplyGraph,
            peer_id: Some(pid),
            message_id: Some(mid),
            media_id: None,
            path: None,
            description: format!("Message ({pid}, {mid}) replies directly to itself."),
            evidence: Some(json!({
                "peer_id": pid,
                "message_id": mid,
                "reply_to_msg_id": r_mid,
                "reply_to_peer_id": r_pid,
                "reply_to_top_id": r_top
            })),
            recommendation: Some("Clear invalid self-referencing reply_to_msg_id.".to_string()),
        });
    }

    let mut target_stmt = conn.prepare(
        "SELECT m.peer_id, m.message_id, m.reply_to_msg_id, m.reply_to_peer_id, m.reply_to_top_id,
                t.state AS target_state,
                (SELECT COUNT(*) FROM peers WHERE peer_id = COALESCE(m.reply_to_peer_id, m.peer_id)) AS target_peer_exists
         FROM messages m
         LEFT JOIN messages t ON t.peer_id = COALESCE(m.reply_to_peer_id, m.peer_id) AND t.message_id = m.reply_to_msg_id
         WHERE m.reply_to_msg_id IS NOT NULL",
    )?;

    let target_rows = target_stmt.query_map([], |row| {
        let pid: i64 = row.get(0)?;
        let mid: i64 = row.get(1)?;
        let r_mid: i64 = row.get(2)?;
        let r_pid: Option<i64> = row.get(3)?;
        let r_top: Option<i64> = row.get(4)?;
        let target_state: Option<String> = row.get(5)?;
        let target_peer_exists: i64 = row.get(6)?;
        Ok((
            pid,
            mid,
            r_mid,
            r_pid,
            r_top,
            target_state,
            target_peer_exists > 0,
        ))
    })?;

    for (pid, mid, r_mid, r_pid, r_top, target_state, peer_exists) in target_rows.flatten() {
        let target_peer_id = r_pid.unwrap_or(pid);
        let is_cross_peer = target_peer_id != pid;
        if is_cross_peer {
            metrics.cross_peer += 1;
        }

        if peer_exists {
            match target_state.as_deref() {
                Some("active") | Some("edited") => {
                    metrics.resolved += 1;
                }
                Some("deleted") | Some("empty") | Some("inaccessible") => {
                    metrics.unavailable += 1;
                    findings.push(VerificationFinding {
                        code: "REPLY_TARGET_UNAVAILABLE".to_string(),
                        severity: FindingSeverity::Info,
                        category: FindingCategory::ReplyGraph,
                        peer_id: Some(pid),
                        message_id: Some(mid),
                        media_id: None,
                        path: None,
                        description: format!(
                            "Message ({pid}, {mid}) replies to message ({target_peer_id}, {r_mid}) which is marked as '{}'.",
                            target_state.as_deref().unwrap_or("unknown")
                        ),
                        evidence: Some(json!({
                            "target_peer_id": target_peer_id,
                            "target_msg_id": r_mid,
                            "target_state": target_state
                        })),
                        recommendation: None,
                    });
                }
                None => {
                    let in_scope = target_peer_id == pid;
                    metrics.missing += 1;
                    findings.push(VerificationFinding {
                        code: if in_scope {
                            "REPLY_TARGET_MISSING".to_string()
                        } else {
                            "REPLY_TARGET_OUT_OF_SCOPE".to_string()
                        },
                        severity: FindingSeverity::Warning,
                        category: FindingCategory::ReplyGraph,
                        peer_id: Some(pid),
                        message_id: Some(mid),
                        media_id: None,
                        path: None,
                        description: if in_scope {
                            format!(
                                "Message ({pid}, {mid}) replies to message ({target_peer_id}, {r_mid}) which does not exist in archive."
                            )
                        } else {
                            format!(
                                "Message ({pid}, {mid}) replies to message ({target_peer_id}, {r_mid}) in an external peer that is not fully synchronized."
                            )
                        },
                        evidence: Some(json!({
                            "target_peer_id": target_peer_id,
                            "target_msg_id": r_mid,
                            "reply_to_top_id": r_top,
                            "in_scope": in_scope
                        })),
                        recommendation: Some(
                            if in_scope {
                                "Synchronize older chat history or resolve reply fallback.".to_string()
                            } else {
                                "Export target peer to resolve cross-peer reply target completely.".to_string()
                            }
                        ),
                    });
                }
                _ => {}
            }
        } else {
            metrics.missing += 1;
            let in_scope = target_peer_id == pid;
            if in_scope {
                findings.push(VerificationFinding {
                    code: "MESSAGE_WITHOUT_PEER_RECORD".to_string(),
                    severity: FindingSeverity::Error,
                    category: FindingCategory::ReferentialIntegrity,
                    peer_id: Some(target_peer_id),
                    message_id: None,
                    media_id: None,
                    path: None,
                    description: format!(
                        "Message ({pid}, {mid}) references in-scope peer {target_peer_id}, but peer record does not exist in peers table."
                    ),
                    evidence: Some(json!({
                        "source_peer_id": pid,
                        "source_message_id": mid,
                        "target_peer_id": target_peer_id,
                        "reply_to_msg_id": r_mid,
                        "in_scope": in_scope
                    })),
                    recommendation: Some("Backfill missing peer record in peers table.".to_string()),
                });
                findings.push(VerificationFinding {
                    code: "REPLY_TARGET_MISSING".to_string(),
                    severity: FindingSeverity::Warning,
                    category: FindingCategory::ReplyGraph,
                    peer_id: Some(pid),
                    message_id: Some(mid),
                    media_id: None,
                    path: None,
                    description: format!(
                        "Message ({pid}, {mid}) replies to message ({target_peer_id}, {r_mid}) in an unrecorded peer."
                    ),
                    evidence: Some(json!({
                        "target_peer_id": target_peer_id,
                        "target_msg_id": r_mid,
                        "in_scope": in_scope
                    })),
                    recommendation: Some("Backfill missing peer and messages.".to_string()),
                });
            } else {
                findings.push(VerificationFinding {
                    code: "REPLY_TARGET_OUT_OF_SCOPE".to_string(),
                    severity: FindingSeverity::Warning,
                    category: FindingCategory::ReplyGraph,
                    peer_id: Some(pid),
                    message_id: Some(mid),
                    media_id: None,
                    path: None,
                    description: format!(
                        "Message ({pid}, {mid}) replies to message ({target_peer_id}, {r_mid}) in an out-of-scope external peer."
                    ),
                    evidence: Some(json!({
                        "target_peer_id": target_peer_id,
                        "target_msg_id": r_mid,
                        "in_scope": in_scope
                    })),
                    recommendation: Some(
                        "Normal for single-peer exports containing cross-chat replies. Export target peer to resolve completely.".to_string(),
                    ),
                });
            }
        }
    }

    if !ctx.is_fast_mode && metrics.total_replies > 0 {
        let mut cycle_stmt = conn.prepare(
            "WITH RECURSIVE reply_walk(root_peer, root_msg, cur_peer, cur_msg, depth, is_cycle, is_depth_exceeded, path) AS (
                SELECT peer_id, message_id, COALESCE(reply_to_peer_id, peer_id), reply_to_msg_id, 1, 0, 0,
                       '/' || peer_id || ':' || message_id || '/'
                FROM messages
                WHERE reply_to_msg_id IS NOT NULL AND reply_to_msg_id != message_id

                UNION ALL

                SELECT w.root_peer, w.root_msg, COALESCE(m.reply_to_peer_id, m.peer_id), m.reply_to_msg_id, w.depth + 1,
                       CASE WHEN instr(w.path, '/' || COALESCE(m.reply_to_peer_id, m.peer_id) || ':' || m.reply_to_msg_id || '/') > 0 THEN 1 ELSE 0 END,
                       CASE WHEN w.depth + 1 >= 1000 THEN 1 ELSE 0 END,
                       w.path || m.peer_id || ':' || m.message_id || '/'
                FROM messages m
                JOIN reply_walk w ON m.peer_id = w.cur_peer AND m.message_id = w.cur_msg
                WHERE m.reply_to_msg_id IS NOT NULL
                  AND w.is_cycle = 0
                  AND w.is_depth_exceeded = 0
                  AND w.depth < 1000
            )
            SELECT DISTINCT root_peer, root_msg, cur_peer, cur_msg, depth, path
            FROM reply_walk
            WHERE is_cycle = 1",
        )?;

        let cycle_rows = cycle_stmt.query_map([], |row| {
            let pid: i64 = row.get(0)?;
            let mid: i64 = row.get(1)?;
            let r_pid: i64 = row.get(2)?;
            let r_mid: i64 = row.get(3)?;
            let depth: i64 = row.get(4)?;
            let path: String = row.get(5)?;
            Ok((pid, mid, r_pid, r_mid, depth, path))
        })?;

        let mut seen_cycles = HashSet::new();

        for (pid, mid, r_pid, r_mid, depth, path) in cycle_rows.flatten() {
            let canonical_key = canonicalize_cycle_path(&path);
            if seen_cycles.insert(canonical_key.clone()) {
                metrics.cycles += 1;
                findings.push(VerificationFinding {
                    code: "REPLY_CYCLE_DETECTED".to_string(),
                    severity: FindingSeverity::Error,
                    category: FindingCategory::ReplyGraph,
                    peer_id: Some(pid),
                    message_id: Some(mid),
                    media_id: None,
                    path: None,
                    description: format!(
                        "Cycle detected in reply chain originating at ({pid}, {mid}) -> ({r_pid}, {r_mid}) (length {depth})."
                    ),
                    evidence: Some(json!({
                        "root_peer_id": pid,
                        "root_message_id": mid,
                        "target_peer_id": r_pid,
                        "target_msg_id": r_mid,
                        "cycle_length": depth,
                        "chain_path": path,
                        "canonical_cycle_key": canonical_key
                    })),
                    recommendation: Some(
                        "Break recursive reply reference in messages table.".to_string(),
                    ),
                });
            }
        }

        let mut depth_stmt = conn.prepare(
            "WITH RECURSIVE reply_walk(root_peer, root_msg, cur_peer, cur_msg, depth, is_cycle, is_depth_exceeded, path) AS (
                SELECT peer_id, message_id, COALESCE(reply_to_peer_id, peer_id), reply_to_msg_id, 1, 0, 0,
                       '/' || peer_id || ':' || message_id || '/'
                FROM messages
                WHERE reply_to_msg_id IS NOT NULL AND reply_to_msg_id != message_id

                UNION ALL

                SELECT w.root_peer, w.root_msg, COALESCE(m.reply_to_peer_id, m.peer_id), m.reply_to_msg_id, w.depth + 1,
                       CASE WHEN instr(w.path, '/' || COALESCE(m.reply_to_peer_id, m.peer_id) || ':' || m.reply_to_msg_id || '/') > 0 THEN 1 ELSE 0 END,
                       CASE WHEN w.depth + 1 >= 1000 THEN 1 ELSE 0 END,
                       w.path || m.peer_id || ':' || m.message_id || '/'
                FROM messages m
                JOIN reply_walk w ON m.peer_id = w.cur_peer AND m.message_id = w.cur_msg
                WHERE m.reply_to_msg_id IS NOT NULL
                  AND w.is_cycle = 0
                  AND w.is_depth_exceeded = 0
                  AND w.depth < 1000
            )
            SELECT DISTINCT root_peer, root_msg, cur_peer, cur_msg, depth, path
            FROM reply_walk
            WHERE is_depth_exceeded = 1 AND is_cycle = 0",
        )?;

        let depth_rows = depth_stmt.query_map([], |row| {
            let pid: i64 = row.get(0)?;
            let mid: i64 = row.get(1)?;
            let r_pid: i64 = row.get(2)?;
            let r_mid: i64 = row.get(3)?;
            let depth: i64 = row.get(4)?;
            let path: String = row.get(5)?;
            Ok((pid, mid, r_pid, r_mid, depth, path))
        })?;

        for (pid, mid, r_pid, r_mid, depth, path) in depth_rows.flatten() {
            metrics.depth_exceeded += 1;
            findings.push(VerificationFinding {
                code: "REPLY_CHAIN_DEPTH_EXCEEDED".to_string(),
                severity: FindingSeverity::Warning,
                category: FindingCategory::ReplyGraph,
                peer_id: Some(pid),
                message_id: Some(mid),
                media_id: None,
                path: None,
                description: format!(
                    "Reply thread depth exceeded maximum limit ({depth} >= 1000) for root ({pid}, {mid})."
                ),
                evidence: Some(json!({
                    "root_peer_id": pid,
                    "root_message_id": mid,
                    "current_peer_id": r_pid,
                    "current_message_id": r_mid,
                    "depth": depth,
                    "chain_path": path
                })),
                recommendation: Some(
                    "Verify deep thread hierarchy or circular reference.".to_string(),
                ),
            });
        }
    }

    Ok((findings, metrics))
}

fn canonicalize_cycle_path(path: &str) -> String {
    let mut raw_parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if raw_parts.is_empty() {
        return path.to_string();
    }
    raw_parts.sort_unstable();
    raw_parts.join(" <-> ")
}
