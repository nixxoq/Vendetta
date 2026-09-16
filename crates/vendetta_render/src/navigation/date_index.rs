use std::{collections::BTreeMap, fmt::Write};

use serde::{Deserialize, Serialize};
use vendetta_model::PeerId;

use crate::{message::edits::days_to_ymd, url_builder::ArchiveUrlBuilder};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DateJumpEntry {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub page_index: usize,
}

#[derive(Debug, Clone, Default)]
pub struct DateNavigator {
    entries: BTreeMap<(i32, u32), BTreeMap<u32, String>>,
}

impl DateNavigator {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub fn record_message_date(&mut self, timestamp: i64, page_index: usize) {
        let page_file = ArchiveUrlBuilder::page_file_name(page_index);
        self.record_message_target(timestamp, page_file);
    }

    pub fn record_message_target(&mut self, timestamp: i64, target_page_file: String) {
        let days = timestamp / 86400;
        let (year, month, day) = days_to_ymd(days);
        let month_map = self.entries.entry((year, month)).or_default();
        month_map.entry(day).or_insert(target_page_file);
    }

    pub fn render_date_jump_menu(
        &self,
        _from_peer: PeerId,
        _topic_id: Option<i32>,
        _is_unified_messages_view: bool,
    ) -> String {
        self.render_date_jump_menu_for_page(None)
    }

    pub fn render_date_jump_menu_for_page(&self, from_page_rel: Option<&str>) -> String {
        if self.entries.is_empty() {
            return String::new();
        }

        let mut html = String::with_capacity(512);
        html.push_str(
            r##"<details class="date-nav-dropdown">
  <summary class="btn-icon" title="Jump to Date" aria-label="Jump to Date">
    <svg class="icon"><use href="#icon-calendar"></use></svg>
  </summary>
  <div class="date-nav-menu">
    <div class="date-nav-title">Jump to Date</div>
    <ul class="date-nav-list">
"##,
        );

        for ((year, month), days) in &self.entries {
            let month_name = month_name_abbr(*month);
            let _ = write!(
                html,
                "      <li class=\"date-year-group\">\n        <div class=\"date-month-heading\">{year} {month_name}</div>\n        <div class=\"date-days-grid\">"
            );
            for (day, page_file) in days {
                let href = if let Some(from_rel) = from_page_rel {
                    ArchiveUrlBuilder::relative_day_to_day(from_rel, page_file)
                } else {
                    page_file.clone()
                };
                let _ = write!(
                    html,
                    "<a href=\"{href}#d-{year:04}-{month:02}-{day:02}\" class=\"date-jump-btn\">{day}</a>"
                );
            }
            html.push_str("</div>\n      </li>\n");
        }

        html.push_str("    </ul>\n  </div>\n</details>\n");
        html
    }
}

fn month_name_abbr(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "???",
    }
}
