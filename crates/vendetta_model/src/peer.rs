use std::fmt;

use serde::{Deserialize, Serialize};
use strum::{AsRefStr, Display, EnumString, VariantArray};

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
pub enum PeerType {
    User,
    Group,
    Channel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PeerId(pub i64);

impl PeerId {
    pub fn new(id: i64) -> Self {
        Self(id)
    }

    pub fn raw(&self) -> i64 {
        self.0
    }

    pub fn decode_user_id(&self) -> i64 {
        self.0.abs()
    }

    pub fn decode_group_id(&self) -> i64 {
        let raw = self.0;
        if (-1_000_000_000_000..0).contains(&raw) {
            -raw
        } else {
            raw
        }
    }

    pub fn decode_channel_id(&self) -> i64 {
        let raw = self.0;
        if raw <= -1_000_000_000_000 {
            -raw - 1_000_000_000_000
        } else {
            raw
        }
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<i64> for PeerId {
    fn from(id: i64) -> Self {
        Self(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRecord {
    pub peer_id: PeerId,
    pub peer_type: PeerType,
    pub name: Option<String>,
    pub username: Option<String>,
    pub phone: Option<String>,
    pub raw_tl: Option<Vec<u8>>,
    pub updated_at: i64,
}
