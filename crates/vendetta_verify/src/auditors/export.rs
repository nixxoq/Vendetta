use std::{collections::HashSet, fs, path::Path};

use serde_json::{Value, json};
use vendetta_render::{
    manifest::{DatasetFingerprint, HtmlExportManifest},
    verifier::HtmlArchiveVerifier,
};
use vendetta_storage::ArchiveDb;

use crate::{
    error::VerificationResult,
    model::{FindingCategory, FindingSeverity, VerificationFinding},
};

pub fn audit_html_export(
    export_dir: &Path,
    db: Option<&ArchiveDb>,
) -> VerificationResult<Vec<VerificationFinding>> {
    let mut findings = Vec::new();

    if !export_dir.exists() {
        findings.push(VerificationFinding {
            code: "HTML_EXPORT_DIR_MISSING".to_string(),
            severity: FindingSeverity::Fatal,
            category: FindingCategory::HtmlExport,
            peer_id: None,
            message_id: None,
            media_id: None,
            path: Some(export_dir.display().to_string()),
            description: format!("Export directory does not exist: {}", export_dir.display()),
            evidence: None,
            recommendation: Some("Generate HTML export using 'vendetta export-html'.".to_string()),
        });
        return Ok(findings);
    }

    let manifest_path = export_dir.join("manifest.json");
    let manifest = if !manifest_path.exists() {
        findings.push(VerificationFinding {
            code: "HTML_MANIFEST_MISSING".to_string(),
            severity: FindingSeverity::Error,
            category: FindingCategory::HtmlExport,
            peer_id: None,
            message_id: None,
            media_id: None,
            path: Some("manifest.json".to_string()),
            description: "manifest.json is missing from export directory.".to_string(),
            evidence: None,
            recommendation: Some("Re-export HTML archive.".to_string()),
        });
        None
    } else {
        match HtmlExportManifest::read_from_file(&manifest_path) {
            Ok(m) => {
                if m.source_fingerprint.source_digest.is_empty() {
                    findings.push(VerificationFinding {
                        code: "HTML_MANIFEST_EMPTY_SOURCE_FINGERPRINT".to_string(),
                        severity: FindingSeverity::Error,
                        category: FindingCategory::HtmlExport,
                        peer_id: None,
                        message_id: None,
                        media_id: None,
                        path: Some("manifest.json".to_string()),
                        description: "manifest.json has empty source_fingerprint digest."
                            .to_string(),
                        evidence: None,
                        recommendation: Some(
                            "Re-export HTML archive with valid source fingerprint.".to_string(),
                        ),
                    });
                }
                if m.export_config_fingerprint.is_empty() {
                    findings.push(VerificationFinding {
                        code: "HTML_MANIFEST_EMPTY_CONFIG_FINGERPRINT".to_string(),
                        severity: FindingSeverity::Error,
                        category: FindingCategory::HtmlExport,
                        peer_id: None,
                        message_id: None,
                        media_id: None,
                        path: Some("manifest.json".to_string()),
                        description: "manifest.json has empty export_config_fingerprint."
                            .to_string(),
                        evidence: None,
                        recommendation: Some("Re-export HTML archive.".to_string()),
                    });
                }
                Some(m)
            }
            Err(e) => {
                findings.push(VerificationFinding {
                    code: "HTML_MANIFEST_CORRUPTED".to_string(),
                    severity: FindingSeverity::Error,
                    category: FindingCategory::HtmlExport,
                    peer_id: None,
                    message_id: None,
                    media_id: None,
                    path: Some("manifest.json".to_string()),
                    description: format!("Failed to parse manifest.json: {e}"),
                    evidence: None,
                    recommendation: Some("Re-export HTML archive.".to_string()),
                });
                None
            }
        }
    };

    if let (Some(db_handle), Some(m)) = (db, &manifest) {
        match DatasetFingerprint::compute_from_db(db_handle) {
            Ok(computed_fp) => {
                if computed_fp.source_digest != m.source_fingerprint.source_digest {
                    findings.push(VerificationFinding {
                        code: "HTML_SOURCE_FINGERPRINT_MISMATCH".to_string(),
                        severity: FindingSeverity::Error,
                        category: FindingCategory::HtmlExport,
                        peer_id: None,
                        message_id: None,
                        media_id: None,
                        path: Some("manifest.json".to_string()),
                        description: format!(
                            "HTML export source fingerprint ({}) does not match source database ({}) - export is out of date.",
                            m.source_fingerprint.source_digest, computed_fp.source_digest
                        ),
                        evidence: Some(json!({
                            "manifest_source_fingerprint": m.source_fingerprint.source_digest,
                            "database_source_fingerprint": computed_fp.source_digest,
                            "manifest_total_messages": m.source_fingerprint.total_messages,
                            "database_total_messages": computed_fp.total_messages,
                        })),
                        recommendation: Some("Re-export HTML archive to synchronize with updated database state.".to_string()),
                    });
                }
            }
            Err(e) => {
                findings.push(VerificationFinding {
                    code: "HTML_SOURCE_FINGERPRINT_CALC_FAILED".to_string(),
                    severity: FindingSeverity::Error,
                    category: FindingCategory::HtmlExport,
                    peer_id: None,
                    message_id: None,
                    media_id: None,
                    path: Some("manifest.json".to_string()),
                    description: format!("Failed to compute source database fingerprint: {e}"),
                    evidence: None,
                    recommendation: Some("Check database integrity.".to_string()),
                });
            }
        }
    }

    let html_verifier = HtmlArchiveVerifier::new(export_dir);
    match html_verifier.verify() {
        Ok(report) => {
            for err in report.errors {
                findings.push(VerificationFinding {
                    code: "HTML_INTEGRITY_VIOLATION".to_string(),
                    severity: FindingSeverity::Error,
                    category: FindingCategory::HtmlExport,
                    peer_id: None,
                    message_id: None,
                    media_id: None,
                    path: None,
                    description: err,
                    evidence: None,
                    recommendation: Some(
                        "Re-export HTML archive to fix broken references.".to_string(),
                    ),
                });
            }
        }
        Err(e) => {
            findings.push(VerificationFinding {
                code: "HTML_VERIFICATION_EXEC_FAILED".to_string(),
                severity: FindingSeverity::Error,
                category: FindingCategory::HtmlExport,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: None,
                description: format!("Internal HTML verification failed: {e}"),
                evidence: None,
                recommendation: Some("Re-export HTML archive.".to_string()),
            });
        }
    }

    Ok(findings)
}

