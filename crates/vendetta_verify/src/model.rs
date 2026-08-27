use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString, VariantArray};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FindingSeverity {
    Info = 0,
    Warning = 1,
    Error = 2,
    Fatal = 3,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FindingCategory {
    Schema,
    ReferentialIntegrity,
    Identity,
    Chronology,
    MessageState,
    ReplyGraph,
    Revision,
    Entity,
    ServiceMessage,
    Media,
    Filesystem,
    SyncState,
    UnsupportedUpdate,
    Queue,
    Migration,
    Search,
    HtmlExport,
    Manifest,
    Security,
    Completeness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationFinding {
    pub code: String,
    pub severity: FindingSeverity,
    pub category: FindingCategory,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<String>,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Default,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum VerificationMode {
    Fast,
    #[default]
    Full,
}

#[derive(Debug, Clone)]
pub struct VerificationOptions {
    pub archive_path: Option<PathBuf>,
    pub html_dir: Option<PathBuf>,
    pub media_dir: Option<PathBuf>,
    pub mode: VerificationMode,
    pub scope_media: bool,
    pub scope_replies: bool,
    pub scope_search: bool,
    pub rehash_media: bool,
    pub strict: bool,
}

impl Default for VerificationOptions {
    fn default() -> Self {
        Self {
            archive_path: None,
            html_dir: None,
            media_dir: None,
            mode: VerificationMode::Full,
            scope_media: false,
            scope_replies: false,
            scope_search: false,
            rehash_media: false,
            strict: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DimensionDetail {
    pub status: String,
    pub reason: String,
    pub affected_count: usize,
    pub evidence_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CompletenessDimensions {
    pub message_history: DimensionDetail,
    pub deletion_verification: DimensionDetail,
    pub media_binaries: DimensionDetail,
    pub channel_discovery: DimensionDetail,
    pub sync_uncertainty: DimensionDetail,
    pub search_index: DimensionDetail,
    pub html_export: DimensionDetail,
}

impl Default for DimensionDetail {
    fn default() -> Self {
        Self {
            status: "not_applicable".to_string(),
            reason: String::new(),
            affected_count: 0,
            evidence_codes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReplyGraphMetrics {
    pub total_replies: usize,
    pub resolved: usize,
    pub unavailable: usize,
    pub out_of_scope: usize,
    pub missing: usize,
    pub cross_peer: usize,
    pub self_cycles: usize,
    pub cycles: usize,
    pub depth_exceeded: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MediaAuditMetrics {
    pub total_media_records: usize,
    pub completed_verified_on_disk: usize,
    pub missing_files: usize,
    pub size_mismatches: usize,
    pub hash_mismatches: usize,
    pub part_files_checked: usize,
    pub orphan_media_files: usize,
    pub total_bytes_verified: u64,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum OverallStatus {
    Passed,
    Warnings,
    Errors,
    Fatal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionScope {
    pub mode: String,
    pub media_scope: bool,
    pub media_scope_implicit_via_rehash: bool,
    pub replies_scope: bool,
    pub search_scope_requested: bool,
    pub search_scope_executed: bool,
    pub search_scope: bool,
    pub html_scope: bool,
    pub rehash: bool,
    pub core_db_auditors_executed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationSummary {
    pub status: OverallStatus,
    pub exit_code: i32,
    pub total_findings: usize,
    pub fatal_count: usize,
    pub error_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    pub scope: ExecutionScope,
    pub category_counts: BTreeMap<FindingCategory, usize>,
    pub dimensions: CompletenessDimensions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_metrics: Option<ReplyGraphMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_metrics: Option<MediaAuditMetrics>,
    pub duration_ms: u64,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RepairCategory {
    SafeAutomation,
    ManualReview,
    RequiresTelegramResync,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairRecommendation {
    pub category: RepairCategory,
    pub action_code: String,
    pub description: String,
    pub affected_count: usize,
    pub affected_items: Vec<String>,
    pub risk_level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_command: Option<String>,
    pub why_safe_or_risky: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RepairPlan {
    pub safe_automation_count: usize,
    pub manual_review_count: usize,
    pub requires_resync_count: usize,
    pub recommendations: Vec<RepairRecommendation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationReport {
    pub schema_version: u32,
    pub summary: VerificationSummary,
    pub findings: Vec<VerificationFinding>,
    pub repair_plan: RepairPlan,
}
