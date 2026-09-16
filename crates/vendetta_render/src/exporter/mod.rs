pub mod media;
pub mod message_builder;
pub mod pages;
pub mod topic;

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use grammers_tl_types::{self as tl, Deserializable};
use sha2::{Digest, Sha256};
use vendetta_model::{MessageRecord, MessageState, PeerId};
use vendetta_storage::ArchiveDb;

use crate::{
    assets::write_all_assets,
    error::{RenderError, RenderResult},
    layout::index::render_global_index,
    manifest::{
        DatasetFingerprint, HtmlExportManifest, ManifestChatEntry, compute_chat_fingerprint,
        compute_export_config_fingerprint_full,
    },
    model::{
        DateStructure, ExportOptions, ExportSummary, RenderMessage, RenderPeer, SplitBy,
    },
    navigation::DateNavigator,
    reply::{ReplyLocationMap, ReplyResolver},
    search::SearchIndexer,
    url_builder::ArchiveUrlBuilder,
    verifier::HtmlArchiveVerifier,
};

fn parse_avatar_stem(stem: &str) -> Option<PeerId> {
    stem.strip_prefix("p_neg_")
        .and_then(|rest| rest.parse::<u64>().ok())
        .map(|abs| PeerId::new(-(abs as i64)))
        .or_else(|| {
            stem.strip_prefix("p_")
                .and_then(|rest| rest.parse::<i64>().ok())
                .map(PeerId::new)
        })
}

fn scan_available_avatars(
    media_src_dir: Option<&Path>,
    existing_dirs: &[&Path],
) -> HashSet<PeerId> {
    let mut avatars = HashSet::new();

    let scan_dir = |dir: &Path, set: &mut HashSet<PeerId>| {
        if let Ok(entries) = fs::read_dir(dir) {
            entries
                .flatten()
                .filter_map(|entry| {
                    let lossy = entry.file_name().to_string_lossy().to_string();
                    let stem = lossy
                        .strip_suffix(".jpg")
                        .or_else(|| lossy.strip_suffix(".png"))?;
                    parse_avatar_stem(stem)
                })
                .for_each(|pid| {
                    set.insert(pid);
                });
        }
    };

    if let Some(src_base_dir) = media_src_dir {
        for cand in [
            src_base_dir.join("avatars"),
            src_base_dir.join("media/avatars"),
            src_base_dir.to_path_buf(),
        ] {
            scan_dir(&cand, &mut avatars);
        }
    }

    for dir in existing_dirs {
        scan_dir(&dir.join("media/avatars"), &mut avatars);
        if let Ok(entries) = fs::read_dir(dir.join("chats")) {
            entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
                .for_each(|entry| scan_dir(&entry.path().join("avatars"), &mut avatars));
        }
    }

    avatars
}

fn extract_message_scope_metadata(
    msgs: &[MessageRecord],
    current_peer_id: PeerId,
) -> (Vec<vendetta_model::MessageId>, HashSet<PeerId>, HashSet<i64>) {
    let mut msg_ids = Vec::with_capacity(msgs.len());
    let mut participants = HashSet::new();
    participants.insert(current_peer_id);
    let mut reaction_doc_ids = HashSet::new();

    for m in msgs {
        msg_ids.push(m.key.message_id);
        if let Some(sid) = m.sender_id {
            participants.insert(sid);
        }
        if let Some(fwd_json) = &m.forward_json
            && let Ok(val) = serde_json::from_str::<serde_json::Value>(fwd_json)
        {
            let v = val.get("Header").unwrap_or(&val);
            if let Some(fid) = v.get("from_id")
                && let Some((pid, _)) = message_builder::parse_json_peer(fid)
            {
                participants.insert(pid);
            }
            if let Some(sid) = v.get("saved_from_peer").or_else(|| v.get("saved_from_id"))
                && let Some((pid, _)) = message_builder::parse_json_peer(sid)
            {
                participants.insert(pid);
            }
        }
        if let Some(react_json) = &m.reactions_json
            && let Ok(val) = serde_json::from_str::<serde_json::Value>(react_json)
            && let Some(arr) = val.as_array()
        {
            arr.iter()
                .filter_map(|item| item.get("document_id").and_then(|v| v.as_i64()))
                .for_each(|doc_id| {
                    reaction_doc_ids.insert(doc_id);
                });
        }
    }

    (msg_ids, participants, reaction_doc_ids)
}

pub struct HtmlArchiveExporter<'a> {
    db: &'a ArchiveDb,
    pub options: ExportOptions,
    pub disable_forum_render: bool,
    pub readable_names: bool,
    pub date_range: (Option<i64>, Option<i64>),
    pub split_by: SplitBy,
    pub date_structure: DateStructure,
}

impl<'a> HtmlArchiveExporter<'a> {
    pub fn new(db: &'a ArchiveDb, options: ExportOptions) -> Self {
        Self {
            db,
            options,
            disable_forum_render: false,
            readable_names: false,
            date_range: (None, None),
            split_by: SplitBy::default(),
            date_structure: DateStructure::default(),
        }
    }

    pub fn with_disable_forum_render(mut self, disable: bool) -> Self {
        self.disable_forum_render = disable;
        self
    }

    pub fn with_readable_names(mut self, readable: bool) -> Self {
        self.readable_names = readable;
        self
    }

    pub fn with_date_range(mut self, from: Option<i64>, to: Option<i64>) -> Self {
        self.date_range = (from, to);
        self
    }

    pub fn with_split_by(mut self, split_by: SplitBy) -> Self {
        self.split_by = split_by;
        self
    }

    pub fn with_date_structure(mut self, date_structure: DateStructure) -> Self {
        self.date_structure = date_structure;
        self
    }

    pub fn export(&self) -> RenderResult<ExportSummary> {
        self.export_with_progress(|_, _, _| {})
    }

