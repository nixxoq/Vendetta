use std::{collections::BTreeMap, time::Instant};

use rusqlite::OptionalExtension;
use vendetta_storage::{ArchiveDb, StorageError};

use crate::{
    auditors::{
        DatabaseAuditContext,
        db::{
            audit_channel_queue, audit_chronology, audit_entities, audit_identity, audit_media,
            audit_message_states, audit_migrations, audit_revisions, audit_schema,
            audit_service_messages, audit_sync_state, audit_unsupported_events,
        },
        export::{audit_html_export, audit_search_index},
        reply_graph::audit_reply_graph,
    },
    error::VerificationResult,
    model::{
        CompletenessDimensions, DimensionDetail, ExecutionScope, FindingCategory, FindingSeverity,
        MediaAuditMetrics, ReplyGraphMetrics, VerificationFinding, VerificationMode,
        VerificationOptions, VerificationReport, VerificationSummary,
    },
    repair::RepairPlanner,
    status::{calculate_exit_status, compare_findings, derive_overall_status},
};

#[derive(Debug, Clone, Default)]
struct ProvenanceMetrics {
    has_sync_reports: bool,
    #[allow(dead_code)]
    provenance_version: u32,
    #[allow(dead_code)]
    deletion_reconciliation_performed: bool,
    deletion_reconciliation_complete: bool,
    deletion_reconciliation_incomplete: bool,
    #[allow(dead_code)]
    historical_message_reconciliation_performed: bool,
    message_reconciliation_complete: bool,
    message_reconciliation_incomplete: bool,
    channel_discovery_complete: bool,
    #[allow(dead_code)]
    fully_lossless_sync: bool,
    #[allow(dead_code)]
    event_window_lost: bool,
    has_blocked_channels: bool,
}

pub struct VerificationEngine {
    options: VerificationOptions,
}

impl VerificationEngine {
    pub fn new(options: VerificationOptions) -> Self {
        Self { options }
    }

