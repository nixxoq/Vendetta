use std::{collections::HashSet, path::Path};

use vendetta_model::PeerType;

use crate::model::ImportChat;

#[derive(Debug, Default)]
pub struct SelfIdentityResolver {
    pub explicit_self_name: Option<String>,
    pub explicit_self_user_id: Option<i64>,
}

impl SelfIdentityResolver {
    pub fn new(source_path: &Path) -> Self {
        let (name, uid) = extract_explicit_tdesktop_self(source_path);
        Self {
            explicit_self_name: name,
            explicit_self_user_id: uid,
        }
    }

    pub fn resolve_and_apply(&self, chats: &mut [ImportChat]) {
        let resolved_self_name = self
            .explicit_self_name
            .clone()
            .or_else(|| discover_self_name_from_private_chats(chats));

        for chat in chats {
            let is_personal = chat.peer_type == PeerType::User;
            let contact_name = chat.name.as_deref().map(str::trim);

            for msg in &mut chat.messages {
                // 1. Explicit self user_id match
                if let Some(uid) = self.explicit_self_user_id
                    && let Some(sid) = msg.sender_id
                    && sid.raw() == uid
                {
                    msg.is_outgoing = true;
                    continue;
                }

                // 2. Resolved self name match
                if let Some(ref self_name) = resolved_self_name {
                    if let Some(s) = msg.sender_name.as_deref().map(base_sender_name) {
                        if s.eq_ignore_ascii_case(self_name) {
                            msg.is_outgoing = true;
                            continue;
                        } else if let Some(c) = contact_name
                            && s.eq_ignore_ascii_case(c)
                        {
                            msg.is_outgoing = false;
                            continue;
                        }
                    }

                    // Check service message prefix
                    if let Some(ref text) = msg.text {
                        let trimmed = text.trim();
                        if trimmed.starts_with(self_name.as_str()) {
                            msg.is_outgoing = true;
                            adapt_service_photo_text(msg, contact_name, true);
                            continue;
                        } else if let Some(c) = contact_name
                            && trimmed.starts_with(c)
                        {
                            msg.is_outgoing = false;
                            adapt_service_photo_text(msg, contact_name, false);
                            continue;
                        }
                    }
                }

                // 3. 1-on-1 private chat fallback
                if is_personal {
                    if let Some(s) = msg.sender_name.as_deref().map(base_sender_name)
                        && let Some(c) = contact_name
                    {
                        if !s.eq_ignore_ascii_case(c) {
                            msg.is_outgoing = true;
                            continue;
                        } else {
                            msg.is_outgoing = false;
                            continue;
                        }
                    }

                    if let Some(ref text) = msg.text {
                        let trimmed = text.trim();
                        if let Some(c) = contact_name
                            && trimmed.contains("suggests to use this photo")
                        {
                            if !trimmed.starts_with(c) {
                                msg.is_outgoing = true;
                                adapt_service_photo_text(msg, contact_name, true);
                                continue;
                            } else {
                                msg.is_outgoing = false;
                                adapt_service_photo_text(msg, contact_name, false);
                                continue;
                            }
                        }
                    }
                }

                // Safe fallback: default to false (incoming / neutral)
                msg.is_outgoing = false;
            }
        }
    }
}

fn base_sender_name(name: &str) -> &str {
    if let Some((base, _)) = name.split_once(" via @") {
        base.trim()
    } else {
        name.trim()
    }
}

fn adapt_service_photo_text(
    msg: &mut crate::model::ImportMessage,
    contact_name: Option<&str>,
    is_out: bool,
) {
    if let Some(ref text) = msg.text
        && text.contains("suggests to use this photo")
    {
        let new_text = if is_out {
            format!(
                "You suggested this photo for {}'s Telegram profile.",
                contact_name.unwrap_or("contact")
            )
        } else {
            let sender = msg
                .sender_name
                .as_deref()
                .or(contact_name)
                .unwrap_or("Contact");
            format!("{sender} suggested this photo for your Telegram profile.")
        };
        msg.text = Some(new_text.clone());
        msg.service_event = Some(crate::model::ImportServiceEvent::ActionText(new_text));
    }
}

fn discover_self_name_from_private_chats(chats: &[ImportChat]) -> Option<String> {
    let private_chats: Vec<&ImportChat> = chats
        .iter()
        .filter(|c| c.peer_type == PeerType::User && c.name.is_some())
        .collect();

    if private_chats.is_empty() {
        return None;
    }

    let mut non_contact_sets: Vec<HashSet<String>> = Vec::new();

    for chat in &private_chats {
        let contact = chat.name.as_deref().unwrap().trim().to_lowercase();
        let mut chat_senders = HashSet::new();

        for msg in &chat.messages {
            if let Some(ref name) = msg.sender_name {
                let base = base_sender_name(name);
                let trimmed = base.to_lowercase();
                if trimmed != contact && !trimmed.is_empty() {
                    chat_senders.insert(base.to_string());
                }
            }
            if let Some(ref text) = msg.text
                && let Some(actor) = text.strip_suffix("suggests to use this photo")
            {
                let base = base_sender_name(actor);
                let actor_trimmed = base.to_lowercase();
                if actor_trimmed != contact && !actor_trimmed.is_empty() {
                    chat_senders.insert(base.to_string());
                }
            }
        }

        if !chat_senders.is_empty() {
            non_contact_sets.push(chat_senders);
        }
    }

    if non_contact_sets.is_empty() {
        return None;
    }

    let mut intersection: HashSet<String> = non_contact_sets[0].clone();
    for set in &non_contact_sets[1..] {
        intersection.retain(|name| set.iter().any(|s| s.eq_ignore_ascii_case(name)));
    }

    if intersection.len() == 1 {
        return intersection.into_iter().next();
    }

    if non_contact_sets.len() == 1 && non_contact_sets[0].len() == 1 {
        return non_contact_sets[0].iter().next().cloned();
    }

    None
}

fn extract_explicit_tdesktop_self(source_path: &Path) -> (Option<String>, Option<i64>) {
    let json_path = if source_path.is_file() {
        source_path.to_path_buf()
    } else {
        source_path.join("result.json")
    };

    if json_path.is_file()
        && let Ok(content) = std::fs::read_to_string(&json_path)
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
        && let Some(pinfo) = val.get("personal_information")
    {
        let uid = pinfo.get("user_id").and_then(|u| u.as_i64());
        let first = pinfo
            .get("first_name")
            .and_then(|f| f.as_str())
            .unwrap_or("");
        let last = pinfo
            .get("last_name")
            .and_then(|l| l.as_str())
            .unwrap_or("");
        let full = format!("{first} {last}").trim().to_string();
        let name = if full.is_empty() { None } else { Some(full) };
        return (name, uid);
    }

    let export_results = if source_path.is_dir() {
        source_path.join("export_results.html")
    } else {
        source_path
            .parent()
            .map(|p| p.join("export_results.html"))
            .unwrap_or_default()
    };

    if export_results.is_file()
        && let Ok(content) = std::fs::read_to_string(&export_results)
    {
        let doc = scraper::Html::parse_document(&content);
        if let Ok(sel) = scraper::Selector::parse(".user_name, .profile .name")
            && let Some(el) = doc.select(&sel).next()
        {
            let text = el.text().collect::<String>().trim().to_string();
            if !text.is_empty() {
                return (Some(text), None);
            }
        }
    }

    (None, None)
}
