use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::Path,
};

use vendetta_model::{MessageState, PeerId, PeerRecord};
use vendetta_storage::ArchiveDb;

use crate::{
    error::RenderResult,
    reply::ReplyLocationMap,
    search::{
        ranking::tokenize_search_text,
        shard_writer::{
            SearchEntry, SearchManifest, SearchPeerMeta, SearchShard, SearchShardMeta,
            generate_manifest_js, generate_shard_js,
        },
    },
    url_builder::ArchiveUrlBuilder,
};

pub struct SearchIndexer<'a> {
    db: &'a ArchiveDb,
    location_map: &'a ReplyLocationMap,
    entries_per_shard: usize,
}

impl<'a> SearchIndexer<'a> {
    pub fn new(db: &'a ArchiveDb, location_map: &'a ReplyLocationMap) -> Self {
        Self {
            db,
            location_map,
            entries_per_shard: 2500,
        }
    }

    pub fn with_entries_per_shard(mut self, count: usize) -> Self {
        self.entries_per_shard = count.max(1);
        self
    }

    pub fn build_and_write_index(
        &self,
        export_dir: &Path,
        peers: &[PeerRecord],
    ) -> RenderResult<usize> {
        let search_dir = export_dir.join("search");
        let shards_dir = search_dir.join("shards");
        fs::create_dir_all(&shards_dir)?;

        let mut peer_names: HashMap<PeerId, String> = HashMap::new();
        let mut peer_metas = Vec::with_capacity(peers.len());
        for p in peers {
            let name = p
                .name
                .clone()
                .unwrap_or_else(|| format!("Chat {}", p.peer_id.raw()));
            peer_names.insert(p.peer_id, name.clone());
            peer_metas.push(SearchPeerMeta {
                peer_id: p.peer_id.raw(),
                name,
                peer_type: p.peer_type.to_string(),
            });
        }

        let mut current_shard_entries = Vec::with_capacity(self.entries_per_shard);
        let mut shard_metas = Vec::new();
        let mut prefix_index: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut shard_id = 1;
        let mut total_indexed = 0;

        for peer in peers {
            let chat_name = peer_names.get(&peer.peer_id).cloned().unwrap_or_default();
            let mut offset_cursor = 0;
            const BATCH_SIZE: usize = 500;

            loop {
                let msgs =
                    self.db
                        .list_messages_by_peer(peer.peer_id, BATCH_SIZE, offset_cursor)?;
                if msgs.is_empty() {
                    break;
                }

                let batch_count = msgs.len();

                for msg in msgs {
                    if msg.state == MessageState::Empty || msg.state == MessageState::Inaccessible {
                        continue;
                    }

                    let (page_idx, target_topic_id) =
                        self.location_map.get_location(&msg.key).unwrap_or((0, None));
                    let anchor = ArchiveUrlBuilder::message_anchor(msg.key.peer_id, msg.key.message_id);
                    let target_chunk = if let Some(tid) = target_topic_id {
                        ArchiveUrlBuilder::topic_chunk_file_rel(msg.key.peer_id, tid, page_idx)
                    } else {
                        ArchiveUrlBuilder::chunk_file_rel(msg.key.peer_id, page_idx)
                    };
                    let url = format!("{target_chunk}#{anchor}");

                    let sender_name = if let Some(sid) = msg.sender_id {
                        if let Some(name) = peer_names.get(&sid) {
                            name.clone()
                        } else if let Ok(Some(p)) = self.db.get_peer(sid) {
                            let n = p.name.unwrap_or_else(|| format!("User {}", sid.raw()));
                            peer_names.insert(sid, n.clone());
                            n
                        } else {
                            format!("User {}", sid.raw())
                        }
                    } else {
                        chat_name.clone()
                    };

                    let text = msg.text.unwrap_or_default();
                    let mut tokens = tokenize_search_text(&text);
                    tokens.extend(tokenize_search_text(&sender_name));
                    tokens.extend(tokenize_search_text(&chat_name));
                    tokens.sort_unstable();
                    tokens.dedup();

                    let media_list = self
                        .db
                        .get_media_for_message(msg.key.peer_id, msg.key.message_id)
                        .unwrap_or_default();
                    let media_types = media_list.into_iter().map(|m| m.kind.to_string()).collect();

                    for token in &tokens {
                        let t_lower = token.to_lowercase();
                        for (byte_idx, ch) in t_lower.char_indices().take(3) {
                            let end_idx = byte_idx + ch.len_utf8();
                            prefix_index
                                .entry(t_lower[..end_idx].to_string())
                                .or_default()
                                .push(shard_id);
                        }
                    }

                    let entry = SearchEntry {
                        id: ArchiveUrlBuilder::message_anchor(msg.key.peer_id, msg.key.message_id),
                        peer_id: msg.key.peer_id.raw(),
                        peer_name: chat_name.clone(),
                        msg_id: msg.key.message_id.raw(),
                        date: msg.date,
                        sender: sender_name,
                        text,
                        tokens,
                        media_types,
                        state: msg.state.to_string(),
                        is_fwd: msg.forward_json.is_some(),
                        is_reply: msg.reply_to_msg_id.is_some(),
                        url,
                    };

                    current_shard_entries.push(entry);
                    total_indexed += 1;

                    if current_shard_entries.len() >= self.entries_per_shard {
                        flush_shard(
                            &shards_dir,
                            shard_id,
                            std::mem::take(&mut current_shard_entries),
                            &mut shard_metas,
                        )?;
                        shard_id += 1;
                    }
                }

                if batch_count < BATCH_SIZE {
                    break;
                }
                offset_cursor += batch_count;
            }
        }

        if !current_shard_entries.is_empty() {
            flush_shard(
                &shards_dir,
                shard_id,
                current_shard_entries,
                &mut shard_metas,
            )?;
        }

        for list in prefix_index.values_mut() {
            list.sort_unstable();
            list.dedup();
        }

        let manifest = SearchManifest {
            total_entries: total_indexed,
            shards: shard_metas,
            peers: peer_metas,
            prefix_index,
        };

        let manifest_path = search_dir.join("manifest.js");
        let manifest_js = generate_manifest_js(&manifest)?;
        fs::write(&manifest_path, manifest_js)?;

        Ok(total_indexed)
    }
}

fn flush_shard(
    shards_dir: &Path,
    shard_id: usize,
    entries: Vec<SearchEntry>,
    shard_metas: &mut Vec<SearchShardMeta>,
) -> RenderResult<()> {
    let shard_file_name = format!("shard_{shard_id:05}.js");
    let shard_path = shards_dir.join(&shard_file_name);
    let mut p_ids: Vec<i64> = entries.iter().map(|e| e.peer_id).collect();
    p_ids.sort_unstable();
    p_ids.dedup();

    let min_d = entries.iter().map(|e| e.date).min().unwrap_or(0);
    let max_d = entries.iter().map(|e| e.date).max().unwrap_or(0);
    let entries_count = entries.len();

    let shard = SearchShard {
        shard_id,
        entries_count,
        entries,
    };

    let js_content = generate_shard_js(&shard)?;
    fs::write(&shard_path, js_content)?;

    shard_metas.push(SearchShardMeta {
        shard_id,
        file_name: shard_file_name,
        entries_count,
        peer_ids: p_ids,
        min_date: min_d,
        max_date: max_d,
    });

    Ok(())
}
