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

pub fn slugify_chat_title(title: &str) -> String {
    let mut clean = String::with_capacity(title.len());
    let mut prev_underscore = false;

    for c in title.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                clean.push(lc);
            }
            prev_underscore = false;
        } else if !prev_underscore {
            clean.push('_');
            prev_underscore = true;
        }
    }

    let trimmed = clean.trim_matches('_');
    if trimmed.is_empty() {
        "chat".to_string()
    } else if trimmed.chars().count() > 64 {
        let truncated: String = trimmed.chars().take(64).collect();
        truncated.trim_end_matches('_').to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn ymd_to_days(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + (d as i64) - 1;
    let doe = (yoe as i64) * 365 + (yoe as i64) / 4 - (yoe as i64) / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn parse_date_bound(s: &str, is_end: bool) -> std::result::Result<i64, String> {
    let s = s.trim();
    if s.len() == 10 && s.chars().nth(4) == Some('-') && s.chars().nth(7) == Some('-') {
        let y: i32 = s[0..4]
            .parse()
            .map_err(|_| format!("Invalid year in date: '{s}'"))?;
        let m: u32 = s[5..7]
            .parse()
            .map_err(|_| format!("Invalid month in date: '{s}'"))?;
        let d: u32 = s[8..10]
            .parse()
            .map_err(|_| format!("Invalid day in date: '{s}'"))?;
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return Err(format!("Date out of range: '{s}'"));
        }
        let days = ymd_to_days(y, m, d);
        let secs = days * 86400;
        return Ok(if is_end { secs + 86399 } else { secs });
    }

    let normalized = s.replace(' ', "T");
    let (date_part, time_part) = normalized
        .split_once('T')
        .ok_or_else(|| format!("Invalid date format: '{s}'. Expected YYYY-MM-DD or RFC3339"))?;

    let [y_str, m_str, d_str] = match date_part.split('-').collect::<Vec<_>>().as_slice() {
        [y, m, d] => [*y, *m, *d],
        _ => return Err(format!("Invalid date part in '{s}'")),
    };
    let y: i32 = y_str.parse().map_err(|_| format!("Invalid year: '{s}'"))?;
    let m: u32 = m_str.parse().map_err(|_| format!("Invalid month: '{s}'"))?;
    let d: u32 = d_str.parse().map_err(|_| format!("Invalid day: '{s}'"))?;
    let days = ymd_to_days(y, m, d);

    let clean_time = time_part.trim_end_matches('Z');
    let (h, min, sec) = match clean_time.split(':').collect::<Vec<_>>().as_slice() {
        [h_str, min_str] => {
            let h: i64 = h_str.parse().map_err(|_| format!("Invalid hour in '{s}'"))?;
            let min: i64 = min_str.parse().map_err(|_| format!("Invalid minute in '{s}'"))?;
            (h, min, 0)
        }
        [h_str, min_str, sec_part, ..] => {
            let h: i64 = h_str.parse().map_err(|_| format!("Invalid hour in '{s}'"))?;
            let min: i64 = min_str.parse().map_err(|_| format!("Invalid minute in '{s}'"))?;
            let sec: i64 = sec_part
                .split('.')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            (h, min, sec)
        }
        _ => return Err(format!("Invalid time part in '{s}'")),
    };

    Ok(days * 86400 + h * 3600 + min * 60 + sec)
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

    #[test]
    fn test_slugify_chat_title() {
        assert_eq!(slugify_chat_title("Rust Architecture Group"), "rust_architecture_group");
        assert_eq!(slugify_chat_title("My Chat / Topic: Test!"), "my_chat_topic_test");
        assert_eq!(slugify_chat_title("🔥🚀"), "chat");
        assert_eq!(slugify_chat_title("   "), "chat");
        assert_eq!(slugify_chat_title("Привет Мир 123"), "привет_мир_123");
    }

    #[test]
    fn test_ymd_and_date_parsing() {
        assert_eq!(ymd_to_days(1970, 1, 1), 0);
        assert_eq!(ymd_to_days(2026, 9, 15), 20711);

        let start = parse_date_bound("2026-09-15", false).unwrap();
        assert_eq!(start, 20711 * 86400);

        let end = parse_date_bound("2026-09-15", true).unwrap();
        assert_eq!(end, 20711 * 86400 + 86399);

        let dt = parse_date_bound("2026-09-15T12:30:00Z", false).unwrap();
        assert_eq!(dt, 20711 * 86400 + 12 * 3600 + 30 * 60);
    }
}