    pub fn export_with_progress<F>(&self, mut on_progress: F) -> RenderResult<ExportSummary>
    where
        F: FnMut(&str, usize, usize),
    {
        let target_dir = &self.options.output_dir;

        if target_dir.exists() && !self.options.replace {
            if self.split_by == SplitBy::Day && target_dir.join("manifest.json").exists() {
                return self.export_incremental_with_progress(&mut on_progress);
            }
            return Err(RenderError::TargetAlreadyExists(target_dir.clone()));
        }

        let run_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        let parent_dir = target_dir.parent().unwrap_or(Path::new("."));
        let target_name = target_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("export");
        let staging_name = format!(".{target_name}.staging-{run_id}");
        let staging_dir = parent_dir.join(staging_name);

        fs::create_dir_all(&staging_dir)?;

        let export_result = self.do_export_into_with_progress(&staging_dir, &mut on_progress);

        if let Err(e) = export_result {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(e);
        }

        let summary = export_result?;

        on_progress("Verifying staged HTML export", 0, 1);
        let verifier = HtmlArchiveVerifier::new(&staging_dir);
        let verify_report = verifier.verify()?;
        if !verify_report.is_success() {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(RenderError::VerificationFailed(format!(
                "Staged export verification failed with {} errors:\n{}",
                verify_report.errors.len(),
                verify_report.errors.join("\n")
            )));
        }
        on_progress("Promoting export to target destination", 1, 1);

        if target_dir.exists() {
            let backup_name = format!(".{target_name}.backup-{run_id}");
            let backup_dir = parent_dir.join(backup_name);

            if let Err(e) = fs::rename(target_dir, &backup_dir) {
                let _ = fs::remove_dir_all(&staging_dir);
                return Err(RenderError::Io(e));
            }

            if let Err(e) = fs::rename(&staging_dir, target_dir) {
                let _ = fs::rename(&backup_dir, target_dir);
                let _ = fs::remove_dir_all(&staging_dir);
                return Err(RenderError::Io(e));
            }

            let _ = fs::remove_dir_all(&backup_dir);
        } else if let Err(e) = fs::rename(&staging_dir, target_dir) {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(RenderError::Io(e));
        }

        Ok(summary)
    }

    fn fetch_all_peer_messages(
        &self,
        peer_id: PeerId,
        capacity_hint: usize,
    ) -> RenderResult<Vec<MessageRecord>> {
        let mut msgs = Vec::with_capacity(capacity_hint);
        let mut offset = 0;
        const BATCH: usize = 1000;
        loop {
            let batch = self.db.list_messages_by_peer(peer_id, BATCH, offset)?;
            if batch.is_empty() {
                break;
            }
            let len = batch.len();
            msgs.extend(batch);
            if len < BATCH {
                break;
            }
            offset += len;
        }
        Ok(msgs)
    }