    pub fn run(&self) -> VerificationResult<VerificationReport> {
        let start_time = Instant::now();
        let mut findings = Vec::new();
        let mut reply_metrics: Option<ReplyGraphMetrics> = None;
        let mut media_metrics: Option<MediaAuditMetrics> = None;
        let mut prov_metrics = ProvenanceMetrics::default();

        let mut dimensions = CompletenessDimensions::default();

        let db_opt = if let Some(archive_path) = &self.options.archive_path {
            if !archive_path.exists() {
                findings.push(VerificationFinding {
                    code: "ARCHIVE_DATABASE_NOT_FOUND".to_string(),
                    severity: FindingSeverity::Fatal,
                    category: FindingCategory::Schema,
                    peer_id: None,
                    message_id: None,
                    media_id: None,
                    path: Some(archive_path.display().to_string()),
                    description: format!(
                        "Archive database file does not exist: {}",
                        archive_path.display()
                    ),
                    evidence: None,
                    recommendation: Some("Verify path to SQLite archive file.".to_string()),
                });
                None
            } else {
                match ArchiveDb::open_read_only(archive_path) {
                    Ok(db) => Some(db),
                    Err(e) => {
                        findings.push(VerificationFinding {
                            code: "ARCHIVE_OPEN_FAILED".to_string(),
                            severity: FindingSeverity::Fatal,
                            category: FindingCategory::Schema,
                            peer_id: None,
                            message_id: None,
                            media_id: None,
                            path: Some(archive_path.display().to_string()),
                            description: format!(
                                "Failed to open archive database in read-only mode: {e}"
                            ),
                            evidence: None,
                            recommendation: Some(
                                "Check file read permissions or SQLite file integrity.".to_string(),
                            ),
                        });
                        None
                    }
                }
            }
        } else {
            None
        };

        let is_fast = self.options.mode == VerificationMode::Fast;
        let should_audit_replies = self.options.scope_replies;
        let should_audit_media = self.options.scope_media || self.options.rehash_media;

        let mut core_db_auditors_executed = Vec::new();

        if let Some(db) = &db_opt {
            let (db_findings, r_m, m_m, prov) = db.with_conn(|conn| -> Result<_, StorageError> {
                let ctx = DatabaseAuditContext {
                    conn,
                    is_fast_mode: is_fast,
                };

                let mut inner_findings = Vec::new();
                let mut inner_r_m = None;
                let mut inner_m_m = None;

                let has_prov_cols = conn
                    .prepare("SELECT provenance_version FROM sync_integrity_reports LIMIT 0")
                    .is_ok();

                struct RawSyncIntegrityRow {
                    _del_comp_legacy: i64,
                    chan_comp: i64,
                    loss_sync: i64,
                    event_lost: i64,
                    prov_version: i64,
                    del_perf: i64,
                    del_comp: i64,
                    del_gaps: i64,
                    msg_perf: i64,
                    msg_comp: i64,
                    msg_gaps: i64,
                }

                let report_opt: Option<RawSyncIntegrityRow> = if has_prov_cols {
                    conn.query_row(
                        "SELECT historical_deletions_complete, channel_discovery_complete,
                                fully_lossless_contiguous_sync, event_window_lost,
                                provenance_version, deletion_reconciliation_performed,
                                deletion_reconciliation_complete, deletion_event_gap_count,
                                historical_message_reconciliation_performed,
                                historical_message_reconciliation_complete,
                                historical_message_gap_count
                         FROM sync_integrity_reports
                         WHERE scope = 'full_sync_run' AND peer_id IS NULL
                         ORDER BY created_at DESC
                         LIMIT 1",
                        [],
                        |r| {
                            Ok(RawSyncIntegrityRow {
                                _del_comp_legacy: r.get(0)?,
                                chan_comp: r.get(1)?,
                                loss_sync: r.get(2)?,
                                event_lost: r.get(3)?,
                                prov_version: r.get(4)?,
                                del_perf: r.get(5)?,
                                del_comp: r.get(6)?,
                                del_gaps: r.get(7)?,
                                msg_perf: r.get(8)?,
                                msg_comp: r.get(9)?,
                                msg_gaps: r.get(10)?,
                            })
                        },
                    )
                    .optional()
                    .unwrap_or(None)
                } else {
                    conn.query_row(
                        "SELECT historical_deletions_complete, channel_discovery_complete,
                                fully_lossless_contiguous_sync, event_window_lost,
                                1, 0, 0, 0, 0, 0, 0
                         FROM sync_integrity_reports
                         WHERE scope = 'full_sync_run' AND peer_id IS NULL
                         ORDER BY created_at DESC
                         LIMIT 1",
                        [],
                        |r| {
                            Ok(RawSyncIntegrityRow {
                                _del_comp_legacy: r.get(0)?,
                                chan_comp: r.get(1)?,
                                loss_sync: r.get(2)?,
                                event_lost: r.get(3)?,
                                prov_version: r.get(4)?,
                                del_perf: r.get(5)?,
                                del_comp: r.get(6)?,
                                del_gaps: r.get(7)?,
                                msg_perf: r.get(8)?,
                                msg_comp: r.get(9)?,
                                msg_gaps: r.get(10)?,
                            })
                        },
                    )
                    .optional()
                    .unwrap_or(None)
                };

                let blocked_count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM channel_sync_queue WHERE status IN ('blocked', 'failed')",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);

                let inner_prov = match report_opt {
                    Some(row) => {
                        let deletion_prov_complete = row.prov_version >= 2
                            && row.del_perf > 0
                            && row.del_comp > 0
                            && row.del_gaps == 0
                            && row.event_lost == 0;

                        let deletion_prov_incomplete = (row.prov_version >= 2
                            && row.del_perf > 0
                            && (row.del_comp == 0 || row.del_gaps > 0))
                            || row.event_lost > 0;

                        let message_prov_complete = row.prov_version >= 2
                            && row.msg_perf > 0
                            && row.msg_comp > 0
                            && row.msg_gaps == 0;

                        let message_prov_incomplete = row.prov_version >= 2
                            && row.msg_perf > 0
                            && (row.msg_comp == 0 || row.msg_gaps > 0);

                        ProvenanceMetrics {
                            has_sync_reports: true,
                            provenance_version: row.prov_version as u32,
                            deletion_reconciliation_performed: row.del_perf > 0,
                            deletion_reconciliation_complete: deletion_prov_complete,
                            deletion_reconciliation_incomplete: deletion_prov_incomplete,
                            historical_message_reconciliation_performed: row.msg_perf > 0,
                            message_reconciliation_complete: message_prov_complete,
                            message_reconciliation_incomplete: message_prov_incomplete,
                            channel_discovery_complete: row.chan_comp > 0,
                            fully_lossless_sync: row.loss_sync > 0,
                            event_window_lost: row.event_lost > 0,
                            has_blocked_channels: blocked_count > 0,
                        }
                    }
                    None => ProvenanceMetrics {
                        has_sync_reports: false,
                        provenance_version: 0,
                        deletion_reconciliation_performed: false,
                        deletion_reconciliation_complete: false,
                        deletion_reconciliation_incomplete: false,
                        historical_message_reconciliation_performed: false,
                        message_reconciliation_complete: false,
                        message_reconciliation_incomplete: false,
                        channel_discovery_complete: false,
                        fully_lossless_sync: false,
                        event_window_lost: false,
                        has_blocked_channels: blocked_count > 0,
                    },
                };

                let schema_findings = audit_schema(&ctx)
                    .map_err(|e| StorageError::Transaction(e.to_string()))?;
                inner_findings.extend(schema_findings);

                let has_schema_fatal = inner_findings
                    .iter()
                    .any(|f| f.severity == FindingSeverity::Fatal);
                if !has_schema_fatal {
                    inner_findings.extend(
                        audit_identity(&ctx)
                            .map_err(|e| StorageError::Transaction(e.to_string()))?,
                    );

                    inner_findings.extend(
                        audit_chronology(&ctx)
                            .map_err(|e| StorageError::Transaction(e.to_string()))?,
                    );

                    inner_findings.extend(
                        audit_message_states(&ctx)
                            .map_err(|e| StorageError::Transaction(e.to_string()))?,
                    );

                    if should_audit_replies {
                        let (reply_findings, r_metrics) = audit_reply_graph(&ctx)
                            .map_err(|e| StorageError::Transaction(e.to_string()))?;
                        inner_findings.extend(reply_findings);
                        inner_r_m = Some(r_metrics);
                    }

                    if !is_fast {
                        inner_findings.extend(
                            audit_revisions(&ctx)
                                .map_err(|e| StorageError::Transaction(e.to_string()))?,
                        );
                        inner_findings.extend(
                            audit_entities(&ctx)
                                .map_err(|e| StorageError::Transaction(e.to_string()))?,
                        );
                        inner_findings.extend(
                            audit_service_messages(&ctx)
                                .map_err(|e| StorageError::Transaction(e.to_string()))?,
                        );
                    }

                    if should_audit_media {
                        let media_dir_resolved = self.options.media_dir.clone().or_else(|| {
                            self.options
                                .archive_path
                                .as_ref()
                                .and_then(|p| p.parent().map(|d| d.join("media")))
                        });
                        let (media_findings, m_metrics) = audit_media(
                            &ctx,
                            media_dir_resolved.as_deref(),
                            self.options.rehash_media,
                        )
                        .map_err(|e| StorageError::Transaction(e.to_string()))?;
                        inner_findings.extend(media_findings);
                        inner_m_m = Some(m_metrics);
                    }

                    if !is_fast {
                        inner_findings.extend(
                            audit_sync_state(&ctx)
                                .map_err(|e| StorageError::Transaction(e.to_string()))?,
                        );
                        inner_findings.extend(
                            audit_unsupported_events(&ctx)
                                .map_err(|e| StorageError::Transaction(e.to_string()))?,
                        );
                        inner_findings.extend(
                            audit_channel_queue(&ctx)
                                .map_err(|e| StorageError::Transaction(e.to_string()))?,
                        );
                        inner_findings.extend(
                            audit_migrations(&ctx)
                                .map_err(|e| StorageError::Transaction(e.to_string()))?,
                        );
                    }
                }

                Ok((inner_findings, inner_r_m, inner_m_m, inner_prov))
            })?;

            findings.extend(db_findings);
            reply_metrics = r_m;
            media_metrics = m_m;
            prov_metrics = prov;

            if is_fast {
                core_db_auditors_executed = vec![
                    "schema".to_string(),
                    "identity".to_string(),
                    "chronology".to_string(),
                    "message_state".to_string(),
                ];
            } else {
                core_db_auditors_executed = vec![
                    "schema".to_string(),
                    "identity".to_string(),
                    "chronology".to_string(),
                    "message_state".to_string(),
                    "revision".to_string(),
                    "entity".to_string(),
                    "service_message".to_string(),
                    "sync_state".to_string(),
                    "unsupported_event".to_string(),
                    "channel_queue".to_string(),
                    "migration".to_string(),
                ];
            }
        }

        let search_scope_executed = self.options.html_dir.is_some();

        if let Some(html_dir) = &self.options.html_dir {
            let html_findings = audit_html_export(html_dir, db_opt.as_ref())?;
            findings.extend(html_findings);

            let search_findings = audit_search_index(html_dir)?;
            findings.extend(search_findings);
        }

        evaluate_dimensions(
            &findings,
            media_metrics.as_ref(),
            &prov_metrics,
            should_audit_media,
            &self.options,
            &mut dimensions,
        );

        let repair_plan = RepairPlanner::build_plan(&findings);
        findings.sort_by(compare_findings);

        let mut fatal_count = 0;
        let mut error_count = 0;
        let mut warning_count = 0;
        let mut info_count = 0;
        let mut category_counts = BTreeMap::new();

        let all_categories = [
            FindingCategory::Schema,
            FindingCategory::ReferentialIntegrity,
            FindingCategory::Identity,
            FindingCategory::Chronology,
            FindingCategory::MessageState,
            FindingCategory::ReplyGraph,
            FindingCategory::Revision,
            FindingCategory::Entity,
            FindingCategory::ServiceMessage,
            FindingCategory::Media,
            FindingCategory::Filesystem,
            FindingCategory::SyncState,
            FindingCategory::UnsupportedUpdate,
            FindingCategory::Queue,
            FindingCategory::Migration,
            FindingCategory::Search,
            FindingCategory::HtmlExport,
            FindingCategory::Manifest,
            FindingCategory::Security,
            FindingCategory::Completeness,
        ];
        for cat in all_categories {
            category_counts.insert(cat, 0);
        }

        for f in &findings {
            match f.severity {
                FindingSeverity::Fatal => fatal_count += 1,
                FindingSeverity::Error => error_count += 1,
                FindingSeverity::Warning => warning_count += 1,
                FindingSeverity::Info => info_count += 1,
            }
            *category_counts.entry(f.category).or_insert(0) += 1;
        }

        let overall_status = derive_overall_status(fatal_count, error_count, warning_count);
        let exit_code =
            calculate_exit_status(fatal_count, error_count, warning_count, self.options.strict);

        let duration_ms = start_time.elapsed().as_millis() as u64;

        let scope = ExecutionScope {
            mode: if is_fast {
                "fast".to_string()
            } else {
                "full".to_string()
            },
            media_scope: should_audit_media,
            media_scope_implicit_via_rehash: self.options.rehash_media && !self.options.scope_media,
            replies_scope: should_audit_replies,
            search_scope_requested: self.options.scope_search,
            search_scope_executed,
            search_scope: search_scope_executed,
            html_scope: self.options.html_dir.is_some(),
            rehash: self.options.rehash_media,
            core_db_auditors_executed,
        };

        let summary = VerificationSummary {
            status: overall_status,
            exit_code,
            total_findings: findings.len(),
            fatal_count,
            error_count,
            warning_count,
            info_count,
            scope,
            category_counts,
            dimensions,
            reply_metrics,
            media_metrics,
            duration_ms,
        };

        Ok(VerificationReport {
            schema_version: 1,
            summary,
            findings,
            repair_plan,
        })
    }
}

