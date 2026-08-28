use std::fmt::Write;

use crate::{
    entity::html_escape,
    model::{RenderReactionGroup, RenderReactionKey},
};

pub fn normalize_reaction_emoji(emoticon: &str) -> String {
    let mut result = String::with_capacity(emoticon.len() + 8);
    let chars: Vec<char> = emoticon.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        result.push(c);
        if (c == '\u{2764}'
            || c == '\u{263a}'
            || c == '\u{2639}'
            || c == '\u{2600}'
            || c == '\u{270c}'
            || c == '\u{270b}'
            || c == '\u{270d}'
            || c == '\u{2763}'
            || c == '\u{26a1}'
            || c == '\u{2601}'
            || c == '\u{2614}'
            || c == '\u{2615}'
            || c == '\u{2753}'
            || c == '\u{2757}')
            && (i + 1 >= chars.len() || chars[i + 1] != '\u{fe0f}')
        {
            result.push('\u{fe0f}');
        }
        i += 1;
    }
    result
}

pub fn render_message_reactions(reactions: &[RenderReactionGroup]) -> String {
    if reactions.is_empty() {
        return String::new();
    }

    let mut html = String::with_capacity(reactions.len() * 512 + 64);
    html.push_str("    <div class=\"message-reactions\">\n");

    for group in reactions {
        let chosen_class = if group.is_chosen_by_me {
            " reaction-chosen"
        } else {
            ""
        };

        let icon_html = match &group.reaction {
            RenderReactionKey::Emoji(s) | RenderReactionKey::Unknown(s) => {
                let normalized = normalize_reaction_emoji(s);
                format!(
                    "<span class=\"reaction-emoji\">{}</span>",
                    html_escape(&normalized)
                )
            }
            RenderReactionKey::CustomEmoji {
                document_id,
                alt_text,
                asset_rel_path,
            } => {
                if let Some(path) = asset_rel_path {
                    let alt = alt_text.as_deref().unwrap_or("Custom Reaction");
                    format!(
                        "<img src=\"{}\" alt=\"{}\" class=\"reaction-custom-icon\" loading=\"lazy\">",
                        html_escape(path),
                        html_escape(alt)
                    )
                } else if let Some(alt) = alt_text {
                    format!(
                        "<span class=\"reaction-custom-fallback\">{}</span>",
                        html_escape(alt)
                    )
                } else {
                    format!(
                        "<span class=\"reaction-custom-fallback\" title=\"Custom reaction #{document_id}\">✨</span>"
                    )
                }
            }
            RenderReactionKey::Paid => {
                "<span class=\"reaction-emoji\" title=\"Telegram Star\">⭐</span>".to_string()
            }
        };

        let count = group.count;
        let suffix = if count == 1 { "reaction" } else { "reactions" };

        let aria_label = match &group.reaction {
            RenderReactionKey::Paid => format!("⭐ {count} {suffix}"),
            RenderReactionKey::Emoji(s) | RenderReactionKey::Unknown(s) => {
                format!("{s} {count} {suffix}")
            }
            RenderReactionKey::CustomEmoji { alt_text, .. } => {
                let alt = alt_text.as_deref().unwrap_or("Custom Reaction");
                format!("{alt} {count} {suffix}")
            }
        };

        let _ = writeln!(
            html,
            "      <div class=\"reaction-badge{chosen_class}\" tabindex=\"0\" role=\"button\" aria-haspopup=\"true\" aria-label=\"{}\">",
            html_escape(&aria_label)
        );
        let _ = writeln!(
            html,
            "        <span class=\"reaction-icon\">{icon_html}</span>"
        );
        let _ = writeln!(
            html,
            "        <span class=\"reaction-count\">{count}</span>"
        );
        html.push_str("        <div class=\"reaction-popover\" role=\"tooltip\">\n          <div class=\"reaction-popover-header\">\n");
        let _ = writeln!(
            html,
            "            <span class=\"reaction-popover-icon\">{icon_html}</span>"
        );
        let _ = writeln!(
            html,
            "            <span class=\"reaction-popover-title\">{count} {suffix}</span>"
        );
        html.push_str("          </div>\n");

        if group.reactors.is_empty() {
            html.push_str("          <div class=\"reaction-popover-empty\">Reactor details unavailable in archive</div>\n");
        } else {
            html.push_str("          <ul class=\"reaction-reactor-list\">\n");
            for reactor in &group.reactors {
                html.push_str("            <li class=\"reaction-reactor-item\">\n");
                if let Some(av) = &reactor.avatar_markup {
                    let _ = writeln!(
                        html,
                        "              <div class=\"reactor-avatar\">{av}</div>"
                    );
                } else {
                    html.push_str("              <div class=\"reactor-avatar\"><div class=\"avatar avatar-placeholder\"></div></div>\n");
                }
                html.push_str("              <div class=\"reactor-info\">\n");
                let _ = writeln!(
                    html,
                    "                <span class=\"reactor-name\">{}</span>",
                    html_escape(&reactor.name)
                );
                if let Some(un) = &reactor.username {
                    let _ = writeln!(
                        html,
                        "                <span class=\"reactor-username\">@{}</span>",
                        html_escape(un)
                    );
                }
                html.push_str("              </div>\n            </li>\n");
            }

            if group.count > group.reactors.len() {
                let remaining = group.count - group.reactors.len();
                let _ = writeln!(
                    html,
                    "            <li class=\"reaction-reactor-more\">+ {remaining} more</li>"
                );
            }
            html.push_str("          </ul>\n");
        }

        html.push_str("        </div>\n      </div>\n");
    }

    html.push_str("    </div>\n");
    html
}
