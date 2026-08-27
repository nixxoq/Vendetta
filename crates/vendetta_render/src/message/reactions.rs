use std::fmt::Write;

use crate::{
    entity::html_escape,
    model::{RenderReactionGroup, RenderReactionKey},
};

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
                format!("<span class=\"reaction-emoji\">{}</span>", html_escape(s))
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
            RenderReactionKey::Paid => "<span class=\"reaction-emoji\">⭐</span>".to_string(),
        };

        let count = group.count;
        let suffix = if count == 1 { "reaction" } else { "reactions" };

        let _ = writeln!(
            html,
            "      <div class=\"reaction-badge{chosen_class}\" tabindex=\"0\" role=\"button\" aria-haspopup=\"true\">"
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