    fn do_export_into_with_progress<F>(
        &self,
        staging_dir: &Path,
        on_progress: &mut F,
    ) -> RenderResult<ExportSummary>
    where
        F: FnMut(&str, usize, usize),
    {
        let mut summary = ExportSummary::default();
        let mut content_hasher = Sha256::new();

        let mut all_peers_raw = self.db.list_dialog_peers_with_messages()?;
        if let Some(targets) = &self.options.target_peers {
            all_peers_raw.retain(|p| targets.contains(&p.peer_id));
        }

        let mut render_peers = Vec::with_capacity(all_peers_raw.len());
        let mut location_map = ReplyLocationMap::new();
        let mut peer_message_counts = HashMap::new();

        for peer in &all_peers_raw {
            let count = self.db.count_messages_by_peer(peer.peer_id)?;
            peer_message_counts.insert(peer.peer_id, count);

            let last_date = self.db.get_last_message_date_by_peer(peer.peer_id)?;

            let authoritative_name = self
                .resolve_authoritative_title(peer.peer_id)
                .unwrap_or_else(|_| {
                    peer.name
                        .clone()
                        .unwrap_or_else(|| format!("Chat {}", peer.peer_id.raw()))
                });

            let is_forum = !self.disable_forum_render
                && peer.raw_tl.as_ref().is_some_and(|raw| {
                    tl::enums::Chat::from_bytes(raw).is_ok_and(|c| match c {
                        tl::enums::Chat::Channel(chan) => chan.forum,
                        _ => false,
                    })
                });

            let all_msgs = self.fetch_all_peer_messages(peer.peer_id, count)?;

            let in_range_msgs: Vec<MessageRecord> = all_msgs
                .iter()
                .filter(|m| {
                    self.date_range.0.is_none_or(|from_ts| m.date >= from_ts)
                        && self.date_range.1.is_none_or(|to_ts| m.date <= to_ts)
                })
                .cloned()
                .collect();
            let in_range_count = in_range_msgs.len();
            peer_message_counts.insert(peer.peer_id, in_range_count);

            let (is_forum_peer, topics) = if is_forum {
                let discovered_topics = topic::discover_topics(&all_msgs);

                let mut topic_messages: BTreeMap<i32, Vec<MessageRecord>> = discovered_topics
                    .keys()
                    .map(|&tid| (tid, Vec::new()))
                    .collect();

                for msg in in_range_msgs {
                    let resolved_tid = topic::resolve_message_topic_id(&msg, &discovered_topics);
                    topic_messages.entry(resolved_tid).or_default().push(msg);
                }

                let peer_topics = topic::build_render_topics(
                    &discovered_topics,
                    &topic_messages,
                    self.options.media_src_dir.as_deref(),
                );

                for t in &peer_topics {
                    if let Some(t_msgs) = topic_messages.get(&t.topic_id) {
                        if self.split_by == crate::model::SplitBy::Day {
                            let day_groups = pages::group_records_by_day(t_msgs);
                            for (day_idx, ((y, m, d), day_msgs)) in day_groups.iter().enumerate() {
                                let day_file = ArchiveUrlBuilder::day_page_file_name(
                                    *y,
                                    *m,
                                    *d,
                                    self.date_structure,
                                );
                                for msg in day_msgs {
                                    location_map.insert_with_file(
                                        msg.key,
                                        day_idx,
                                        Some(t.topic_id),
                                        day_file.clone(),
                                    );
                                }
                            }
                        } else {
                            for (idx_in_topic, msg) in t_msgs.iter().enumerate() {
                                let page_idx = idx_in_topic / self.options.chunk_size;
                                location_map.insert(msg.key, page_idx, Some(t.topic_id));
                            }
                        }
                    }
                }

                (true, peer_topics)
            } else {
                if self.split_by == crate::model::SplitBy::Day {
                    let day_groups = pages::group_records_by_day(&in_range_msgs);
                    for (day_idx, ((y, m, d), day_msgs)) in day_groups.iter().enumerate() {
                        let day_file = ArchiveUrlBuilder::day_page_file_name(
                            *y,
                            *m,
                            *d,
                            self.date_structure,
                        );
                        for msg in day_msgs {
                            location_map.insert_with_file(msg.key, day_idx, None, day_file.clone());
                        }
                    }
                } else {
                    for (idx_in_chat, msg) in in_range_msgs.into_iter().enumerate() {
                        let page_idx = idx_in_chat / self.options.chunk_size;
                        location_map.insert(msg.key, page_idx, None);
                    }
                }
                (false, Vec::new())
            };

            render_peers.push(RenderPeer {
                peer_id: peer.peer_id,
                peer_type: peer.peer_type,
                name: authoritative_name,
                username: peer.username.clone(),
                phone: peer.phone.clone(),
                total_messages: in_range_count,
                last_message_date: last_date,
                is_forum: is_forum_peer,
                topics,
            });
        }

        summary.dialogs_count = render_peers.len();
        let total_messages_to_render: usize = peer_message_counts.values().sum();

        let chat_dirs = resolve_chat_directories(&render_peers, self.readable_names);

        on_progress("Writing static assets (CSS, JS, icons)", 0, 1);
        write_all_assets(staging_dir)?;

        summary.media_copied_count = 0;

        if self.options.build_search_index {
            on_progress("Building search index shards", 0, 1);
            let indexer = SearchIndexer::new(self.db, &location_map).with_peer_dirs(&chat_dirs);
            let indexed_count = indexer.build_and_write_index(staging_dir, &all_peers_raw)?;
            summary.search_shards_count = (indexed_count / 2500) + 1;
        }

        on_progress(
            "Rendering chat messages",
            0,
            total_messages_to_render.max(1),
        );
        let reply_resolver = ReplyResolver::new(self.db, &location_map);
        let chats_dir = staging_dir.join("chats");
        fs::create_dir_all(&chats_dir)?;

        let mut available_avatars = scan_available_avatars(
            self.options.media_src_dir.as_deref(),
            &[staging_dir, &self.options.output_dir],
        );

        let config_fingerprint = compute_export_config_fingerprint_full(
            &self.options,
            self.readable_names,
            self.date_range.0,
            self.date_range.1,
            self.split_by,
            self.date_structure,
        );

        let prev_manifest = if self.options.output_dir.join("manifest.json").exists() {
            HtmlExportManifest::read_from_file(&self.options.output_dir.join("manifest.json")).ok()
        } else {
            None
        };

        let can_incremental = prev_manifest.as_ref().is_some_and(|prev| {
            prev.export_format == "chat-portable-v1"
                && prev.renderer_version == "vendetta_render_v2"
                && prev.export_config_fingerprint == config_fingerprint
                && prev.from_date == self.date_range.0
                && prev.to_date == self.date_range.1
                && prev.readable_names == self.readable_names
                && prev.presentation_mode == self.options.presentation_mode.to_string()
                && prev.media_mode == self.options.media_mode.to_string()
                && prev.chunk_size == self.options.chunk_size
        });

        let prev_chats_by_peer: HashMap<i64, &ManifestChatEntry> = if can_incremental {
            prev_manifest
                .as_ref()
                .map(|m| m.chats.iter().map(|c| (c.peer_id, c)).collect())
                .unwrap_or_default()
        } else {
            HashMap::new()
        };

        let mut total_rendered_messages = 0;
        let mut total_chunks = 0;
        let exported_peer_ids: HashSet<PeerId> = render_peers.iter().map(|p| p.peer_id).collect();
        let mut manifest_chat_entries = Vec::with_capacity(render_peers.len());

        for current_peer in &render_peers {
            let default_dir = ArchiveUrlBuilder::peer_token(current_peer.peer_id);
            let chat_dir = chat_dirs
                .get(&current_peer.peer_id)
                .cloned()
                .unwrap_or(default_dir);
            let peer_chat_dir = chats_dir.join(&chat_dir);

            let chat_fingerprint = compute_chat_fingerprint(
                self.db,
                current_peer.peer_id.raw(),
                &config_fingerprint,
                self.date_range.0,
                self.date_range.1,
            )?;

            if let Some(prev_entry) = prev_chats_by_peer.get(&current_peer.peer_id.raw()) {
                let prev_chat_path = self
                    .options
                    .output_dir
                    .join("chats")
                    .join(&prev_entry.directory);
                if prev_entry.fingerprint == chat_fingerprint && prev_chat_path.exists() {
                    copy_dir_all(&prev_chat_path, &peer_chat_dir)?;
                    let mut reused = (*prev_entry).clone();
                    reused.directory = chat_dir.clone();
                    reused
                        .avatar_files
                        .iter()
                        .filter_map(|av| {
                            let stem = av.strip_suffix(".jpg").or_else(|| av.strip_suffix(".png")).unwrap_or(av);
                            parse_avatar_stem(stem)
                        })
                        .for_each(|pid| {
                            available_avatars.insert(pid);
                        });
                    let chat_chunks = reused
                        .pages
                        .iter()
                        .filter(|p| {
                            if p.ends_with("index.html") {
                                false
                            } else if current_peer.is_forum && !current_peer.topics.is_empty() {
                                p.starts_with("topics/")
                            } else {
                                true
                            }
                        })
                        .count();
                    total_chunks += chat_chunks;
                    total_rendered_messages += current_peer.total_messages;
                    summary.media_copied_count += reused.media_files.len()
                        + reused.avatar_files.len()
                        + reused.reaction_files.len()
                        + reused.topic_assets.len();
                    manifest_chat_entries.push(reused);
                    continue;
                }
            }

            fs::create_dir_all(&peer_chat_dir)?;

            let total_msgs = self.db.count_messages_by_peer(current_peer.peer_id)?;
            let raw_peer_msgs = self.fetch_all_peer_messages(current_peer.peer_id, total_msgs)?;

            let in_range_raw_msgs: Vec<MessageRecord> = raw_peer_msgs
                .into_iter()
                .filter(|m| {
                    self.date_range.0.is_none_or(|from_ts| m.date >= from_ts)
                        && self.date_range.1.is_none_or(|to_ts| m.date <= to_ts)
                })
                .collect();

            let (msg_ids, participants, reaction_doc_ids) =
                extract_message_scope_metadata(&in_range_raw_msgs, current_peer.peer_id);

            let topic_icon_files: Vec<String> = current_peer
                .topics
                .iter()
                .filter_map(|top| top.icon_asset.as_deref())
                .filter_map(|icon| Path::new(icon).file_name()?.to_str())
                .filter(|name| !name.is_empty())
                .map(String::from)
                .collect();

            let chat_media_manifest = media::materialize_chat_scope(
                self.db,
                &peer_chat_dir,
                current_peer.peer_id,
                &msg_ids,
                self.options.media_src_dir.as_deref(),
                self.options.media_mode,
                &participants,
                &reaction_doc_ids,
                &topic_icon_files,
                &mut content_hasher,
            )?;
            summary.media_copied_count += chat_media_manifest.total_copied;

            chat_media_manifest
                .avatar_files
                .iter()
                .filter_map(|av| {
                    let stem = av.strip_suffix(".jpg").or_else(|| av.strip_suffix(".png")).unwrap_or(av);
                    parse_avatar_stem(stem)
                })
                .for_each(|pid| {
                    available_avatars.insert(pid);
                });

            let build_ctx = message_builder::MessageBuildContext {
                db: self.db,
                reply_resolver: &reply_resolver,
                available_avatars: &available_avatars,
                exported_peer_ids: &exported_peer_ids,
                media_src_dir: self.options.media_src_dir.as_deref(),
                include_edit_history: self.options.include_edit_history,
                authoritative_name_resolver: |pid| self.resolve_authoritative_title(pid).ok(),
                chat_depth: if current_peer.is_forum && !current_peer.topics.is_empty() {
                    2
                } else {
                    0
                },
            };

            let mut all_render_messages = Vec::with_capacity(in_range_raw_msgs.len());
            for m in &in_range_raw_msgs {
                let is_srv = m.raw_tl.as_ref().is_some_and(|raw| {
                    tl::enums::Message::from_bytes(raw)
                        .is_ok_and(|t| matches!(t, tl::enums::Message::Service(_)))
                });

                if !self.options.include_service_messages && is_srv {
                    continue;
                }
                if !self.options.include_deleted_messages && m.state == MessageState::Deleted {
                    continue;
                }

                if m.state == MessageState::Deleted {
                    summary.deleted_messages_count += 1;
                }
                if m.state == MessageState::Edited {
                    summary.edited_messages_count += 1;
                }

                if let Some(txt) = &m.text {
                    content_hasher.update(txt.as_bytes());
                }

                let r_msg = message_builder::build_render_message(&build_ctx, m)?;
                all_render_messages.push(r_msg);
                total_rendered_messages += 1;
            }

            let created_pages = if current_peer.is_forum && !current_peer.topics.is_empty() {
                let mut pages = Vec::new();
                pages.extend(pages::render_topic_scoped_pages(
                    &peer_chat_dir,
                    current_peer,
                    &render_peers,
                    &all_render_messages,
                    &location_map,
                    &self.options,
                    self.split_by,
                    self.date_structure,
                    &available_avatars,
                    Some(&chat_dirs),
                )?);

                pages.extend(pages::render_unified_messages_pages(
                    &peer_chat_dir,
                    current_peer,
                    &render_peers,
                    &all_render_messages,
                    &location_map,
                    &self.options,
                    &available_avatars,
                    Some(&chat_dirs),
                )?);

                pages.extend(pages::write_root_topic_redirect(
                    &peer_chat_dir,
                    &current_peer.topics,
                    &self.options,
                    self.split_by,
                )?);
                pages
            } else {
                pages::render_flat_dialog_pages(
                    &peer_chat_dir,
                    current_peer,
                    &render_peers,
                    &all_render_messages,
                    &self.options,
                    self.split_by,
                    self.date_structure,
                    &available_avatars,
                    Some(&chat_dirs),
                )?
            };
            let chat_chunks = created_pages
                .iter()
                .filter(|p| {
                    if p.ends_with("index.html") {
                        false
                    } else if current_peer.is_forum && !current_peer.topics.is_empty() {
                        p.starts_with("topics/")
                    } else {
                        true
                    }
                })
                .count();
            total_chunks += chat_chunks;

            let chat_search_entries: Vec<crate::search::SearchEntry> = all_render_messages
                .iter()
                .enumerate()
                .map(|(idx, m)| {
                    let loc = location_map.get_location_full(&m.key);
                    let (target_topic_id, page_file) = loc
                        .map(|l| (l.topic_id, l.page_file.clone()))
                        .unwrap_or_else(|| {
                            let page_idx = idx / self.options.chunk_size;
                            (None, ArchiveUrlBuilder::page_file_name(page_idx))
                        });
                    let anchor = ArchiveUrlBuilder::message_anchor(m.key.peer_id, m.key.message_id);
                    let url = target_topic_id
                        .map(|tid| format!("topics/{tid}/{page_file}#{anchor}"))
                        .unwrap_or_else(|| format!("{page_file}#{anchor}"));

                    let text = m.raw_text.as_deref().unwrap_or("");
                    let sender = m.sender_name.as_deref().unwrap_or("");
                    let mut tokens = crate::search::tokenize_search_text(text);
                    tokens.extend(crate::search::tokenize_search_text(sender));
                    tokens.extend(crate::search::tokenize_search_text(&current_peer.name));
                    tokens.sort_unstable();
                    tokens.dedup();

                    crate::search::SearchEntry {
                        id: format!("{}-{}", m.key.peer_id.raw(), m.key.message_id.0),
                        peer_id: m.key.peer_id.raw(),
                        peer_name: current_peer.name.clone(),
                        msg_id: m.key.message_id.0,
                        date: m.date,
                        sender: sender.to_string(),
                        text: text.to_string(),
                        tokens,
                        media_types: m
                            .media_items
                            .iter()
                            .map(|med| format!("{:?}", med.record.kind))
                            .collect(),
                        state: format!("{:?}", m.state),
                        is_fwd: m.forward_info.is_some(),
                        is_reply: m.reply_preview.is_some(),
                        url,
                    }
                })
                .collect();
            let chat_search_dir = peer_chat_dir.join("search");
            fs::create_dir_all(&chat_search_dir)?;
            let chat_search_js = crate::search::generate_chat_search_js(&chat_search_entries)
                .map_err(RenderError::Json)?;
            fs::write(chat_search_dir.join("index.js"), chat_search_js)?;

            let mut day_fingerprints = HashMap::new();
            if self.split_by == crate::model::SplitBy::Day {
                if current_peer.is_forum && !current_peer.topics.is_empty() {
                    for topic in &current_peer.topics {
                        let t_msgs: Vec<RenderMessage> = all_render_messages
                            .iter()
                            .filter(|m| {
                                location_map
                                    .get_location(&m.key)
                                    .and_then(|loc| loc.1)
                                    .unwrap_or(1)
                                    == topic.topic_id
                            })
                            .cloned()
                            .collect();
                        let day_groups = pages::group_messages_by_day(&t_msgs);
                        for ((y, m, d), day_msgs) in day_groups {
                            let file_rel = ArchiveUrlBuilder::day_page_file_name(y, m, d, self.date_structure);
                            let day_path = format!("topics/{}/{}", topic.topic_id, file_rel);
                            let fp = crate::manifest::compute_day_fingerprint(&day_msgs, &config_fingerprint);
                            day_fingerprints.insert(day_path, fp);
                        }
                    }
                } else {
                    let day_groups = pages::group_messages_by_day(&all_render_messages);
                    for ((y, m, d), day_msgs) in day_groups {
                        let file_rel = ArchiveUrlBuilder::day_page_file_name(y, m, d, self.date_structure);
                        let fp = crate::manifest::compute_day_fingerprint(&day_msgs, &config_fingerprint);
                        day_fingerprints.insert(file_rel, fp);
                    }
                }
            }

            manifest_chat_entries.push(ManifestChatEntry {
                peer_id: current_peer.peer_id.raw(),
                directory: chat_dir.clone(),
                title: Some(current_peer.name.clone()),
                fingerprint: chat_fingerprint,
                pages: created_pages,
                day_fingerprints,
                media_files: chat_media_manifest.media_files,
                avatar_files: chat_media_manifest.avatar_files,
                reaction_files: chat_media_manifest.reaction_files,
                topic_assets: chat_media_manifest.topic_assets,
            });
        }

        summary.messages_count = total_rendered_messages;
        summary.chunks_count = total_chunks;
        summary.manifest_path = staging_dir.join("manifest.json");

        let global_index_html = render_global_index(
            &render_peers,
            self.options.presentation_mode,
            self.options.theme,
            &summary,
            &available_avatars,
            &chat_dirs,
        );
        fs::write(staging_dir.join("index.html"), global_index_html)?;

        let source_fingerprint = DatasetFingerprint::compute_from_db(self.db)?;

        let manifest = HtmlExportManifest {
            format_version: 2,
            export_format: "chat-portable-v1".to_string(),
            renderer_version: "vendetta_render_v2".to_string(),
            readable_names: self.readable_names,
            from_date: self.date_range.0,
            to_date: self.date_range.1,
            presentation_mode: self.options.presentation_mode.to_string(),
            media_mode: self.options.media_mode.to_string(),
            chunk_size: self.options.chunk_size,
            split_by: Some(self.split_by.to_string()),
            date_structure: Some(self.date_structure.to_string()),
            source_fingerprint,
            export_config_fingerprint: config_fingerprint,
            summary: summary.clone(),
            chats: manifest_chat_entries,
        };

        manifest.write_to_file(&staging_dir.join("manifest.json"))?;

        Ok(summary)
    }

