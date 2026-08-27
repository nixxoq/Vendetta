use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchEntry {
    pub id: String,
    pub peer_id: i64,
    pub peer_name: String,
    pub msg_id: i64,
    pub date: i64,
    pub sender: String,
    pub text: String,
    pub tokens: Vec<String>,
    pub media_types: Vec<String>,
    pub state: String,
    pub is_fwd: bool,
    pub is_reply: bool,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchShard {
    pub shard_id: usize,
    pub entries_count: usize,
    pub entries: Vec<SearchEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchShardMeta {
    pub shard_id: usize,
    pub file_name: String,
    pub entries_count: usize,
    pub peer_ids: Vec<i64>,
    pub min_date: i64,
    pub max_date: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchManifest {
    pub total_entries: usize,
    pub shards: Vec<SearchShardMeta>,
    pub peers: Vec<SearchPeerMeta>,
    pub prefix_index: BTreeMap<String, Vec<usize>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchPeerMeta {
    pub peer_id: i64,
    pub name: String,
    pub peer_type: String,
}

pub fn safe_json_for_script<T: Serialize>(value: &T) -> serde_json::Result<String> {
    let json_str = serde_json::to_string(value)?;
    let mut safe_str = String::with_capacity(json_str.len() + 32);

    for c in json_str.chars() {
        match c {
            '<' => safe_str.push_str("\\u003C"),
            '>' => safe_str.push_str("\\u003E"),
            '&' => safe_str.push_str("\\u0026"),
            '\u{2028}' => safe_str.push_str("\\u2028"),
            '\u{2029}' => safe_str.push_str("\\u2029"),
            _ => safe_str.push(c),
        }
    }

    Ok(safe_str)
}

pub fn generate_shard_js(shard: &SearchShard) -> serde_json::Result<String> {
    let safe_json = safe_json_for_script(shard)?;
    Ok(format!(
        "window.__VENDETTA_REGISTER_SEARCH_SHARD__({safe_json});\n"
    ))
}

pub fn generate_manifest_js(manifest: &SearchManifest) -> serde_json::Result<String> {
    let safe_json = safe_json_for_script(manifest)?;
    Ok(format!(
        "window.__VENDETTA_SEARCH_MANIFEST__ = {safe_json};\n"
    ))
}
