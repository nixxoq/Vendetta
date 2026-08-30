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
    manifest::{DatasetFingerprint, HtmlExportManifest, compute_export_config_fingerprint},
    model::{ExportOptions, ExportSummary, RenderPeer},
    reply::{ReplyLocationMap, ReplyResolver},
    search::SearchIndexer,
    url_builder::ArchiveUrlBuilder,
    verifier::HtmlArchiveVerifier,
};

pub struct HtmlArchiveExporter<'a> {
    db: &'a ArchiveDb,
    options: ExportOptions,
    pub disable_forum_render: bool,
}

impl<'a> HtmlArchiveExporter<'a> {
    pub fn new(db: &'a ArchiveDb, options: ExportOptions) -> Self {
        Self {
            db,
            options,
            disable_forum_render: false,
        }
    }

    pub fn with_disable_forum_render(mut self, disable: bool) -> Self {
        self.disable_forum_render = disable;
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

            let (is_forum_peer, topics) = if is_forum {
                let discovered_topics = topic::discover_topics(&all_msgs);

                let mut topic_messages: BTreeMap<i32, Vec<MessageRecord>> = discovered_topics
                    .keys()
                    .map(|&tid| (tid, Vec::new()))
                    .collect();

                for msg in all_msgs {
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
                        for (idx_in_topic, msg) in t_msgs.iter().enumerate() {
                            let page_idx = idx_in_topic / self.options.chunk_size;
                            location_map.insert(msg.key, page_idx, Some(t.topic_id));
                        }
                    }
                }

                (true, peer_topics)
            } else {
                for (idx_in_chat, msg) in all_msgs.into_iter().enumerate() {
                    let page_idx = idx_in_chat / self.options.chunk_size;
                    location_map.insert(msg.key, page_idx, None);
                }
                (false, Vec::new())
            };

            render_peers.push(RenderPeer {
                peer_id: peer.peer_id,
                peer_type: peer.peer_type,
                name: authoritative_name,
                username: peer.username.clone(),
                phone: peer.phone.clone(),
                total_messages: count,
                last_message_date: last_date,
                is_forum: is_forum_peer,
                topics,
            });
        }

        summary.dialogs_count = render_peers.len();
        let total_messages_to_render: usize = peer_message_counts.values().sum();

        on_progress("Writing static assets (CSS, JS, icons)", 0, 1);
        write_all_assets(staging_dir)?;

        on_progress("Materializing media assets into export", 0, 1);
        let media_copied = media::materialize_media(
            self.db,
            staging_dir,
            &all_peers_raw,
            self.options.media_src_dir.as_deref(),
            self.options.media_mode,
            &mut content_hasher,
        )?;
        summary.media_copied_count = media_copied;

        if self.options.build_search_index {
            on_progress("Building search index shards", 0, 1);
            let indexer = SearchIndexer::new(self.db, &location_map);
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

        let mut available_avatars = HashSet::new();
        let export_avatars_dir = staging_dir.join("media/avatars");
        if let Ok(entries) = fs::read_dir(&export_avatars_dir) {
            for entry in entries.flatten() {
                let fname = entry.file_name();
                let s = fname.to_string_lossy();
                let stem = s.strip_suffix(".jpg").or_else(|| s.strip_suffix(".png"));
                if let Some(stem) = stem {
                    if let Some(rest) = stem.strip_prefix("p_neg_")
                        && let Ok(abs_id) = rest.parse::<u64>()
                    {
                        available_avatars.insert(PeerId::new(-(abs_id as i64)));
                    } else if let Some(rest) = stem.strip_prefix("p_")
                        && let Ok(id) = rest.parse::<i64>()
                    {
                        available_avatars.insert(PeerId::new(id));
                    }
                }
            }
        }

        let mut total_rendered_messages = 0;
        let mut total_chunks = 0;
        let exported_peer_ids: HashSet<PeerId> = render_peers.iter().map(|p| p.peer_id).collect();

        for current_peer in &render_peers {
            let peer_chat_dir = chats_dir.join(ArchiveUrlBuilder::peer_token(current_peer.peer_id));
            fs::create_dir_all(&peer_chat_dir)?;

            let total_msgs = *peer_message_counts.get(&current_peer.peer_id).unwrap_or(&0);
            let raw_peer_msgs = self.fetch_all_peer_messages(current_peer.peer_id, total_msgs)?;

            let build_ctx = message_builder::MessageBuildContext {
                db: self.db,
                reply_resolver: &reply_resolver,
                available_avatars: &available_avatars,
                exported_peer_ids: &exported_peer_ids,
                media_src_dir: self.options.media_src_dir.as_deref(),
                include_edit_history: self.options.include_edit_history,
                authoritative_name_resolver: |pid| self.resolve_authoritative_title(pid).ok(),
            };

            let mut all_render_messages = Vec::with_capacity(raw_peer_msgs.len());
            for m in &raw_peer_msgs {
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

            if current_peer.is_forum && !current_peer.topics.is_empty() {
                total_chunks += pages::render_topic_scoped_pages(
                    &peer_chat_dir,
                    current_peer,
                    &render_peers,
                    &all_render_messages,
                    &location_map,
                    &self.options,
                    &available_avatars,
                )?;

                total_chunks += pages::render_unified_messages_pages(
                    &peer_chat_dir,
                    current_peer,
                    &render_peers,
                    &all_render_messages,
                    &location_map,
                    &self.options,
                    &available_avatars,
                )?;

                total_chunks +=
                    pages::write_root_topic_redirect(&peer_chat_dir, &current_peer.topics)?;
            } else {
                total_chunks += pages::render_flat_dialog_pages(
                    &peer_chat_dir,
                    current_peer,
                    &render_peers,
                    &all_render_messages,
                    &self.options,
                    &available_avatars,
                )?;
            }
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
        );
        fs::write(staging_dir.join("index.html"), global_index_html)?;

        let source_fingerprint = DatasetFingerprint::compute_from_db(self.db)?;
        let export_config_fingerprint = compute_export_config_fingerprint(&self.options);

        let manifest = HtmlExportManifest {
            format_version: 1,
            presentation_mode: self.options.presentation_mode.to_string(),
            media_mode: self.options.media_mode.to_string(),
            chunk_size: self.options.chunk_size,
            source_fingerprint,
            export_config_fingerprint,
            summary: summary.clone(),
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
}
