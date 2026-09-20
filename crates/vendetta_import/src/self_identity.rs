use std::{collections::HashSet, path::Path};

use vendetta_model::PeerType;

use crate::model::{ImportChat, ImportMessage, ImportServiceEvent};

#[derive(Debug, Default)]
pub struct SelfIdentityResolver {
    pub explicit_self_name: Option<String>,
    pub explicit_self_user_id: Option<i64>,
}

impl SelfIdentityResolver {
    pub fn new(source_path: &Path) -> Self {
        let (name, uid) = extract_tdesktop_self(source_path);
        Self {
            explicit_self_name: name,
            explicit_self_user_id: uid,
        }
    }

    pub fn resolve_and_apply(&self, chats: &mut [ImportChat]) {
        let resolved_self_name = self
            .explicit_self_name
            .clone()
            .or_else(|| self_name(chats));

        for chat in chats {
            let is_personal = chat.peer_type == PeerType::User;
            let contact_name = chat.name.as_deref().map(str::trim);

            for msg in &mut chat.messages {
                let is_out = message_outgoing(
                    msg,
                    self.explicit_self_user_id,
                    resolved_self_name.as_deref(),
                    contact_name,
                    is_personal,
                );

                msg.is_outgoing = is_out;
                service_photo(msg, contact_name, is_out);
            }
        }
    }
}

fn message_outgoing(
    msg: &ImportMessage,
    explicit_uid: Option<i64>,
    self_name: Option<&str>,
    contact_name: Option<&str>,
    is_personal: bool,
) -> bool {
    if let Some(uid) = explicit_uid
        && let Some(sid) = msg.sender_id
        && sid.raw() == uid
    {
        return true;
    }

    let sender = msg.sender_name.as_deref().map(base_sender_name);
    let text_trimmed = msg.text.as_deref().map(str::trim);

    if let Some(self_name) = self_name {
        if let Some(s) = sender {
            if s.eq_ignore_ascii_case(self_name) {
                return true;
            }
            if let Some(c) = contact_name
                && s.eq_ignore_ascii_case(c)
            {
                return false;
            }
        }

        if let Some(text) = text_trimmed {
            if text.starts_with(self_name) {
                return true;
            }
            if let Some(c) = contact_name
                && text.starts_with(c)
            {
                return false;
            }
        }
    }

    if is_personal {
        if let Some(s) = sender
            && let Some(c) = contact_name
        {
            return !s.eq_ignore_ascii_case(c);
        }

        if let Some(text) = text_trimmed
            && let Some(c) = contact_name
            && text.contains("suggests to use this photo")
        {
            return !text.starts_with(c);
        }
    }

    false
}

fn base_sender_name(name: &str) -> &str {
    name.split_once(" via @")
        .map_or(name, |(base, _)| base)
        .trim()
}

fn service_photo(msg: &mut ImportMessage, contact_name: Option<&str>, is_out: bool) {
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
        msg.service_event = Some(ImportServiceEvent::ActionText(new_text));
    }
}

fn self_name(chats: &[ImportChat]) -> Option<String> {
    let non_contact_sets: Vec<HashSet<String>> = chats
        .iter()
        .filter(|c| c.peer_type == PeerType::User)
        .filter_map(|chat| {
            let contact = chat.name.as_deref()?.trim().to_lowercase();
            let senders: HashSet<String> = chat
                .messages
                .iter()
                .flat_map(|msg| {
                    let from_name = msg.sender_name.as_deref();
                    let from_service = msg
                        .text
                        .as_deref()
                        .and_then(|t| t.strip_suffix("suggests to use this photo"));
                    from_name.into_iter().chain(from_service)
                })
                .map(base_sender_name)
                .filter(|base| {
                    let lower = base.to_lowercase();
                    !lower.is_empty() && lower != contact
                })
                .map(ToString::to_string)
                .collect();

            (!senders.is_empty()).then_some(senders)
        })
        .collect();

    let mut sets_iter = non_contact_sets.into_iter();
    let first = sets_iter.next()?;

    let intersection = sets_iter.fold(first, |mut acc, set| {
        acc.retain(|name| set.iter().any(|s| s.eq_ignore_ascii_case(name)));
        acc
    });

    (intersection.len() == 1)
        .then(|| intersection.into_iter().next())
        .flatten()
}

fn extract_tdesktop_self(source_path: &Path) -> (Option<String>, Option<i64>) {
    extract_from_json(source_path).unwrap_or_else(|| (extract_from_html(source_path), None))
}

fn extract_from_json(source_path: &Path) -> Option<(Option<String>, Option<i64>)> {
    let json_path = if source_path.is_file() {
        source_path.to_path_buf()
    } else {
        source_path.join("result.json")
    };

    let content = std::fs::read_to_string(json_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    let pinfo = val.get("personal_information")?;

    let uid = pinfo.get("user_id").and_then(|u| u.as_i64());
    let first = pinfo
        .get("first_name")
        .and_then(|f| f.as_str())
        .unwrap_or_default();
    let last = pinfo
        .get("last_name")
        .and_then(|l| l.as_str())
        .unwrap_or_default();

    let full = format!("{first} {last}").trim().to_string();
    let name = (!full.is_empty()).then_some(full);

    Some((name, uid))
}

fn extract_from_html(source_path: &Path) -> Option<String> {
    let export_results = if source_path.is_dir() {
        source_path.join("export_results.html")
    } else {
        source_path.parent()?.join("export_results.html")
    };

    let content = std::fs::read_to_string(export_results).ok()?;
    let doc = scraper::Html::parse_document(&content);
    let sel = scraper::Selector::parse(".user_name, .profile .name").ok()?;
    let el = doc.select(&sel).next()?;

    let text = el.text().collect::<String>().trim().to_string();
    (!text.is_empty()).then_some(text)
}
