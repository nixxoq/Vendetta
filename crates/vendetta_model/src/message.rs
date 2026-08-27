use std::fmt;

use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString, VariantArray};

use crate::peer::PeerId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MessageId(pub i64);

impl MessageId {
    pub fn new(id: i64) -> Self {
        Self(id)
    }

    pub fn raw(&self) -> i64 {
        self.0
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<i64> for MessageId {
    fn from(id: i64) -> Self {
        Self(id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MessageKey {
    pub peer_id: PeerId,
    pub message_id: MessageId,
}

impl MessageKey {
    pub fn new(peer_id: impl Into<PeerId>, message_id: impl Into<MessageId>) -> Self {
        Self {
            peer_id: peer_id.into(),
            message_id: message_id.into(),
        }
    }
}

impl fmt::Display for MessageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}:{})", self.peer_id, self.message_id)
    }
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum MessageState {
    Active,
    Edited,
    Deleted,
    Empty,
    Inaccessible,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Display,
    EnumString,
    AsRefStr,
    VariantArray,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum VerificationObservation {
    VerifiedActive,
    ConfirmedDeleted,
    ObservedEmptyOrUnavailable,
    ObservedInaccessible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRecord {
    pub key: MessageKey,
    pub date: i64,
    pub sender_id: Option<PeerId>,
    pub text: Option<String>,
    pub entities_json: Option<String>,
    pub edit_date: Option<i64>,
    pub state: MessageState,
    pub reply_to_msg_id: Option<MessageId>,
    pub reply_to_top_id: Option<MessageId>,
    pub reply_to_peer_id: Option<PeerId>,
    pub grouped_id: Option<i64>,
    pub forward_json: Option<String>,
    pub reactions_json: Option<String>,
    pub views: Option<i32>,
    pub forwards_count: Option<i32>,
    pub raw_tl: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRevisionRecord {
    pub revision_id: Option<i64>,
    pub key: MessageKey,
    pub captured_at: i64,
    pub edit_date: Option<i64>,
    pub text: Option<String>,
    pub entities_json: Option<String>,
    pub raw_tl: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReactionKey {
    Emoji(String),
    CustomEmoji { document_id: i64 },
    Paid,
    Unknown(String),
}

impl ReactionKey {
    pub fn display_label(&self) -> String {
        match self {
            Self::Emoji(s) | Self::Unknown(s) => s.clone(),
            Self::CustomEmoji { document_id } => format!("custom_{document_id}"),
            Self::Paid => "⭐".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactorInfo {
    pub peer_id: PeerId,
    pub date: i64,
    pub is_me: bool,
    pub reaction: ReactionKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionCountInfo {
    pub reaction: ReactionKey,
    pub count: usize,
    pub chosen_order: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MessageReactionsData {
    pub results: Vec<ReactionCountInfo>,
    pub recent_reactors: Vec<ReactorInfo>,
    pub can_see_list: bool,
}

pub fn parse_reactions_json(raw_json: &str) -> Option<MessageReactionsData> {
    if raw_json.trim().is_empty() {
        return None;
    }

    let val: serde_json::Value = serde_json::from_str(raw_json).ok()?;
    let reactions_obj = val.get("Reactions").unwrap_or(&val);

    let can_see_list = reactions_obj
        .get("can_see_list")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let mut results = Vec::new();
    if let Some(res_arr) = reactions_obj.get("results").and_then(|v| v.as_array()) {
        for item in res_arr {
            let count_obj = item.get("Count").unwrap_or(item);
            let count = count_obj.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let chosen_order = count_obj
                .get("chosen_order")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);

            let reaction = parse_reaction_key(count_obj.get("reaction"));
            results.push(ReactionCountInfo {
                reaction,
                count,
                chosen_order,
            });
        }
    }

    let mut recent_reactors = Vec::new();
    if let Some(recent_arr) = reactions_obj
        .get("recent_reactions")
        .and_then(|v| v.as_array())
    {
        for item in recent_arr {
            let r_obj = item.get("Reaction").unwrap_or(item);
            let is_me = r_obj.get("my").and_then(|v| v.as_bool()).unwrap_or(false);
            let date = r_obj.get("date").and_then(|v| v.as_i64()).unwrap_or(0);
            let reaction = parse_reaction_key(r_obj.get("reaction"));

            if let Some(peer_id) = r_obj.get("peer_id").and_then(parse_reaction_peer_id) {
                recent_reactors.push(ReactorInfo {
                    peer_id,
                    date,
                    is_me,
                    reaction,
                });
            }
        }
    }

    if results.is_empty() && recent_reactors.is_empty() {
        None
    } else {
        Some(MessageReactionsData {
            results,
            recent_reactors,
            can_see_list,
        })
    }
}

fn parse_reaction_peer_id(p: &serde_json::Value) -> Option<PeerId> {
    if let Some(uid) = p
        .get("User")
        .and_then(|u| u.get("user_id"))
        .and_then(|v| v.as_i64())
    {
        Some(PeerId::new(uid))
    } else if let Some(cid) = p
        .get("Channel")
        .and_then(|c| c.get("channel_id"))
        .and_then(|v| v.as_i64())
    {
        Some(PeerId::new(-1_000_000_000_000i64 - cid))
    } else if let Some(chat_id) = p
        .get("Chat")
        .and_then(|c| c.get("chat_id"))
        .and_then(|v| v.as_i64())
    {
        Some(PeerId::new(-chat_id))
    } else {
        p.as_i64().map(PeerId::new)
    }
}

fn parse_reaction_key(val: Option<&serde_json::Value>) -> ReactionKey {
    let Some(v) = val else {
        return ReactionKey::Unknown("unknown".to_string());
    };

    if let Some(emoticon) = v
        .get("Emoji")
        .and_then(|e| e.get("emoticon"))
        .and_then(|s| s.as_str())
    {
        return ReactionKey::Emoji(emoticon.to_string());
    }
    if let Some(doc_id) = v
        .get("CustomEmoji")
        .and_then(|c| c.get("document_id"))
        .and_then(|d| d.as_i64())
    {
        return ReactionKey::CustomEmoji {
            document_id: doc_id,
        };
    }
    if v.get("Paid").is_some() {
        return ReactionKey::Paid;
    }
    if let Some(s) = v.as_str() {
        return ReactionKey::Emoji(s.to_string());
    }

    ReactionKey::Unknown(v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reactions_json_handles_empty_and_corrupt_inputs() {
        assert_eq!(parse_reactions_json(""), None);
        assert_eq!(parse_reactions_json("   "), None);
        assert_eq!(parse_reactions_json("not-json"), None);
        assert_eq!(parse_reactions_json("{}"), None);
    }

    #[test]
    fn parse_reactions_json_extracts_counts_and_reactors() {
        let json_str = r#"{
            "can_see_list": true,
            "results": [
                {
                    "reaction": { "Emoji": { "emoticon": "👍" } },
                    "count": 5,
                    "chosen_order": 1
                },
                {
                    "reaction": { "CustomEmoji": { "document_id": 999888777 } },
                    "count": 2
                }
            ],
            "recent_reactions": [
                {
                    "peer_id": { "User": { "user_id": 12345 } },
                    "date": 1700000000,
                    "reaction": { "Emoji": { "emoticon": "👍" } }
                }
            ]
        }"#;

        let parsed = parse_reactions_json(json_str).expect("should parse valid reactions");
        assert!(parsed.can_see_list);
        assert_eq!(parsed.results.len(), 2);
        assert_eq!(
            parsed.results[0].reaction,
            ReactionKey::Emoji("👍".to_string())
        );
        assert_eq!(parsed.results[0].count, 5);
        assert_eq!(parsed.results[0].chosen_order, Some(1));
        assert_eq!(
            parsed.results[1].reaction,
            ReactionKey::CustomEmoji {
                document_id: 999888777
            }
        );
        assert_eq!(parsed.recent_reactors.len(), 1);
        assert_eq!(parsed.recent_reactors[0].peer_id, PeerId::new(12345));
    }
}