pub fn audit_search_index(export_dir: &Path) -> VerificationResult<Vec<VerificationFinding>> {
    let mut findings = Vec::new();
    let search_dir = export_dir.join("search");
    let manifest_path = search_dir.join("manifest.js");

    if !manifest_path.exists() {
        let export_manifest_path = export_dir.join("manifest.json");
        let is_intentionally_disabled = if export_manifest_path.exists() {
            if let Ok(m) = HtmlExportManifest::read_from_file(&export_manifest_path) {
                m.summary.search_shards_count == 0
            } else {
                false
            }
        } else {
            false
        };

        if !is_intentionally_disabled {
            findings.push(VerificationFinding {
                code: "SEARCH_MANIFEST_MISSING".to_string(),
                severity: FindingSeverity::Warning,
                category: FindingCategory::Search,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: Some("search/manifest.js".to_string()),
                description: "Search manifest.js is missing from search directory.".to_string(),
                evidence: None,
                recommendation: Some(
                    "Rebuild search index if client-side search is desired.".to_string(),
                ),
            });
        }
        return Ok(findings);
    }

    let manifest_content = match fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(e) => {
            findings.push(VerificationFinding {
                code: "SEARCH_MANIFEST_UNREADABLE".to_string(),
                severity: FindingSeverity::Error,
                category: FindingCategory::Search,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: Some("search/manifest.js".to_string()),
                description: format!("Failed to read search manifest.js: {e}"),
                evidence: None,
                recommendation: Some("Re-export search index.".to_string()),
            });
            return Ok(findings);
        }
    };

    let trimmed = manifest_content.trim();
    let prefix = "window.__VENDETTA_SEARCH_MANIFEST__ = ";
    let Some(stripped_manifest) = trimmed.strip_prefix(prefix) else {
        findings.push(VerificationFinding {
            code: "SEARCH_MANIFEST_MALFORMED_JSON".to_string(),
            severity: FindingSeverity::Error,
            category: FindingCategory::Search,
            peer_id: None,
            message_id: None,
            media_id: None,
            path: Some("search/manifest.js".to_string()),
            description: "Search manifest.js lacks window.__VENDETTA_SEARCH_MANIFEST__ wrapper."
                .to_string(),
            evidence: None,
            recommendation: Some("Re-export search index.".to_string()),
        });
        return Ok(findings);
    };

    let json_str = stripped_manifest.trim().trim_end_matches(';');

    let manifest_json: Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            findings.push(VerificationFinding {
                code: "SEARCH_MANIFEST_MALFORMED_JSON".to_string(),
                severity: FindingSeverity::Error,
                category: FindingCategory::Search,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: Some("search/manifest.js".to_string()),
                description: format!("Malformed JSON payload in search manifest.js: {e}"),
                evidence: Some(json!({ "error": e.to_string() })),
                recommendation: Some("Rebuild search index manifest.".to_string()),
            });
            return Ok(findings);
        }
    };

    let shards_array = manifest_json.get("shards").and_then(|v| v.as_array());
    let declared_shards_count = shards_array.map(|a| a.len()).unwrap_or(0);

    let mut declared_shard_files = HashSet::new();
    let mut declared_shard_ids = HashSet::new();

    let shards_dir = search_dir.join("shards");
    if let Some(shards) = shards_array {
        for shard_meta in shards {
            let shard_id = shard_meta
                .get("shard_id")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let file_name = shard_meta
                .get("file_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if !declared_shard_ids.insert(shard_id) {
                findings.push(VerificationFinding {
                    code: "SEARCH_DUPLICATE_SHARD_ID".to_string(),
                    severity: FindingSeverity::Error,
                    category: FindingCategory::Search,
                    peer_id: None,
                    message_id: None,
                    media_id: None,
                    path: Some(format!("search/shards/{file_name}")),
                    description: format!(
                        "Duplicate shard_id {shard_id} detected in search manifest."
                    ),
                    evidence: Some(json!({ "shard_id": shard_id })),
                    recommendation: Some("Rebuild search index.".to_string()),
                });
            }
            declared_shard_files.insert(file_name.clone());

            let shard_path = shards_dir.join(&file_name);
            if !shard_path.exists() {
                findings.push(VerificationFinding {
                    code: "SEARCH_SHARD_MISSING".to_string(),
                    severity: FindingSeverity::Error,
                    category: FindingCategory::Search,
                    peer_id: None,
                    message_id: None,
                    media_id: None,
                    path: Some(format!("search/shards/{file_name}")),
                    description: format!("Search shard file '{file_name}' is declared in manifest but missing on disk."),
                    evidence: Some(json!({ "shard_id": shard_id })),
                    recommendation: Some("Rebuild search index shards.".to_string()),
                });
                continue;
            }

            if let Ok(shard_content) = fs::read_to_string(&shard_path) {
                let s_trimmed = shard_content.trim();
                let s_prefix = "window.__VENDETTA_REGISTER_SEARCH_SHARD__(";
                if let Some(stripped_shard) = s_trimmed.strip_prefix(s_prefix) {
                    let inner = stripped_shard.trim().trim_end_matches([';', ')']);
                    if let Ok(shard_obj) = serde_json::from_str::<Value>(inner) {
                        let actual_entries = shard_obj
                            .get("entries")
                            .and_then(|v| v.as_array())
                            .map(|a| a.len())
                            .unwrap_or(0);
                        let declared_entries = shard_obj
                            .get("entries_count")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as usize;

                        if actual_entries != declared_entries {
                            findings.push(VerificationFinding {
                                code: "SEARCH_SHARD_ENTRIES_MISMATCH".to_string(),
                                severity: FindingSeverity::Error,
                                category: FindingCategory::Search,
                                peer_id: None,
                                message_id: None,
                                media_id: None,
                                path: Some(format!("search/shards/{file_name}")),
                                description: format!(
                                    "Shard '{file_name}' declared entries_count={declared_entries} but contains {actual_entries} items."
                                ),
                                evidence: Some(json!({
                                    "declared_count": declared_entries,
                                    "actual_count": actual_entries
                                })),
                                recommendation: Some("Rebuild search shards.".to_string()),
                            });
                        }
                    } else {
                        findings.push(VerificationFinding {
                            code: "SEARCH_SHARD_CORRUPTED_JSON".to_string(),
                            severity: FindingSeverity::Error,
                            category: FindingCategory::Search,
                            peer_id: None,
                            message_id: None,
                            media_id: None,
                            path: Some(format!("search/shards/{file_name}")),
                            description: format!(
                                "Shard '{file_name}' contains unparseable JSON payload."
                            ),
                            evidence: None,
                            recommendation: Some("Rebuild search shard.".to_string()),
                        });
                    }
                }
            }
        }
    }

    if shards_dir.exists()
        && let Ok(entries) = fs::read_dir(&shards_dir)
    {
        for entry in entries.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.starts_with("shard_")
                && file_name.ends_with(".js")
                && !declared_shard_files.contains(&file_name)
            {
                findings.push(VerificationFinding {
                    code: "SEARCH_UNDECLARED_SHARD".to_string(),
                    severity: FindingSeverity::Warning,
                    category: FindingCategory::Search,
                    peer_id: None,
                    message_id: None,
                    media_id: None,
                    path: Some(format!("search/shards/{file_name}")),
                    description: format!(
                        "Undeclared search shard file found in search directory: {file_name}"
                    ),
                    evidence: None,
                    recommendation: Some("Remove undeclared stale search shard files.".to_string()),
                });
            }
        }
    }

    if let Some(prefix_obj) = manifest_json
        .get("prefix_index")
        .and_then(|v| v.as_object())
    {
        for (prefix, target_val) in prefix_obj {
            if let Some(target_arr) = target_val.as_array() {
                for target in target_arr {
                    if let Some(target_id) = target.as_u64()
                        && !declared_shard_ids.contains(&(target_id as usize))
                    {
                        findings.push(VerificationFinding {
                            code: "SEARCH_PREFIX_INDEX_OUT_OF_BOUNDS".to_string(),
                            severity: FindingSeverity::Error,
                            category: FindingCategory::Search,
                            peer_id: None,
                            message_id: None,
                            media_id: None,
                            path: Some("search/manifest.js".to_string()),
                            description: format!(
                                "Prefix '{prefix}' points to undeclared shard_id {target_id}."
                            ),
                            evidence: Some(json!({
                                "prefix": prefix,
                                "target_shard_id": target_id,
                                "total_shards": declared_shards_count
                            })),
                            recommendation: Some("Rebuild prefix index.".to_string()),
                        });
                    }
                }
            }
        }
    }

    Ok(findings)
}