    pub fn resolve_authoritative_title(&self, peer_id: PeerId) -> RenderResult<String> {
        let is_valid = |s: &str| {
            let t = s.trim();
            !t.is_empty() && t != "Unknown"
        };

        if let Some(peer) = self.db.get_peer(peer_id).ok().flatten() {
            if let Some(name) = &peer.name
                && is_valid(name)
            {
                return Ok(name.trim().to_string());
            }

            if let Some(ref raw) = peer.raw_tl {
                if let Ok(tl_chat) = tl::enums::Chat::from_bytes(raw) {
                    match tl_chat {
                        tl::enums::Chat::Channel(c) if is_valid(&c.title) => {
                            return Ok(c.title.trim().to_string());
                        }
                        tl::enums::Chat::Chat(c) if is_valid(&c.title) => {
                            return Ok(c.title.trim().to_string());
                        }
                        _ => {}
                    }
                } else if let Ok(tl::enums::User::User(u)) = tl::enums::User::from_bytes(raw) {
                    let full = match (&u.first_name, &u.last_name) {
                        (Some(f), Some(l)) => format!("{f} {l}"),
                        (Some(f), None) => f.clone(),
                        (None, Some(l)) => l.clone(),
                        (None, None) => u.username.clone().unwrap_or_default(),
                    };
                    if is_valid(&full) {
                        return Ok(full.trim().to_string());
                    }
                }
            }

            if let Some(uname) = &peer.username {
                let u = uname.trim();
                if !u.is_empty() {
                    return Ok(format!("@{u}"));
                }
            }
        }

        if let Ok(Some(title)) = self.db.find_creation_or_title_change(peer_id)
            && is_valid(&title)
        {
            return Ok(title.trim().to_string());
        }

        Ok(format!("Chat {}", peer_id.raw()))
    }

