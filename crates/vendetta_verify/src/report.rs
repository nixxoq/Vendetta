use std::fmt::Write;

use crate::model::{FindingSeverity, OverallStatus, RepairCategory, VerificationReport};

pub fn format_human_readable(report: &VerificationReport) -> String {
    let mut out = String::with_capacity(2048);
    let s = &report.summary;

    out.push_str(
        "================================================================================\n                      VENDETTA ARCHIVE INTEGRITY REPORT                         \n================================================================================\n\n",
    );

    let status_str = match s.status {
        OverallStatus::Passed => "\x1b[32mPASSED\x1b[0m",
        OverallStatus::Warnings => "\x1b[33mWARNINGS\x1b[0m",
        OverallStatus::Errors => "\x1b[31mERRORS\x1b[0m",
        OverallStatus::Fatal => "\x1b[31;1mFATAL\x1b[0m",
    };

    let _ = writeln!(
        out,
        "Overall Status : {status_str} (Exit Code: {})",
        s.exit_code
    );
    let _ = writeln!(out, "Audit Duration : {} ms", s.duration_ms);
    let _ = writeln!(
        out,
        "Findings Summary: {} total ({} fatal, {} errors, {} warnings, {} info)\n",
        s.total_findings, s.fatal_count, s.error_count, s.warning_count, s.info_count
    );

    out.push_str(
        "--------------------------------------------------------------------------------\nEXECUTION SCOPE\n--------------------------------------------------------------------------------\n",
    );
    let _ = writeln!(
        out,
        "  Execution Mode         : {}",
        s.scope.mode.to_uppercase()
    );
    let _ = writeln!(
        out,
        "  Reply Graph Scope      : {}",
        if s.scope.replies_scope {
            "Enabled"
        } else {
            "Disabled (use --replies)"
        }
    );

    let media_status = if s.scope.media_scope_implicit_via_rehash {
        "Enabled (implicit via --rehash)".to_string()
    } else if s.scope.media_scope {
        "Enabled".to_string()
    } else {
        "Disabled (use --media)".to_string()
    };
    let _ = writeln!(out, "  Media Filesystem Scope : {media_status}");
    let _ = writeln!(
        out,
        "  Media Rehash (SHA-256) : {}",
        if s.scope.rehash {
            "Enabled"
        } else {
            "Disabled (use --rehash)"
        }
    );
    let _ = writeln!(
        out,
        "  HTML Export Scope      : {}",
        if s.scope.html_scope {
            "Enabled"
        } else {
            "Disabled (use --html)"
        }
    );

    let search_status = if s.scope.search_scope_executed {
        if s.scope.search_scope_requested {
            "Enabled (requested & executed)".to_string()
        } else {
            "Enabled (executed via HTML export)".to_string()
        }
    } else {
        "Disabled".to_string()
    };
    let _ = writeln!(out, "  Search Index Scope     : {search_status}");

    if !s.scope.core_db_auditors_executed.is_empty() {
        let _ = writeln!(
            out,
            "  Core DB Auditors Run   : {}",
            s.scope.core_db_auditors_executed.join(", ")
        );
    }
    out.push('\n');

    out.push_str(
        "--------------------------------------------------------------------------------\nCATEGORY BREAKDOWN\n--------------------------------------------------------------------------------\n",
    );
    for (cat, count) in &s.category_counts {
        let label = format!("{cat:?}");
        let count_str = if *count == 0 {
            "0 (PASS)".to_string()
        } else {
            format!("{count} finding(s)")
        };
        let _ = writeln!(out, "  {:<26} : {}", label, count_str);
    }
    out.push('\n');

    out.push_str(
        "--------------------------------------------------------------------------------\nCOMPLETENESS DIMENSIONS\n--------------------------------------------------------------------------------\n",
    );
    let d = &s.dimensions;
    let _ = writeln!(
        out,
        "  Message History        : {} ({})",
        d.message_history.status, d.message_history.reason
    );
    let _ = writeln!(
        out,
        "  Deletion Verification  : {} ({})",
        d.deletion_verification.status, d.deletion_verification.reason
    );
    let _ = writeln!(
        out,
        "  Media Binaries         : {} ({})",
        d.media_binaries.status, d.media_binaries.reason
    );
    let _ = writeln!(
        out,
        "  Channel Discovery      : {} ({})",
        d.channel_discovery.status, d.channel_discovery.reason
    );
    let _ = writeln!(
        out,
        "  Sync Uncertainty       : {} ({})",
        d.sync_uncertainty.status, d.sync_uncertainty.reason
    );
    let _ = writeln!(
        out,
        "  Search Index           : {} ({})",
        d.search_index.status, d.search_index.reason
    );
    let _ = writeln!(
        out,
        "  HTML Export            : {} ({})",
        d.html_export.status, d.html_export.reason
    );
    out.push('\n');

    if let Some(rm) = &s.reply_metrics {
        out.push_str(
            "--------------------------------------------------------------------------------\nREPLY & THREAD GRAPH METRICS\n--------------------------------------------------------------------------------\n",
        );
        let _ = writeln!(out, "  Total Reply References : {}", rm.total_replies);
        let _ = writeln!(out, "  Resolved Targets       : {}", rm.resolved);
        let _ = writeln!(
            out,
            "  Unavailable Targets    : {} (Deleted/Empty/Inaccessible)",
            rm.unavailable
        );
        let _ = writeln!(out, "  Out-of-Scope Targets   : {}", rm.out_of_scope);
        let _ = writeln!(out, "  Missing Targets        : {}", rm.missing);
        let _ = writeln!(out, "  Cross-Peer Replies     : {}", rm.cross_peer);
        let _ = writeln!(
            out,
            "  Self-Cycles / Cycles   : {} / {}",
            rm.self_cycles, rm.cycles
        );
        let _ = writeln!(out, "  Depth Exceeded Chains  : {}", rm.depth_exceeded);
        out.push('\n');
    }

    if let Some(mm) = &s.media_metrics {
        out.push_str(
            "--------------------------------------------------------------------------------\nMEDIA & STORAGE AUDIT METRICS\n--------------------------------------------------------------------------------\n",
        );
        let _ = writeln!(out, "  Total Media Records    : {}", mm.total_media_records);
        let _ = writeln!(
            out,
            "  Completed & Verified   : {}",
            mm.completed_verified_on_disk
        );
        let _ = writeln!(out, "  Missing Files on Disk  : {}", mm.missing_files);
        let _ = writeln!(out, "  Size Mismatches        : {}", mm.size_mismatches);
        let _ = writeln!(out, "  Hash Mismatches        : {}", mm.hash_mismatches);
        let _ = writeln!(out, "  Active .part Files     : {}", mm.part_files_checked);
        let _ = writeln!(out, "  Orphan Files on Disk   : {}", mm.orphan_media_files);
        let _ = writeln!(
            out,
            "  Total Verified Bytes   : {} bytes ({:.2} MB)",
            mm.total_bytes_verified,
            mm.total_bytes_verified as f64 / 1_048_576.0
        );
        out.push('\n');
    }

    if !report.findings.is_empty() {
        out.push_str(
            "--------------------------------------------------------------------------------\nAUDIT FINDINGS\n--------------------------------------------------------------------------------\n",
        );
        for (idx, f) in report.findings.iter().take(50).enumerate() {
            let sev_tag = match f.severity {
                FindingSeverity::Fatal => "[FATAL]",
                FindingSeverity::Error => "[ERROR]",
                FindingSeverity::Warning => "[WARN ]",
                FindingSeverity::Info => "[INFO ]",
            };
            let _ = writeln!(
                out,
                "{:<3}. {} {:<32} - {}",
                idx + 1,
                sev_tag,
                f.code,
                f.description
            );
            if let Some(pid) = f.peer_id {
                let mid_str = f
                    .message_id
                    .map(|m| format!(" Msg:{m}"))
                    .unwrap_or_default();
                let _ = writeln!(out, "     Context: Peer:{pid}{mid_str}");
            }
            if let Some(media_id) = &f.media_id {
                let _ = writeln!(out, "     Media ID: {media_id}");
            }
            if let Some(path) = &f.path {
                let _ = writeln!(out, "     Path: {path}");
            }
        }
        if report.findings.len() > 50 {
            let _ = writeln!(
                out,
                "  ... and {} more findings (run with --json for complete list)",
                report.findings.len() - 50
            );
        }
        out.push('\n');
    }

    if !report.repair_plan.recommendations.is_empty() {
        out.push_str(
            "--------------------------------------------------------------------------------\nREPAIR RECOMMENDATIONS\n--------------------------------------------------------------------------------\n",
        );
        for (idx, rec) in report.repair_plan.recommendations.iter().enumerate() {
            let cat_str = match rec.category {
                RepairCategory::SafeAutomation => "[SAFE AUTO]",
                RepairCategory::ManualReview => "[MANUAL   ]",
                RepairCategory::RequiresTelegramResync => "[RESYNC   ]",
            };
            let _ = writeln!(
                out,
                "{:<2}. {} {} (Affected: {})",
                idx + 1,
                cat_str,
                rec.description,
                rec.affected_count
            );
            if let Some(cmd) = &rec.suggested_command {
                let _ = writeln!(out, "    Suggested Command: `{cmd}`");
            }
            let _ = writeln!(out, "    Rationale: {}", rec.why_safe_or_risky);
        }
        out.push('\n');
    }

    out.push_str(
        "================================================================================\n",
    );
    out
}

pub fn format_json(report: &VerificationReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}
