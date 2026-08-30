use serde::{Deserialize, Serialize};

pub type TopicId = i32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicInfo {
    pub topic_id: TopicId,
    pub title: String,
    pub icon_color: Option<i32>,
    pub icon_emoji_id: Option<i64>,
    pub is_general: bool,
    pub is_closed: bool,
    pub is_pinned: bool,
    pub is_hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TopicAction {
    Created {
        title: String,
        icon_color: Option<i32>,
        icon_emoji_id: Option<i64>,
    },
    Edited {
        new_title: Option<String>,
        new_icon_emoji_id: Option<i64>,
        is_closed: Option<bool>,
        is_hidden: Option<bool>,
    },
}
