use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use crate::model::{FindingCategory, FindingSeverity, VerificationFinding};

pub fn audit_relative_path(
    root: &Path,
    rel_str: &str,
    category: FindingCategory,
) -> Result<PathBuf, Box<VerificationFinding>> {
    if rel_str.contains('\0') {
        return Err(Box::new(VerificationFinding {
            code: "PATH_CONTAINS_NULL_BYTE".to_string(),
            severity: FindingSeverity::Error,
            category,
            peer_id: None,
            message_id: None,
            media_id: None,
            path: Some(rel_str.to_string()),
            description: format!("Path contains dangerous null byte: {rel_str}"),
            evidence: None,
            recommendation: Some("Sanitize path to remove illegal characters.".to_string()),
        }));
    }

    let p = Path::new(rel_str);

    if p.is_absolute() {
        return Err(Box::new(VerificationFinding {
            code: "PATH_ABSOLUTE_ESCAPE".to_string(),
            severity: FindingSeverity::Error,
            category,
            peer_id: None,
            message_id: None,
            media_id: None,
            path: Some(rel_str.to_string()),
            description: format!("Absolute path is not permitted in relative storage: {rel_str}"),
            evidence: None,
            recommendation: Some(
                "Convert path to a relative path within archive root.".to_string(),
            ),
        }));
    }

    if rel_str.starts_with('\\') || (rel_str.as_bytes().get(1) == Some(&b':')) {
        return Err(Box::new(VerificationFinding {
            code: "PATH_WINDOWS_DRIVE_ESCAPE".to_string(),
            severity: FindingSeverity::Error,
            category,
            peer_id: None,
            message_id: None,
            media_id: None,
            path: Some(rel_str.to_string()),
            description: format!("Windows drive letter or UNC prefix is not permitted: {rel_str}"),
            evidence: None,
            recommendation: Some("Use standard relative path separators.".to_string()),
        }));
    }

    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                return Err(Box::new(VerificationFinding {
                    code: "PATH_TRAVERSAL_ESCAPE".to_string(),
                    severity: FindingSeverity::Error,
                    category,
                    peer_id: None,
                    message_id: None,
                    media_id: None,
                    path: Some(rel_str.to_string()),
                    description: format!("Path traversal '..' component detected: {rel_str}"),
                    evidence: None,
                    recommendation: Some("Remove parent directory traversals.".to_string()),
                }));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(Box::new(VerificationFinding {
                    code: "PATH_ROOT_ESCAPE".to_string(),
                    severity: FindingSeverity::Error,
                    category,
                    peer_id: None,
                    message_id: None,
                    media_id: None,
                    path: Some(rel_str.to_string()),
                    description: format!("Path root component detected: {rel_str}"),
                    evidence: None,
                    recommendation: Some("Use relative path components only.".to_string()),
                }));
            }
            _ => {}
        }
    }

    let full_path = root.join(p);

    if full_path.exists()
        && let Ok(target) = fs::read_link(&full_path)
    {
        let resolved_target = if target.is_relative() {
            full_path.parent().unwrap_or(root).join(target)
        } else {
            target
        };

        if let (Ok(canonical_target), Ok(canonical_root)) =
            (resolved_target.canonicalize(), root.canonicalize())
            && !canonical_target.starts_with(&canonical_root)
        {
            return Err(Box::new(VerificationFinding {
                code: "SYMLINK_ESCAPE_DETECTED".to_string(),
                severity: FindingSeverity::Error,
                category,
                peer_id: None,
                message_id: None,
                media_id: None,
                path: Some(rel_str.to_string()),
                description: format!(
                    "Symlink at {} points outside archive root: {}",
                    full_path.display(),
                    canonical_target.display()
                ),
                evidence: None,
                recommendation: Some(
                    "Replace escaping symlink with a local file or relative link.".to_string(),
                ),
            }));
        }
    }

    Ok(full_path)
}