    pub fn export_incremental_with_progress<F>(&self, _on_progress: &mut F) -> RenderResult<ExportSummary>
    where
        F: FnMut(&str, usize, usize),
    {
        let target_dir = &self.options.output_dir;
        let manifest_path = target_dir.join("manifest.json");
        let old_manifest = HtmlExportManifest::read_from_file(&manifest_path)?;

        let old_chats: HashMap<i64, ManifestChatEntry> = old_manifest
            .chats
            .into_iter()
            .map(|c| (c.peer_id, c))
            .collect();

        let run_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        let mut summary = ExportSummary::default();
        let mut content_hasher = Sha256::new();

        let mut all_peers_raw = self.db.list_dialog_peers_with_messages()?;
        if let Some(targets) = &self.options.target_peers {
            all_peers_raw.retain(|p| targets.contains(&p.peer_id));
        }

        let mut render_peers = Vec::with_capacity(all_peers_raw.len());
        let mut location_map = ReplyLocationMap::new();
        let mut peer_message_counts = HashMap::new();

        for peer in &all_peers_raw {
            let count = self.db.count_messages_by_peer(peer.peer_id)?;
            peer_message_counts.insert(peer.peer_id, count);

            let last_date = self.db.get_last_message_date_by_peer(peer.peer_id)?;
            let authoritative_name = self
                .resolve_authoritative_title(peer.peer_id)
                .unwrap_or_else(|_| {
                    peer.name
                        .clone()
                        .unwrap_or_else(|| format!("Chat {}", peer.peer_id.raw()))
                });

            let is_forum = !self.disable_forum_render
                && peer.raw_tl.as_ref().is_some_and(|raw| {
                    tl::enums::Chat::from_bytes(raw).is_ok_and(|c| match c {
                        tl::enums::Chat::Channel(chan) => chan.forum,
                        _ => false,
                    })
                });

            let all_msgs = self.fetch_all_peer_messages(peer.peer_id, count)?;

            let in_range_msgs: Vec<MessageRecord> = all_msgs
                .iter()
                .filter(|m| {
                    self.date_range.0.is_none_or(|from_ts| m.date >= from_ts)
                        && self.date_range.1.is_none_or(|to_ts| m.date <= to_ts)
                })
                .cloned()
                .collect();
            let in_range_count = in_range_msgs.len();
            peer_message_counts.insert(peer.peer_id, in_range_count);

            let (is_forum_peer, topics) = if is_forum {
                let discovered_topics = topic::discover_topics(&all_msgs);

                let mut topic_messages: BTreeMap<i32, Vec<MessageRecord>> = discovered_topics
                    .keys()
                    .map(|&tid| (tid, Vec::new()))
                    .collect();

                for msg in in_range_msgs {
                    let resolved_tid = topic::resolve_message_topic_id(&msg, &discovered_topics);
                    topic_messages.entry(resolved_tid).or_default().push(msg);
                }

                let peer_topics = topic::build_render_topics(
                    &discovered_topics,
                    &topic_messages,
                    self.options.media_src_dir.as_deref(),
                );

                for t in &peer_topics {
                    if let Some(t_msgs) = topic_messages.get(&t.topic_id) {
                        let day_groups = pages::group_records_by_day(t_msgs);
                        for (day_idx, ((y, m, d), day_msgs)) in day_groups.iter().enumerate() {
                            let day_file = ArchiveUrlBuilder::day_page_file_name(
                                *y,
                                *m,
                                *d,
                                self.date_structure,
                            );
                            for msg in day_msgs {
                                location_map.insert_with_file(
                                    msg.key,
                                    day_idx,
                                    Some(t.topic_id),
                                    day_file.clone(),
                                );
                            }
                        }
                    }
                }

                (true, peer_topics)
            } else {
                let day_groups = pages::group_records_by_day(&in_range_msgs);
                for (day_idx, ((y, m, d), day_msgs)) in day_groups.iter().enumerate() {
                    let day_file = ArchiveUrlBuilder::day_page_file_name(
                        *y,
                        *m,
                        *d,
                        self.date_structure,
                    );
                    for msg in day_msgs {
                        location_map.insert_with_file(msg.key, day_idx, None, day_file.clone());
                    }
                }
                (false, Vec::new())
            };

            render_peers.push(RenderPeer {
                peer_id: peer.peer_id,
                peer_type: peer.peer_type,
                name: authoritative_name,
                username: peer.username.clone(),
                phone: peer.phone.clone(),
                total_messages: in_range_count,
                last_message_date: last_date,
                is_forum: is_forum_peer,
                topics,
            });
        }

        summary.dialogs_count = render_peers.len();
        let _total_messages_to_render: usize = peer_message_counts.values().sum();
        let chat_dirs = resolve_chat_directories(&render_peers, self.readable_names);

        write_all_assets(target_dir)?;

        if self.options.build_search_index {
            let indexer = SearchIndexer::new(self.db, &location_map).with_peer_dirs(&chat_dirs);
            let indexed_count = indexer.build_and_write_index(target_dir, &all_peers_raw)?;
            summary.search_shards_count = (indexed_count / 2500) + 1;
        }

        let reply_resolver = ReplyResolver::new(self.db, &location_map);
        let chats_dir = target_dir.join("chats");
        fs::create_dir_all(&chats_dir)?;

        let mut available_avatars = scan_available_avatars(
            self.options.media_src_dir.as_deref(),
            &[target_dir],
        );

        let exported_peer_ids: HashSet<PeerId> =
            render_peers.iter().map(|p| p.peer_id).collect();
        let config_fingerprint = compute_export_config_fingerprint_full(
            &self.options,
            self.readable_names,
            self.date_range.0,
            self.date_range.1,
            self.split_by,
            self.date_structure,
        );

        let mut manifest_chat_entries = Vec::with_capacity(render_peers.len());
        let mut total_rendered_messages = 0;
        let mut total_chunks = 0;

        for current_peer in &render_peers {
            let default_dir = ArchiveUrlBuilder::peer_token(current_peer.peer_id);
            let chat_dir = chat_dirs
                .get(&current_peer.peer_id)
                .cloned()
                .unwrap_or(default_dir);
            let peer_chat_dir = chats_dir.join(&chat_dir);
            fs::create_dir_all(&peer_chat_dir)?;

            let old_entry = old_chats.get(&current_peer.peer_id.raw());

            let chat_fingerprint = compute_chat_fingerprint(
                self.db,
                current_peer.peer_id.raw(),
                &config_fingerprint,
                self.date_range.0,
                self.date_range.1,
            )?;

            let total_msgs = self.db.count_messages_by_peer(current_peer.peer_id)?;
            let raw_peer_msgs = self.fetch_all_peer_messages(current_peer.peer_id, total_msgs)?;

            let in_range_raw_msgs: Vec<MessageRecord> = raw_peer_msgs
                .into_iter()
                .filter(|m| {
                    self.date_range.0.is_none_or(|from_ts| m.date >= from_ts)
                        && self.date_range.1.is_none_or(|to_ts| m.date <= to_ts)
                })
                .collect();

            let (msg_ids, participants, reaction_doc_ids) =
                extract_message_scope_metadata(&in_range_raw_msgs, current_peer.peer_id);

            let topic_icon_files: Vec<String> = current_peer
                .topics
                .iter()
                .filter_map(|top| top.icon_asset.as_deref())
                .filter_map(|icon| Path::new(icon).file_name()?.to_str())
                .filter(|name| !name.is_empty())
                .map(String::from)
                .collect();

            let chat_media_manifest = media::materialize_chat_scope(
                self.db,
                &peer_chat_dir,
                current_peer.peer_id,
                &msg_ids,
                self.options.media_src_dir.as_deref(),
                self.options.media_mode,
                &participants,
                &reaction_doc_ids,
                &topic_icon_files,
                &mut content_hasher,
            )?;
            summary.media_copied_count += chat_media_manifest.total_copied;

            chat_media_manifest
                .avatar_files
                .iter()
                .filter_map(|av| {
                    let stem = av.strip_suffix(".jpg").or_else(|| av.strip_suffix(".png")).unwrap_or(av);
                    parse_avatar_stem(stem)
                })
                .for_each(|pid| {
                    available_avatars.insert(pid);
                });

            let build_ctx = message_builder::MessageBuildContext {
                db: self.db,
                reply_resolver: &reply_resolver,
                available_avatars: &available_avatars,
                exported_peer_ids: &exported_peer_ids,
                media_src_dir: self.options.media_src_dir.as_deref(),
                include_edit_history: self.options.include_edit_history,
                authoritative_name_resolver: |pid| self.resolve_authoritative_title(pid).ok(),
                chat_depth: if current_peer.is_forum && !current_peer.topics.is_empty() {
                    2
                } else {
                    0
                },
            };

            let mut all_render_messages = Vec::with_capacity(in_range_raw_msgs.len());
            for m in &in_range_raw_msgs {
                let is_srv = m.raw_tl.as_ref().is_some_and(|raw| {
                    tl::enums::Message::from_bytes(raw)
                        .is_ok_and(|t| matches!(t, tl::enums::Message::Service(_)))
                });

                if !self.options.include_service_messages && is_srv {
                    continue;
                }
                if !self.options.include_deleted_messages && m.state == MessageState::Deleted {
                    continue;
                }

                if m.state == MessageState::Deleted {
                    summary.deleted_messages_count += 1;
                }
                if m.state == MessageState::Edited {
                    summary.edited_messages_count += 1;
                }

                if let Some(txt) = &m.text {
                    content_hasher.update(txt.as_bytes());
                }

                let r_msg = message_builder::build_render_message(&build_ctx, m)?;
                all_render_messages.push(r_msg);
                total_rendered_messages += 1;
            }

            let mut created_pages = Vec::new();
            let mut new_day_fingerprints = HashMap::new();

            if current_peer.is_forum && !current_peer.topics.is_empty() {
                for topic in &current_peer.topics {
                    let topic_dir = peer_chat_dir.join("topics").join(topic.topic_id.to_string());
                    fs::create_dir_all(&topic_dir)?;

                    let topic_msgs: Vec<RenderMessage> = all_render_messages
                        .iter()
                        .filter(|m| {
                            let tid = location_map
                                .get_location(&m.key)
                                .and_then(|loc| loc.1)
                                .unwrap_or(1);
                            tid == topic.topic_id
                        })
                        .cloned()
                        .collect();

                    if !topic_msgs.is_empty() {
                        let day_groups = pages::group_messages_by_day(&topic_msgs);
                        let total_days = day_groups.len();
                        let day_file_names: Vec<String> = day_groups
                            .iter()
                            .map(|((y, m, d), _)| {
                                ArchiveUrlBuilder::day_page_file_name(*y, *m, *d, self.date_structure)
                            })
                            .collect();

                        let mut date_navigator = DateNavigator::new();
                        if self.options.build_date_index {
                            for (((_y, _m, _d), msgs), file_name) in day_groups.iter().zip(&day_file_names) {
                                if let Some(first_msg) = msgs.first() {
                                    date_navigator.record_message_target(first_msg.date, file_name.clone());
                                }
                            }
                        }

                        for (day_idx, (((_y, _m, _d), day_msgs), file_rel)) in
                            day_groups.iter().zip(&day_file_names).enumerate()
                        {
                            let day_path = format!("topics/{}/{}", topic.topic_id, file_rel);
                            let target_file = topic_dir.join(file_rel);
                            let day_fp = crate::manifest::compute_day_fingerprint(day_msgs, &config_fingerprint);

                            let is_unchanged = old_entry
                                .and_then(|e| e.day_fingerprints.get(&day_path))
                                .map(|s| s == &day_fp)
                                .unwrap_or(false)
                                && target_file.exists();

                            if is_unchanged {
                                // Physically untouched!
                                created_pages.push(day_path.clone());
                                new_day_fingerprints.insert(day_path, day_fp);
                            } else {
                                let page_html = pages::render_single_day_dialog_page(
                                    current_peer,
                                    &render_peers,
                                    Some(topic),
                                    &current_peer.topics,
                                    day_msgs,
                                    day_idx,
                                    total_days,
                                    file_rel,
                                    &day_file_names,
                                    &self.options,
                                    self.date_structure,
                                    &available_avatars,
                                    Some(&date_navigator),
                                    Some(&chat_dirs),
                                );

                                let file_name = target_file.file_name().and_then(|n| n.to_str()).unwrap_or("day.html");
                                let tmp_path = target_file.with_file_name(format!("{file_name}.tmp-{run_id}"));
                                if let Some(parent) = tmp_path.parent() {
                                    fs::create_dir_all(parent)?;
                                }
                                fs::write(&tmp_path, page_html)?;
                                fs::rename(&tmp_path, &target_file)?;

                                created_pages.push(day_path.clone());
                                new_day_fingerprints.insert(day_path, day_fp);
                            }
                        }

                        if let Some(earliest_day_file) = day_file_names.first() {
                            let index_redirect = format!(
                                r#"<!DOCTYPE html><html><head><meta http-equiv="refresh" content="0; url={earliest_day_file}"><script>window.location.replace("{earliest_day_file}");</script></head><body><p>Redirecting to <a href="{earliest_day_file}">topic</a>...</p></body></html>"#
                            );
                            let tmp_tindex = topic_dir.join(format!("index.html.tmp-{run_id}"));
                            fs::write(&tmp_tindex, &index_redirect)?;
                            fs::rename(&tmp_tindex, topic_dir.join("index.html"))?;
                            created_pages.push(format!("topics/{}/index.html", topic.topic_id));
                        }
                    }
                }

                created_pages.extend(pages::render_unified_messages_pages(
                    &peer_chat_dir,
                    current_peer,
                    &render_peers,
                    &all_render_messages,
                    &location_map,
                    &self.options,
                    &available_avatars,
                    Some(&chat_dirs),
                )?);

                created_pages.extend(pages::write_root_topic_redirect(
                    &peer_chat_dir,
                    &current_peer.topics,
                    &self.options,
                    self.split_by,
                )?);
            } else if !all_render_messages.is_empty() {
                let day_groups = pages::group_messages_by_day(&all_render_messages);
                let total_days = day_groups.len();
                let day_file_names: Vec<String> = day_groups
                    .iter()
                    .map(|((y, m, d), _)| {
                        ArchiveUrlBuilder::day_page_file_name(*y, *m, *d, self.date_structure)
                    })
                    .collect();

                let mut date_navigator = DateNavigator::new();
                if self.options.build_date_index {
                    for (((_y, _m, _d), msgs), file_name) in day_groups.iter().zip(&day_file_names) {
                        if let Some(first_msg) = msgs.first() {
                            date_navigator.record_message_target(first_msg.date, file_name.clone());
                        }
                    }
                }

                for (day_idx, (((_y, _m, _d), day_msgs), file_rel)) in
                    day_groups.iter().zip(&day_file_names).enumerate()
                {
                    let target_file = peer_chat_dir.join(file_rel);
                    let day_fp = crate::manifest::compute_day_fingerprint(day_msgs, &config_fingerprint);

                    let is_unchanged = old_entry
                        .and_then(|e| e.day_fingerprints.get(file_rel))
                        .map(|s| s == &day_fp)
                        .unwrap_or(false)
                        && target_file.exists();

                    if is_unchanged {
                        // Unchanged day file: completely untouched!
                        created_pages.push(file_rel.clone());
                        new_day_fingerprints.insert(file_rel.clone(), day_fp);
                    } else {
                        let page_html = pages::render_single_day_dialog_page(
                            current_peer,
                            &render_peers,
                            None,
                            &[],
                            day_msgs,
                            day_idx,
                            total_days,
                            file_rel,
                            &day_file_names,
                            &self.options,
                            self.date_structure,
                            &available_avatars,
                            Some(&date_navigator),
                            Some(&chat_dirs),
                        );

                        let file_name = target_file.file_name().and_then(|n| n.to_str()).unwrap_or("day.html");
                        let tmp_path = target_file.with_file_name(format!("{file_name}.tmp-{run_id}"));
                        if let Some(parent) = tmp_path.parent() {
                            fs::create_dir_all(parent)?;
                        }
                        fs::write(&tmp_path, page_html)?;
                        fs::rename(&tmp_path, &target_file)?;

                        created_pages.push(file_rel.clone());
                        new_day_fingerprints.insert(file_rel.clone(), day_fp);
                    }
                }

                if let Some(earliest_day_file) = day_file_names.first() {
                    let index_redirect = format!(
                        r#"<!DOCTYPE html><html><head><meta http-equiv="refresh" content="0; url={earliest_day_file}"><script>window.location.replace("{earliest_day_file}");</script></head><body><p>Redirecting to <a href="{earliest_day_file}">chat</a>...</p></body></html>"#
                    );
                    let tmp_index = peer_chat_dir.join(format!("index.html.tmp-{run_id}"));
                    fs::write(&tmp_index, &index_redirect)?;
                    fs::rename(&tmp_index, peer_chat_dir.join("index.html"))?;
                    created_pages.push("index.html".to_string());
                }
            } else {
                created_pages.extend(pages::render_flat_dialog_pages(
                    &peer_chat_dir,
                    current_peer,
                    &render_peers,
                    &all_render_messages,
                    &self.options,
                    self.split_by,
                    self.date_structure,
                    &available_avatars,
                    Some(&chat_dirs),
                )?);
            }

            // Stale cleanup
            if let Some(old_e) = old_entry {
                for old_file in old_e.day_fingerprints.keys() {
                    if !new_day_fingerprints.contains_key(old_file) {
                        let _ = fs::remove_file(peer_chat_dir.join(old_file));
                    }
                }
            }

            let chat_chunks = created_pages
                .iter()
                .filter(|p| {
                    if p.ends_with("index.html") {
                        false
                    } else if current_peer.is_forum && !current_peer.topics.is_empty() {
                        p.starts_with("topics/")
                    } else {
                        true
                    }
                })
                .count();
            total_chunks += chat_chunks;

            let chat_search_entries: Vec<crate::search::SearchEntry> = all_render_messages
                .iter()
                .enumerate()
                .map(|(idx, m)| {
                    let loc = location_map.get_location_full(&m.key);
                    let (target_topic_id, page_file) = loc
                        .map(|l| (l.topic_id, l.page_file.clone()))
                        .unwrap_or_else(|| {
                            let page_idx = idx / self.options.chunk_size;
                            (None, ArchiveUrlBuilder::page_file_name(page_idx))
                        });
                    let anchor = ArchiveUrlBuilder::message_anchor(m.key.peer_id, m.key.message_id);
                    let url = target_topic_id
                        .map(|tid| format!("topics/{tid}/{page_file}#{anchor}"))
                        .unwrap_or_else(|| format!("{page_file}#{anchor}"));

                    let text = m.raw_text.as_deref().unwrap_or("");
                    let sender = m.sender_name.as_deref().unwrap_or("");
                    let mut tokens = crate::search::tokenize_search_text(text);
                    tokens.extend(crate::search::tokenize_search_text(sender));
                    tokens.extend(crate::search::tokenize_search_text(&current_peer.name));
                    tokens.sort_unstable();
                    tokens.dedup();

                    crate::search::SearchEntry {
                        id: format!("{}-{}", m.key.peer_id.raw(), m.key.message_id.0),
                        peer_id: m.key.peer_id.raw(),
                        peer_name: current_peer.name.clone(),
                        msg_id: m.key.message_id.0,
                        date: m.date,
                        sender: sender.to_string(),
                        text: text.to_string(),
                        tokens,
                        media_types: m
                            .media_items
                            .iter()
                            .map(|med| format!("{:?}", med.record.kind))
                            .collect(),
                        state: format!("{:?}", m.state),
                        is_fwd: m.forward_info.is_some(),
                        is_reply: m.reply_preview.is_some(),
                        url,
                    }
                })
                .collect();

            let chat_search_dir = peer_chat_dir.join("search");
            fs::create_dir_all(&chat_search_dir)?;
            let chat_search_js = crate::search::generate_chat_search_js(&chat_search_entries)
                .map_err(RenderError::Json)?;
            let tmp_search_js = chat_search_dir.join(format!("index.js.tmp-{run_id}"));
            fs::write(&tmp_search_js, &chat_search_js)?;
            fs::rename(&tmp_search_js, chat_search_dir.join("index.js"))?;

            manifest_chat_entries.push(ManifestChatEntry {
                peer_id: current_peer.peer_id.raw(),
                directory: chat_dir.clone(),
                title: Some(current_peer.name.clone()),
                fingerprint: chat_fingerprint,
                pages: created_pages,
                day_fingerprints: new_day_fingerprints,
                media_files: chat_media_manifest.media_files,
                avatar_files: chat_media_manifest.avatar_files,
                reaction_files: chat_media_manifest.reaction_files,
                topic_assets: chat_media_manifest.topic_assets,
            });
        }

        summary.messages_count = total_rendered_messages;
        summary.chunks_count = total_chunks;
        summary.manifest_path = target_dir.join("manifest.json");

        let global_index_html = render_global_index(
            &render_peers,
            self.options.presentation_mode,
            self.options.theme,
            &summary,
            &available_avatars,
            &chat_dirs,
        );
        let tmp_global_index = target_dir.join(format!("index.html.tmp-{run_id}"));
        fs::write(&tmp_global_index, &global_index_html)?;
        fs::rename(&tmp_global_index, target_dir.join("index.html"))?;

        let source_fingerprint = DatasetFingerprint::compute_from_db(self.db)?;

        let manifest = HtmlExportManifest {
            format_version: 2,
            export_format: "chat-portable-v1".to_string(),
            renderer_version: "vendetta_render_v2".to_string(),
            readable_names: self.readable_names,
            from_date: self.date_range.0,
            to_date: self.date_range.1,
            presentation_mode: self.options.presentation_mode.to_string(),
            media_mode: self.options.media_mode.to_string(),
            chunk_size: self.options.chunk_size,
            split_by: Some(self.split_by.to_string()),
            date_structure: Some(self.date_structure.to_string()),
            source_fingerprint,
            export_config_fingerprint: config_fingerprint,
            summary: summary.clone(),
            chats: manifest_chat_entries,
        };

        let tmp_manifest = target_dir.join(format!("manifest.json.tmp-{run_id}"));
        manifest.write_to_file(&tmp_manifest)?;
        fs::rename(&tmp_manifest, target_dir.join("manifest.json"))?;

        let verifier = HtmlArchiveVerifier::new(target_dir);
        let verify_report = verifier.verify()?;
        if !verify_report.is_success() {
            return Err(RenderError::VerificationFailed(format!(
                "Incremental export verification failed with {} errors:\n{}",
                verify_report.errors.len(),
                verify_report.errors.join("\n")
            )));
        }

        Ok(summary)
    }
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

pub fn resolve_chat_directories(
    peers: &[RenderPeer],
    readable_names: bool,
) -> HashMap<PeerId, String> {
    if !readable_names {
        return peers
            .iter()
            .map(|p| (p.peer_id, ArchiveUrlBuilder::peer_token(p.peer_id)))
            .collect();
    }

    let base_slugs: Vec<(PeerId, String)> = peers
        .iter()
        .map(|p| {
            let slug = vendetta_core::slugify_chat_title(&p.name);
            let final_slug = if slug.is_empty() {
                ArchiveUrlBuilder::peer_token(p.peer_id)
            } else {
                slug
            };
            (p.peer_id, final_slug)
        })
        .collect();

    let counts = base_slugs
        .iter()
        .fold(HashMap::new(), |mut acc, (_, base)| {
            *acc.entry(base.clone()).or_insert(0) += 1;
            acc
        });

    base_slugs
        .into_iter()
        .map(|(pid, base)| {
            let dir = if counts.get(&base).copied().unwrap_or(0) > 1 {
                let token = ArchiveUrlBuilder::peer_token(pid);
                format!("{base}__{token}")
            } else {
                base
            };
            (pid, dir)
        })
        .collect()
}

