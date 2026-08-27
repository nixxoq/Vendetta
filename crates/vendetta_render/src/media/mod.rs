pub mod album;
pub mod card;
pub mod placeholder;

pub use album::render_album_gallery;
pub use card::render_media_card;
pub use placeholder::{format_file_size, render_unavailable_media};

use std::path::{Component, Path, PathBuf};

use crate::error::{RenderError, RenderResult};

pub fn validate_and_clean_media_rel_path(raw_rel_path: &str) -> RenderResult<PathBuf> {
    if raw_rel_path.is_empty() {
        return Err(RenderError::UnsafePath("Empty media path".to_string()));
    }

    if raw_rel_path.starts_with('/') || raw_rel_path.starts_with('\\') {
        return Err(RenderError::UnsafePath(format!(
            "Absolute path detected in media path: {raw_rel_path}"
        )));
    }

    let stripped = raw_rel_path
        .strip_prefix("media/")
        .or_else(|| raw_rel_path.strip_prefix("media\\"))
        .unwrap_or(raw_rel_path);

    let path = Path::new(stripped);
    let mut result = PathBuf::new();

    for comp in path.components() {
        match comp {
            Component::Normal(os_str) => {
                let s = os_str.to_str().ok_or_else(|| {
                    RenderError::UnsafePath(format!("Invalid UTF-8 in media path: {raw_rel_path}"))
                })?;

                if s.contains([':', '\0', '\\']) || s.contains("..") {
                    return Err(RenderError::UnsafePath(format!(
                        "Forbidden component in media path: {raw_rel_path}"
                    )));
                }
                result.push(s);
            }
            Component::ParentDir => {
                return Err(RenderError::UnsafePath(format!(
                    "Path traversal '..' detected in media path: {raw_rel_path}"
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(RenderError::UnsafePath(format!(
                    "Absolute path prefix detected in media path: {raw_rel_path}"
                )));
            }
            Component::CurDir => {}
        }
    }

    if result.as_os_str().is_empty() {
        return Err(RenderError::UnsafePath(format!(
            "Normalized media path is empty: {raw_rel_path}"
        )));
    }

    Ok(result)
}
