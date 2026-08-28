use crate::model::{RepairCategory, RepairPlan, RepairRecommendation, VerificationFinding};

pub struct RepairPlanner;

impl RepairPlanner {
    pub fn build_plan(findings: &[VerificationFinding]) -> RepairPlan {
        let mut recommendations = Vec::new();

        let mut missing_media_ids = Vec::new();
        let mut orphan_part_paths = Vec::new();
        let mut orphan_media_paths = Vec::new();
        let mut uncertain_sync_events = Vec::new();
        let mut broken_search_shards = Vec::new();
        let mut html_fingerprint_mismatch = false;
        let mut missing_html_pages = Vec::new();
        let mut blocked_channels = Vec::new();

        for f in findings {
            match f.code.as_str() {
                "MEDIA_FILE_MISSING" | "MEDIA_HASH_MISMATCH" | "MEDIA_CORRUPTED_SIZE" => {
                    if let Some(media_id) = &f.media_id {
                        missing_media_ids.push(media_id.clone());
                    }
                }
                "ORPHAN_PART_FILE" => {
                    let is_safe = f.evidence.as_ref().is_some_and(|e| {
                        e.get("no_active_lease")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                            && e.get("no_matching_media_object")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                    });

                    if let Some(path) = &f.path
                        && is_safe
                    {
                        orphan_part_paths.push(path.clone());
                    }
                }
                "ORPHAN_MEDIA_FILE" => {
                    if let Some(path) = &f.path {
                        orphan_media_paths.push(path.clone());
                    }
                }
                "SYNC_UNCERTAIN" | "UNSUPPORTED_STATE_AFFECTING_UPDATE" => {
                    uncertain_sync_events.push(f.description.clone());
                }
                "SEARCH_SHARD_MISSING" | "SEARCH_SHARD_COUNT_MISMATCH" | "SEARCH_PREFIX_BROKEN" => {
                    broken_search_shards.push(f.description.clone());
                }
                "HTML_SOURCE_FINGERPRINT_MISMATCH" => {
                    html_fingerprint_mismatch = true;
                }
                "HTML_PAGE_MISSING" | "HTML_LINK_BROKEN" => {
                    if let Some(p) = &f.path {
                        missing_html_pages.push(p.clone());
                    }
                }
                "QUEUE_CHANNEL_BLOCKED" => {
                    if let Some(pid) = f.peer_id {
                        blocked_channels.push(pid.to_string());
                    }
                }
                _ => {}
            }
        }

        if !orphan_part_paths.is_empty() {
            recommendations.push(RepairRecommendation {
                category: RepairCategory::SafeAutomation,
                action_code: "CLEANUP_ORPHAN_PART_FILES".to_string(),
                description: format!(
                    "Remove {} unreferenced .part temporary files with no active download lease.",
                    orphan_part_paths.len()
                ),
                affected_count: orphan_part_paths.len(),
                affected_items: orphan_part_paths.clone(),
                risk_level: "None".to_string(),
                suggested_command: Some("vendetta clean-orphans --parts-only".to_string()),
                why_safe_or_risky: "These temporary files are not referenced by any database record and hold no active worker claims.".to_string(),
            });
        }

        if !broken_search_shards.is_empty() {
            recommendations.push(RepairRecommendation {
                category: RepairCategory::ManualReview,
                action_code: "REBUILD_SEARCH_INDEX".to_string(),
                description: "Rebuild search index manifest and shards from current SQLite archive.".to_string(),
                affected_count: broken_search_shards.len(),
                affected_items: broken_search_shards.clone(),
                risk_level: "Low".to_string(),
                suggested_command: Some("vendetta export-html --build-search-index true --replace".to_string()),
                why_safe_or_risky: "Rebuilding search index is a read-only pass over SQLite that regenerates static shard files.".to_string(),
            });
        }

        if html_fingerprint_mismatch || !missing_html_pages.is_empty() {
            recommendations.push(RepairRecommendation {
                category: RepairCategory::ManualReview,
                action_code: "RE_EXPORT_HTML".to_string(),
                description: "Re-export static HTML archive to synchronize with updated SQLite archive state.".to_string(),
                affected_count: missing_html_pages.len().max(1),
                affected_items: missing_html_pages.clone(),
                risk_level: "Low".to_string(),
                suggested_command: Some("vendetta export-html --replace".to_string()),
                why_safe_or_risky: "Re-exporting static HTML regenerates static views without modifying database contents.".to_string(),
            });
        }

        if !orphan_media_paths.is_empty() {
            recommendations.push(RepairRecommendation {
                category: RepairCategory::ManualReview,
                action_code: "REVIEW_ORPHAN_MEDIA_FILES".to_string(),
                description: format!(
                    "Review {} media files present in storage directory but not linked to any database record.",
                    orphan_media_paths.len()
                ),
                affected_count: orphan_media_paths.len(),
                affected_items: orphan_media_paths.clone(),
                risk_level: "Medium".to_string(),
                suggested_command: None,
                why_safe_or_risky: "Files may belong to a concurrent export or manual user additions; require review before deletion.".to_string(),
            });
        }

        if !missing_media_ids.is_empty() {
            recommendations.push(RepairRecommendation {
                category: RepairCategory::RequiresTelegramResync,
                action_code: "REDOWNLOAD_MISSING_MEDIA".to_string(),
                description: format!(
                    "Re-download {} missing or corrupted media objects from Telegram MTProto servers.",
                    missing_media_ids.len()
                ),
                affected_count: missing_media_ids.len(),
                affected_items: missing_media_ids.clone(),
                risk_level: "Medium".to_string(),
                suggested_command: Some("vendetta sync --media-only".to_string()),
                why_safe_or_risky: "Requires active Telegram API connection and may trigger rate limits (FLOOD_WAIT).".to_string(),
            });
        }

        if !uncertain_sync_events.is_empty() {
            recommendations.push(RepairRecommendation {
                category: RepairCategory::RequiresTelegramResync,
                action_code: "RESYNC_UNCERTAIN_UPDATES".to_string(),
                description: "Re-synchronize difference history to resolve state uncertainty and gaps.".to_string(),
                affected_count: uncertain_sync_events.len(),
                affected_items: uncertain_sync_events.clone(),
                risk_level: "Low".to_string(),
                suggested_command: Some("vendetta sync --incremental".to_string()),
                why_safe_or_risky: "Fetches missing updates from Telegram server to restore cryptographic state certainty.".to_string(),
            });
        }

        if !blocked_channels.is_empty() {
            recommendations.push(RepairRecommendation {
                category: RepairCategory::RequiresTelegramResync,
                action_code: "RESOLVE_BLOCKED_CHANNELS".to_string(),
                description: format!(
                    "Re-authenticate or verify channel permissions for {} blocked/unresolved channels.",
                    blocked_channels.len()
                ),
                affected_count: blocked_channels.len(),
                affected_items: blocked_channels.clone(),
                risk_level: "Medium".to_string(),
                suggested_command: Some("vendetta sync --retry-blocked".to_string()),
                why_safe_or_risky: "Access permissions or chat membership must be restored on Telegram servers.".to_string(),
            });
        }

        let mut safe_automation_count = 0;
        let mut manual_review_count = 0;
        let mut requires_resync_count = 0;

        for r in &recommendations {
            match r.category {
                RepairCategory::SafeAutomation => safe_automation_count += 1,
                RepairCategory::ManualReview => manual_review_count += 1,
                RepairCategory::RequiresTelegramResync => requires_resync_count += 1,
            }
        }

        RepairPlan {
            safe_automation_count,
            manual_review_count,
            requires_resync_count,
            recommendations,
        }
    }
}
