pub mod auditors;
pub mod engine;
pub mod error;
pub mod model;
pub mod repair;
pub mod report;
pub mod status;

pub use engine::VerificationEngine;
pub use error::{VerificationError, VerificationResult};
pub use model::{
    CompletenessDimensions, DimensionDetail, FindingCategory, FindingSeverity, MediaAuditMetrics,
    OverallStatus, RepairCategory, RepairPlan, RepairRecommendation, ReplyGraphMetrics,
    VerificationFinding, VerificationMode, VerificationOptions, VerificationReport,
    VerificationSummary,
};
pub use report::{format_human_readable, format_json};
pub use status::calculate_exit_status;
