use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString, VariantArray};

use crate::message::MessageKey;

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
pub enum ReplyResolutionStatus {
    Resolved,
    ContextOnly,
    Missing,
    Deleted,
    Inaccessible,
    NotRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReplyRecord {
    pub source: MessageKey,
    pub target: MessageKey,
    pub top_message_id: Option<i64>,
    pub resolution_status: ReplyResolutionStatus,
}
