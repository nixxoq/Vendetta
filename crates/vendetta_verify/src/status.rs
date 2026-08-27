use std::cmp::Ordering;

use crate::model::{FindingCategory, OverallStatus, VerificationFinding};
#[allow(unused_imports)]
use crate::FindingSeverity;

pub fn calculate_exit_status(
    fatal_count: usize,
    error_count: usize,
    warning_count: usize,
    strict_mode: bool,
) -> i32 {
    if fatal_count > 0 {
        3
    } else if error_count > 0 || (strict_mode && warning_count > 0) {
        2
    } else if warning_count > 0 {
        1
    } else {
        0
    }
}

pub fn derive_overall_status(
    fatal_count: usize,
    error_count: usize,
    warning_count: usize,
) -> OverallStatus {
    if fatal_count > 0 {
        OverallStatus::Fatal
    } else if error_count > 0 {
        OverallStatus::Errors
    } else if warning_count > 0 {
        OverallStatus::Warnings
    } else {
        OverallStatus::Passed
    }
}

pub fn category_sort_order(cat: FindingCategory) -> u32 {
    (cat as u32) + 1
}

pub fn compare_findings(a: &VerificationFinding, b: &VerificationFinding) -> Ordering {
    b.severity
        .cmp(&a.severity)
        .then_with(|| a.category.cmp(&b.category))
        .then_with(|| a.peer_id.cmp(&b.peer_id))
        .then_with(|| a.message_id.cmp(&b.message_id))
        .then_with(|| a.media_id.cmp(&b.media_id))
        .then_with(|| a.path.cmp(&b.path))
        .then_with(|| a.code.cmp(&b.code))
        .then_with(|| a.description.cmp(&b.description))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_status_evaluates_truth_table() {
        assert_eq!(calculate_exit_status(0, 0, 0, false), 0);
        assert_eq!(calculate_exit_status(0, 0, 0, true), 0);

        assert_eq!(calculate_exit_status(0, 0, 5, false), 1);
        assert_eq!(calculate_exit_status(0, 0, 5, true), 2);

        assert_eq!(calculate_exit_status(0, 1, 0, false), 2);
        assert_eq!(calculate_exit_status(0, 1, 0, true), 2);
        assert_eq!(calculate_exit_status(0, 3, 5, false), 2);
        assert_eq!(calculate_exit_status(0, 3, 5, true), 2);

        assert_eq!(calculate_exit_status(1, 0, 0, false), 3);
        assert_eq!(calculate_exit_status(1, 0, 0, true), 3);
        assert_eq!(calculate_exit_status(2, 5, 10, false), 3);
        assert_eq!(calculate_exit_status(2, 5, 10, true), 3);
    }

    #[test]
    fn finding_sorting_is_deterministic_with_nulls() {
        let mut findings = [
            VerificationFinding {
                code: "B_ERR".to_string(),
                severity: FindingSeverity::Error,
                category: FindingCategory::Media,
                peer_id: Some(100),
                message_id: Some(5),
                media_id: None,
                path: None,
                description: "desc 2".to_string(),
                evidence: None,
                recommendation: None,
            },
            VerificationFinding {
                code: "A_FATAL".to_string(),
                severity: FindingSeverity::Fatal,
                category: FindingCategory::Schema,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: None,
                description: "fatal error".to_string(),
                evidence: None,
                recommendation: None,
            },
            VerificationFinding {
                code: "C_WARN".to_string(),
                severity: FindingSeverity::Warning,
                category: FindingCategory::SyncState,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: None,
                description: "warning".to_string(),
                evidence: None,
                recommendation: None,
            },
            VerificationFinding {
                code: "A_ERR".to_string(),
                severity: FindingSeverity::Error,
                category: FindingCategory::Identity,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: None,
                description: "desc 1".to_string(),
                evidence: None,
                recommendation: None,
            },
        ];

        findings.sort_by(compare_findings);

        assert_eq!(findings[0].code, "A_FATAL");
        assert_eq!(findings[1].code, "A_ERR");
        assert_eq!(findings[2].code, "B_ERR");
        assert_eq!(findings[3].code, "C_WARN");
    }
}