fn dim_na(reason: &'static str) -> DimensionDetail {
    DimensionDetail {
        status: "not_applicable".to_string(),
        reason: reason.to_string(),
        affected_count: 0,
        evidence_codes: Vec::new(),
    }
}

fn dim_complete(reason: impl Into<String>) -> DimensionDetail {
    DimensionDetail {
        status: "complete".to_string(),
        reason: reason.into(),
        affected_count: 0,
        evidence_codes: Vec::new(),
    }
}

fn dim_issue(
    status: &str,
    reason: impl Into<String>,
    affected: usize,
    evidence: &[&str],
) -> DimensionDetail {
    DimensionDetail {
        status: status.to_string(),
        reason: reason.into(),
        affected_count: affected,
        evidence_codes: evidence.iter().map(|s| s.to_string()).collect(),
    }
}

fn evaluate_dimensions(
    findings: &[VerificationFinding],
    media_metrics: Option<&MediaAuditMetrics>,
    prov_metrics: &ProvenanceMetrics,
    media_audited: bool,
    opts: &VerificationOptions,
    dims: &mut CompletenessDimensions,
) {
    let has_sync_uncertainty = findings
        .iter()
        .any(|f| f.code == "SYNC_UNCERTAIN" || f.code == "PEER_SYNC_UNCERTAIN");
    let has_unsupported_state = findings
        .iter()
        .any(|f| f.code == "UNSUPPORTED_STATE_AFFECTING_UPDATE");
    let has_blocked_channels = findings.iter().any(|f| f.code == "QUEUE_CHANNEL_BLOCKED");
    let has_missing_peer_record = findings
        .iter()
        .any(|f| f.code == "MESSAGE_WITHOUT_PEER_RECORD");
    let has_html_mismatch = findings
        .iter()
        .any(|f| f.code == "HTML_SOURCE_FINGERPRINT_MISMATCH");
    let has_html_error = findings.iter().any(|f| {
        (f.category == FindingCategory::HtmlExport || f.category == FindingCategory::Manifest)
            && (f.severity == FindingSeverity::Error || f.severity == FindingSeverity::Fatal)
            && f.code != "HTML_SOURCE_FINGERPRINT_MISMATCH"
    });
    let has_html_warning = findings.iter().any(|f| {
        (f.category == FindingCategory::HtmlExport || f.category == FindingCategory::Manifest)
            && f.severity == FindingSeverity::Warning
    });

    let has_search_error = findings.iter().any(|f| {
        f.category == FindingCategory::Search
            && (f.severity == FindingSeverity::Error || f.severity == FindingSeverity::Fatal)
    });
    let has_search_warning = findings
        .iter()
        .any(|f| f.category == FindingCategory::Search && f.severity == FindingSeverity::Warning);

    if opts.archive_path.is_some() {
        if has_sync_uncertainty || has_unsupported_state {
            dims.message_history = dim_issue(
                "uncertain",
                "Unresolved sync gap or unsupported state-affecting update recorded.",
                1,
                &["SYNC_UNCERTAIN"],
            );
        } else if has_missing_peer_record {
            dims.message_history = dim_issue(
                "incomplete",
                "Messages exist with unrecorded peer records.",
                1,
                &["MESSAGE_WITHOUT_PEER_RECORD"],
            );
        } else if prov_metrics.has_sync_reports {
            if prov_metrics.message_reconciliation_complete {
                dims.message_history = dim_complete(
                    "Historical message coverage and gap reconciliation completed cleanly with verified baseline.",
                );
            } else if prov_metrics.message_reconciliation_incomplete {
                dims.message_history = dim_issue(
                    "incomplete",
                    "Historical message reconciliation detected unrecovered message gaps.",
                    1,
                    &["MESSAGE_HISTORY_GAP_DETECTED"],
                );
            } else {
                dims.message_history = dim_issue(
                    "uncertain",
                    "No unresolved sync gaps detected, but historical completeness provenance is unavailable.",
                    0,
                    &[],
                );
            }
        } else {
            dims.message_history = dim_issue(
                "uncertain",
                "No unresolved sync gaps detected, but historical completeness provenance is unavailable.",
                0,
                &[],
            );
        }

        if has_sync_uncertainty {
            dims.deletion_verification = dim_issue(
                "uncertain",
                "Deletions cannot be definitively verified due to sync stream gaps.",
                1,
                &["SYNC_UNCERTAIN"],
            );
        } else if prov_metrics.has_sync_reports {
            if prov_metrics.deletion_reconciliation_complete {
                dims.deletion_verification = dim_complete(
                    "Historical deletion tombstones and difference events reconciled with verified baseline.",
                );
            } else if prov_metrics.deletion_reconciliation_incomplete {
                dims.deletion_verification = dim_issue(
                    "incomplete",
                    "Historical deletion reconciliation recorded event window loss or gaps.",
                    1,
                    &["DELETION_RECONCILIATION_INCOMPLETE"],
                );
            } else {
                dims.deletion_verification = dim_issue(
                    "uncertain",
                    "Historical deletion tracking provenance absent or unverified against server differences.",
                    0,
                    &[],
                );
            }
        } else {
            dims.deletion_verification = dim_issue(
                "uncertain",
                "Historical deletion tracking provenance absent or unverified against server differences.",
                0,
                &[],
            );
        }

        if opts.mode == VerificationMode::Fast {
            dims.channel_discovery = dim_na("Channel queue skipped in fast mode.");
        } else if has_blocked_channels || prov_metrics.has_blocked_channels {
            dims.channel_discovery = dim_issue(
                "incomplete",
                "One or more channels are blocked or inaccessible.",
                1,
                &["QUEUE_CHANNEL_BLOCKED"],
            );
        } else if prov_metrics.has_sync_reports {
            if prov_metrics.channel_discovery_complete {
                dims.channel_discovery =
                    dim_complete("All discovered channels have baselines established.");
            } else {
                dims.channel_discovery = dim_issue(
                    "incomplete",
                    "Channel discovery finished with incomplete status.",
                    1,
                    &["CHANNEL_DISCOVERY_INCOMPLETE"],
                );
            }
        } else {
            dims.channel_discovery = dim_issue(
                "uncertain",
                "Channel discovery provenance unavailable or unverified against server dialogs.",
                0,
                &[],
            );
        }

        if opts.mode == VerificationMode::Fast {
            dims.sync_uncertainty = dim_na("Sync state skipped in fast mode.");
        } else if has_sync_uncertainty || has_unsupported_state {
            dims.sync_uncertainty = dim_issue(
                "uncertain",
                "Account or peer sync state is marked sync_uncertain.",
                1,
                &["SYNC_UNCERTAIN"],
            );
        } else {
            dims.sync_uncertainty = DimensionDetail {
                status: "clean".to_string(),
                reason: "No active state uncertainty flags.".to_string(),
                affected_count: 0,
                evidence_codes: Vec::new(),
            };
        }

        if media_audited {
            if let Some(mm) = media_metrics {
                let unverified_count = findings
                    .iter()
                    .filter(|f| {
                        f.code == "MEDIA_UNVERIFIED" || f.code == "MEDIA_VERIFICATION_FAILED"
                    })
                    .count();

                if mm.total_media_records == 0 {
                    dims.media_binaries = dim_complete("No media objects present in archive.");
                } else if mm.missing_files > 0
                    || mm.size_mismatches > 0
                    || mm.hash_mismatches > 0
                    || unverified_count > 0
                {
                    let missing_total = mm.missing_files
                        + mm.size_mismatches
                        + mm.hash_mismatches
                        + unverified_count;
                    let pct = (mm.completed_verified_on_disk as f64
                        / mm.total_media_records as f64)
                        * 100.0;
                    dims.media_binaries = dim_issue(
                        &format!("{pct:.1}%"),
                        format!(
                            "{missing_total} media files missing, unverified, or corrupted on disk."
                        ),
                        missing_total,
                        &["MEDIA_FILE_MISSING"],
                    );
                } else {
                    dims.media_binaries = DimensionDetail {
                        status: "100.0%".to_string(),
                        reason: format!(
                            "All {} media binaries verified on disk.",
                            mm.completed_verified_on_disk
                        ),
                        affected_count: 0,
                        evidence_codes: Vec::new(),
                    };
                }
            } else {
                dims.media_binaries = dim_na("Media binaries not audited (use --media to audit).");
            }
        } else {
            dims.media_binaries = dim_na("Media binaries not audited (use --media to audit).");
        }
    } else {
        dims.message_history = dim_na("No SQLite archive provided.");
        dims.deletion_verification = dim_na("No SQLite archive provided.");
        dims.channel_discovery = dim_na("No SQLite archive provided.");
        dims.sync_uncertainty = dim_na("No SQLite archive provided.");
        dims.media_binaries = dim_na("No SQLite archive provided.");
    }

    if let Some(html_dir) = &opts.html_dir {
        if has_html_mismatch {
            dims.html_export = dim_issue(
                "mismatched",
                "HTML export source fingerprint does not match SQLite archive.",
                1,
                &["HTML_SOURCE_FINGERPRINT_MISMATCH"],
            );
        } else if has_html_error {
            dims.html_export = dim_issue(
                "corrupted",
                "HTML export structure or manifest contains integrity violations.",
                1,
                &["HTML_INTEGRITY_VIOLATION"],
            );
        } else if has_html_warning {
            dims.html_export = dim_issue(
                "incomplete",
                "HTML export structure or manifest contains warnings or inconsistencies.",
                1,
                &["HTML_EXPORT_WARNING"],
            );
        } else if opts.archive_path.is_none() {
            dims.html_export = DimensionDetail {
                status: "consistent".to_string(),
                reason: "Static HTML structure/search/assets verified; source archive equivalence was not checked.".to_string(),
                affected_count: 0,
                evidence_codes: Vec::new(),
            };
        } else {
            dims.html_export = DimensionDetail {
                status: "consistent".to_string(),
                reason: "Static HTML verified and source fingerprint matches archive.".to_string(),
                affected_count: 0,
                evidence_codes: Vec::new(),
            };
        }

        if has_search_error {
            dims.search_index = dim_issue(
                "corrupted",
                "Search manifest or shard files are missing or malformed.",
                1,
                &["SEARCH_SHARD_MISSING"],
            );
        } else if has_search_warning {
            dims.search_index = dim_issue(
                "incomplete",
                "Search index contains undeclared shards or inconsistencies.",
                1,
                &["SEARCH_UNDECLARED_SHARD"],
            );
        } else {
            let is_disabled = html_dir.join("manifest.json").exists()
                && !html_dir.join("search/manifest.js").exists();
            if is_disabled {
                dims.search_index = dim_na("Search index generation was disabled for this export.");
            } else {
                dims.search_index =
                    dim_complete("Search manifest and all declared shards verified.");
            }
        }
    } else {
        dims.html_export = dim_na("No HTML export provided.");
        dims.search_index = dim_na("No HTML/search export provided.");
    }
}
