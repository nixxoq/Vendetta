use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn sanitize_file_name(name: &str) -> String {
    let mut clean = String::with_capacity(name.len());
    let mut prev_dot = false;

    for c in name.chars() {
        match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'..='\x1f' | '\x7f' => {
                clean.push('_');
                prev_dot = false;
            }
            '.' => {
                if prev_dot {
                    if clean.ends_with('.') {
                        clean.pop();
                        clean.push('_');
                    }
                    clean.push('_');
                } else {
                    clean.push('.');
                    prev_dot = true;
                }
            }
            other => {
                clean.push(other);
                prev_dot = false;
            }
        }
    }

    let trimmed = clean.trim_matches(|c| matches!(c, '.' | ' ' | '_'));
    if trimmed.is_empty() {
        "unnamed_file".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_now_unix_secs() {
        let ts = now_unix_secs();
        assert!(ts > 1_700_000_000);
    }

    #[test]
    fn test_sanitize_file_name() {
        assert_eq!(sanitize_file_name("../../../etc/passwd"), "etc_passwd");
        assert_eq!(
            sanitize_file_name("foo/bar\\baz:qux*?.txt"),
            "foo_bar_baz_qux__.txt"
        );
        assert_eq!(sanitize_file_name("..."), "unnamed_file");
        assert_eq!(sanitize_file_name("normal_photo.jpg"), "normal_photo.jpg");
    }
}
