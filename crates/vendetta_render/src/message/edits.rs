use std::fmt::Write;

use crate::{entity::html_escape, model::RenderRevision};

pub fn render_edit_history(revisions: &[RenderRevision]) -> String {
    if revisions.is_empty() {
        return String::new();
    }

    let rev_count = revisions.len();
    let suffix = if rev_count == 1 {
        "revision"
    } else {
        "revisions"
    };

    let mut html = String::with_capacity(revisions.len() * 256 + 128);
    let _ = writeln!(
        html,
        "<details class=\"edit-history\">\n  <summary><span class=\"badge-edited\">Edited</span> <span class=\"edit-count\">Edited • {rev_count} {suffix}</span></summary>\n  <div class=\"revision-timeline\">"
    );

    for (idx, rev) in revisions.iter().enumerate() {
        let rev_num = idx + 1;
        let date_str = chrono_like_format(rev.edit_date.unwrap_or(rev.captured_at));

        let _ = writeln!(html, "    <div class=\"revision-entry\">");
        let _ = writeln!(
            html,
            "      <div class=\"revision-meta\"><span class=\"rev-badge\">v{rev_num}</span> <time>{date_str}</time></div>\n      <div class=\"revision-content\">"
        );

        if !rev.formatted_html.is_empty() {
            let _ = writeln!(html, "        {}", rev.formatted_html);
        } else if let Some(raw) = &rev.raw_text {
            let _ = writeln!(html, "        {}", html_escape(raw).replace('\n', "<br>"));
        } else {
            html.push_str("        <em class=\"text-muted\">[Empty message text]</em>\n");
        }

        html.push_str("      </div>\n    </div>\n");
    }

    html.push_str("  </div>\n</details>\n");
    html
}

pub fn chrono_like_format(ts: i64) -> String {
    let seconds_in_day = 86400;
    let days = ts / seconds_in_day;
    let rem_seconds = (ts % seconds_in_day).unsigned_abs();
    let hours = rem_seconds / 3600;
    let minutes = (rem_seconds % 3600) / 60;
    let seconds = rem_seconds % 60;

    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02}:{seconds:02} UTC")
}

pub fn days_to_ymd(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}
