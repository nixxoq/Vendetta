use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use grammers_tl_types::{self as tl, Deserializable};
use sha2::{Digest, Sha256};
use vendetta_model::{MessageKey, MessageRecord, MessageState, PeerId, PeerRecord, PeerType};
use vendetta_storage::ArchiveDb;

use crate::{
    assets::write_all_assets,
    entity::render_formatted_text,
    error::{RenderError, RenderResult},
    layout::{
        dialog::{DialogPageContext, render_dialog_page},
        index::render_global_index,
    },
    manifest::{DatasetFingerprint, HtmlExportManifest, compute_export_config_fingerprint},
    media::validate_and_clean_media_rel_path,
    message::group_messages_into_render_items,
    model::{
        ExportOptions, ExportSummary, MediaMode, RenderForwardInfo, RenderMediaItem, RenderMessage,
        RenderPeer, RenderReactionGroup, RenderReactionKey, RenderReactor, RenderRevision,
    },
    navigation::DateNavigator,
    reply::{ReplyLocationMap, ReplyResolver},
    search::SearchIndexer,
    url_builder::ArchiveUrlBuilder,
    verifier::HtmlArchiveVerifier,
};

pub struct HtmlArchiveExporter<'a> {
    db: &'a ArchiveDb,
    options: ExportOptions,
}

impl<'a> HtmlArchiveExporter<'a> {
    pub fn new(db: &'a ArchiveDb, options: ExportOptions) -> Self {
        Self { db, options }
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

            render_peers.push(RenderPeer {
                peer_id: peer.peer_id,
                peer_type: peer.peer_type,
                name: authoritative_name,
                username: peer.username.clone(),
                phone: peer.phone.clone(),
                total_messages: count,
                last_message_date: last_date,
            });

            let mut offset = 0;
            const SCAN_BATCH: usize = 1000;
            loop {
                let msgs = self
                    .db
                    .list_messages_by_peer(peer.peer_id, SCAN_BATCH, offset)?;
                if msgs.is_empty() {
                    break;
                }
                let batch_len = msgs.len();
                for (idx_in_chat, msg) in msgs.into_iter().enumerate() {
                    let global_idx = offset + idx_in_chat;
                    let page_idx = global_idx / self.options.chunk_size;
                    location_map.insert(msg.key, page_idx);
                }
                if batch_len < SCAN_BATCH {
                    break;
                }
                offset += batch_len;
            }
        }

        summary.dialogs_count = render_peers.len();
        let total_messages_to_render: usize = peer_message_counts.values().sum();

        on_progress("Writing static assets (CSS, JS, icons)", 0, 1);
        write_all_assets(staging_dir)?;

        on_progress("Materializing media assets into export", 0, 1);
        let media_copied =
            self.materialize_media(staging_dir, &all_peers_raw, &mut content_hasher)?;
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
            let total_msgs = *peer_message_counts.get(&current_peer.peer_id).unwrap_or(&0);
            let total_pages = if total_msgs == 0 {
                1
            } else {
                total_msgs.div_ceil(self.options.chunk_size)
            };

            let peer_chat_dir = chats_dir.join(ArchiveUrlBuilder::peer_token(current_peer.peer_id));
            fs::create_dir_all(&peer_chat_dir)?;

            let mut date_navigator = DateNavigator::new();
            if self.options.build_date_index {
                let peer_dates = self.db.list_message_dates_by_peer(current_peer.peer_id)?;
                for (msg_idx, date) in peer_dates.into_iter().enumerate() {
                    let p_idx = msg_idx / self.options.chunk_size;
                    date_navigator.record_message_date(date, p_idx);
                }
            }

            for page_idx in 0..total_pages {
                let offset = page_idx * self.options.chunk_size;
                let raw_msgs = self.db.list_messages_by_peer(
                    current_peer.peer_id,
                    self.options.chunk_size,
                    offset,
                )?;

                let mut render_messages = Vec::with_capacity(raw_msgs.len());

                for m in raw_msgs {
                    let is_srv = if let Some(ref raw) = m.raw_tl {
                        tl::enums::Message::from_bytes(raw)
                            .map(|t| matches!(t, tl::enums::Message::Service(_)))
                            .unwrap_or(false)
                    } else {
                        false
                    };

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

                    content_hasher.update(m.key.peer_id.raw().to_le_bytes());
                    content_hasher.update(m.key.message_id.raw().to_le_bytes());
                    content_hasher.update(m.date.to_le_bytes());
                    content_hasher.update(m.state.as_ref().as_bytes());
                    if let Some(txt) = &m.text {
                        content_hasher.update(txt.as_bytes());
                    }

                    let r_msg = self.build_render_message(
                        &m,
                        &reply_resolver,
                        &available_avatars,
                        &exported_peer_ids,
                    )?;
                    render_messages.push(r_msg);
                    total_rendered_messages += 1;
                }

                let first_gid = render_messages.first().and_then(|m| m.grouped_id);
                let last_gid = render_messages.last().and_then(|m| m.grouped_id);

                let cont_prev_gid = if page_idx > 0 && first_gid.is_some() && offset > 0 {
                    self.db
                        .list_messages_by_peer(current_peer.peer_id, 1, offset - 1)?
                        .first()
                        .and_then(|m| m.grouped_id)
                        .filter(|&gid| Some(gid) == first_gid)
                } else {
                    None
                };

                let cont_next_gid = if (page_idx + 1) < total_pages && last_gid.is_some() {
                    self.db
                        .list_messages_by_peer(
                            current_peer.peer_id,
                            1,
                            offset + self.options.chunk_size,
                        )?
                        .first()
                        .and_then(|m| m.grouped_id)
                        .filter(|&gid| Some(gid) == last_gid)
                } else {
                    None
                };

                let render_items =
                    group_messages_into_render_items(render_messages, cont_prev_gid, cont_next_gid);

                let date_nav_html = if self.options.build_date_index {
                    Some(date_navigator.render_date_jump_menu(current_peer.peer_id))
                } else {
                    None
                };

                let page_ctx = DialogPageContext {
                    current_peer,
                    all_peers: &render_peers,
                    items: &render_items,
                    page_index: page_idx,
                    total_pages,
                    presentation_mode: self.options.presentation_mode,
                    theme: self.options.theme,
                    date_nav_html: date_nav_html.as_deref(),
                    available_avatars: &available_avatars,
                };

                let page_html = render_dialog_page(&page_ctx);
                let page_file_name = ArchiveUrlBuilder::page_file_name(page_idx);
                fs::write(peer_chat_dir.join(page_file_name), page_html)?;

                total_chunks += 1;
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
        if let Some(peer) = self.db.get_peer(peer_id).ok().flatten() {
            if let Some(name) = &peer.name {
                let trimmed = name.trim();
                if !trimmed.is_empty() && trimmed != "Unknown" {
                    return Ok(trimmed.to_string());
                }
            }

            // i added two fallbacks cuz mtproto sucks:

            // First, retrieve chat/group/user title directly from Raw TL
            if let Some(ref raw) = peer.raw_tl {
                if let Ok(tl::enums::Chat::Channel(c)) = tl::enums::Chat::from_bytes(raw) {
                    let t = c.title.trim();
                    if !t.is_empty() && t != "Unknown" {
                        return Ok(t.to_string());
                    }
                } else if let Ok(tl::enums::Chat::Chat(c)) = tl::enums::Chat::from_bytes(raw) {
                    let t = c.title.trim();
                    if !t.is_empty() && t != "Unknown" {
                        return Ok(t.to_string());
                    }
                } else if let Ok(tl::enums::User::User(u)) = tl::enums::User::from_bytes(raw) {
                    let full = match (&u.first_name, &u.last_name) {
                        (Some(f), Some(l)) => format!("{f} {l}"),
                        (Some(f), None) => f.clone(),
                        (None, Some(l)) => l.clone(),
                        (None, None) => u.username.clone().unwrap_or_default(),
                    };
                    let trimmed = full.trim();
                    if !trimmed.is_empty() && trimmed != "Unknown" {
                        return Ok(trimmed.to_string());
                    }
                }
            }

            // If not, use fucking username
            if let Some(uname) = &peer.username {
                let u = uname.trim();
                if !u.is_empty() {
                    return Ok(format!("@{u}"));
                }
            }
        }

        // Oh yeah, i cooked cringe fallback, but it works lol
        // for channels only
        if let Ok(Some(title)) = self.db.find_creation_or_title_change(peer_id) {
            let t = title.trim();
            if !t.is_empty() && t != "Unknown" {
                return Ok(t.to_string());
            }
        }

        Ok(format!("Chat {}", peer_id.raw()))
    }

    fn build_render_message(
        &self,
        msg: &MessageRecord,
        reply_resolver: &ReplyResolver,
        available_avatars: &HashSet<PeerId>,
        exported_peer_ids: &HashSet<PeerId>,
    ) -> RenderResult<RenderMessage> {
        let current_peer = self.db.get_peer(msg.key.peer_id).ok().flatten();
        let is_channel = current_peer
            .as_ref()
            .map(|p| p.peer_type == PeerType::Channel)
            .unwrap_or(false);

        let (sender_name, is_channel_post) = if let Some(sid) = msg.sender_id {
            let name = self
                .resolve_authoritative_title(sid)
                .unwrap_or_else(|_| format!("User {}", sid.raw()));
            (Some(name), false)
        } else if is_channel {
            // In broadcast channels, sender is the channel itself!
            let name = self
                .resolve_authoritative_title(msg.key.peer_id)
                .unwrap_or_else(|_| format!("Channel {}", msg.key.peer_id.raw()));
            (Some(name), true)
        } else {
            (None, false)
        };

        let mut is_service = false;
        let mut service_description = None;
        let mut author_signature = None;
        let mut comments_count = None;
        let mut has_comments = false;

        // get comments (replies) from channel
        if let Some(ref raw) = msg.raw_tl {
            if let Ok(tl::enums::Message::Service(s)) = tl::enums::Message::from_bytes(raw) {
                is_service = true;
                service_description = if let Some(t) = &msg.text
                    && !t.trim().is_empty()
                {
                    Some(t.clone())
                } else {
                    Some(format_service_action(&s.action))
                };
            } else if let Ok(tl::enums::Message::Message(m)) = tl::enums::Message::from_bytes(raw) {
                author_signature = m.post_author;
                if let Some(tl::enums::MessageReplies::Replies(r)) = m.replies {
                    has_comments = r.comments;
                    if r.replies > 0 {
                        comments_count = Some(r.replies);
                    }
                }
            }
        }

        let formatted_html = msg
            .text
            .as_deref()
            .map(|t| render_formatted_text(t, msg.entities_json.as_deref()));

        let reply_preview = msg.reply_to_msg_id.map(|target_id| {
            let target_peer = msg.reply_to_peer_id.unwrap_or(msg.key.peer_id);
            let target_key = MessageKey::new(target_peer, target_id);
            reply_resolver.resolve_reply(msg.key.peer_id, target_key)
        });

        let forward_info = self.resolve_forward_info(msg, available_avatars, exported_peer_ids);

        let mut revisions = Vec::new();
        if self.options.include_edit_history {
            let rev_records = self.db.list_message_revisions(msg.key)?;
            for rev in rev_records {
                let rev_html = rev
                    .text
                    .as_deref()
                    .map(|t| render_formatted_text(t, rev.entities_json.as_deref()))
                    .unwrap_or_default();

                revisions.push(RenderRevision {
                    captured_at: rev.captured_at,
                    edit_date: rev.edit_date,
                    formatted_html: rev_html,
                    raw_text: rev.text,
                });
            }
        }

        let mut media_items = Vec::new();
        let raw_media_joins = self
            .db
            .get_message_media_with_roles(msg.key.peer_id, msg.key.message_id)?;

        let mut seen_media_ids = HashSet::new();
        let mut has_primary_video = false;

        for (m_rec, role, position) in raw_media_joins {
            if !seen_media_ids.insert(m_rec.media_id.clone()) {
                continue;
            }

            if !role.is_primary_attachment() {
                continue;
            }

            if let Some(ref mime) = m_rec.mime_type
                && (mime.contains("mpegurl") || mime.contains("tgstoryboard"))
            {
                continue;
            }

            if m_rec.kind == vendetta_model::MediaKind::Video {
                if has_primary_video && position > 0 {
                    continue;
                }
                has_primary_video = true;
            }

            let rel_url = m_rec
                .local_rel_path
                .as_deref()
                .map(|p| ArchiveUrlBuilder::media_url(2, p));

            let (is_available, unavailable_reason) = match m_rec.download_status {
                vendetta_model::MediaDownloadStatus::Completed => {
                    if m_rec.verification_status
                        == vendetta_model::MediaVerificationStatus::CorruptedHash
                        || m_rec.verification_status
                            == vendetta_model::MediaVerificationStatus::CorruptedSize
                    {
                        (false, Some("File corrupted on disk".to_string()))
                    } else if m_rec.verification_status
                        == vendetta_model::MediaVerificationStatus::MissingFile
                    {
                        (false, Some("File missing on disk".to_string()))
                    } else if m_rec.local_rel_path.is_some() {
                        (true, None)
                    } else {
                        (false, Some("No local path assigned".to_string()))
                    }
                }
                vendetta_model::MediaDownloadStatus::Skipped => {
                    let r = m_rec
                        .filter_reason
                        .map(|fr| format!("Skipped: {}", fr.as_ref()))
                        .unwrap_or_else(|| "Skipped by download filter policy".to_string());
                    (false, Some(r))
                }
                vendetta_model::MediaDownloadStatus::VerificationFailed => {
                    (false, Some("Verification failed".to_string()))
                }
                vendetta_model::MediaDownloadStatus::PermanentlyFailed => {
                    (false, Some("Download permanently failed".to_string()))
                }
                _ => (
                    false,
                    Some("Download pending / not yet fetched".to_string()),
                ),
            };

            media_items.push(RenderMediaItem {
                record: m_rec,
                relative_url: rel_url,
                is_available,
                unavailable_reason,
            });
        }

        let reactions = self.resolve_reactions(msg, available_avatars);

        Ok(RenderMessage {
            key: msg.key,
            date: msg.date,
            sender_id: msg.sender_id,
            sender_name,
            is_outgoing: false,
            state: msg.state,
            formatted_html,
            raw_text: msg.text.clone(),
            reply_preview,
            forward_info,
            media_items,
            revisions,
            grouped_id: msg.grouped_id,
            is_service,
            service_description,
            views: msg.views,
            forwards_count: msg.forwards_count,
            author_signature,
            reply_to_top_id: msg.reply_to_top_id,
            reactions,
            is_channel_post,
            comments_count,
            has_comments,
        })
    }

    fn resolve_reactions(
        &self,
        msg: &MessageRecord,
        available_avatars: &HashSet<PeerId>,
    ) -> Vec<RenderReactionGroup> {
        let Some(ref raw_json) = msg.reactions_json else {
            return Vec::new();
        };

        let Some(data) = vendetta_model::parse_reactions_json(raw_json) else {
            return Vec::new();
        };

        let mut groups = Vec::new();

        for res in data.results {
            let is_chosen = res.chosen_order.is_some()
                || data
                    .recent_reactors
                    .iter()
                    .any(|r| r.is_me && r.reaction == res.reaction);

            let matching_reactors: Vec<RenderReactor> = data
                .recent_reactors
                .iter()
                .filter(|r| r.reaction == res.reaction)
                .map(|r| {
                    let (name, username) = if let Ok(Some(p)) = self.db.get_peer(r.peer_id) {
                        (
                            p.name
                                .unwrap_or_else(|| format!("User {}", r.peer_id.raw())),
                            p.username,
                        )
                    } else {
                        (format!("User {}", r.peer_id.raw()), None)
                    };

                    let avatar_markup = crate::url_builder::render_avatar_markup(
                        Some(r.peer_id),
                        &name,
                        2,
                        false,
                        "avatar-img",
                        available_avatars,
                    );

                    RenderReactor {
                        peer_id: r.peer_id,
                        name,
                        username,
                        avatar_markup: Some(avatar_markup),
                        is_me: r.is_me,
                    }
                })
                .collect();

            let render_key = match res.reaction {
                vendetta_model::ReactionKey::Emoji(s) => RenderReactionKey::Emoji(s),
                vendetta_model::ReactionKey::CustomEmoji { document_id } => {
                    let asset_path = format!("media/reactions/{document_id}.webp");
                    let exists = self.options.media_src_dir.as_ref().is_some_and(|src| {
                        src.join(&asset_path).is_file()
                            || src.join(format!("reactions/{document_id}.webp")).is_file()
                    });

                    if exists {
                        let rel_url = ArchiveUrlBuilder::media_url(2, &asset_path);
                        RenderReactionKey::CustomEmoji {
                            document_id,
                            alt_text: None,
                            asset_rel_path: Some(rel_url),
                        }
                    } else {
                        RenderReactionKey::CustomEmoji {
                            document_id,
                            alt_text: None,
                            asset_rel_path: None,
                        }
                    }
                }
                vendetta_model::ReactionKey::Paid => RenderReactionKey::Paid,
                vendetta_model::ReactionKey::Unknown(s) => RenderReactionKey::Unknown(s),
            };

            groups.push(RenderReactionGroup {
                reaction: render_key,
                count: res.count,
                is_chosen_by_me: is_chosen,
                reactors: matching_reactors,
            });
        }

        groups
    }

    fn resolve_forward_info(
        &self,
        msg: &MessageRecord,
        available_avatars: &HashSet<PeerId>,
        exported_peer_ids: &HashSet<PeerId>,
    ) -> Option<RenderForwardInfo> {
        let mut from_peer: Option<(PeerId, PeerType)> = None;
        let mut saved_from_peer: Option<(PeerId, PeerType)> = None;
        let mut from_name: Option<String> = None;
        let mut saved_from_name: Option<String> = None;
        let mut channel_post: Option<i64> = None;
        let mut saved_from_msg_id: Option<i64> = None;
        let mut post_author: Option<String> = None;
        let mut date: Option<i64> = None;
        let mut saved_date: Option<i64> = None;

        let mut found_in_tl = false;
        if let Some(ref bytes) = msg.raw_tl
            && let Ok(tl::enums::Message::Message(m)) = tl::enums::Message::from_bytes(bytes)
            && let Some(tl::enums::MessageFwdHeader::Header(h)) = &m.fwd_from
        {
            found_in_tl = true;
            from_peer = h.from_id.as_ref().map(normalize_peer_and_type);
            saved_from_peer = h.saved_from_peer.as_ref().map(normalize_peer_and_type);
            from_name = h.from_name.clone();
            saved_from_name = h.saved_from_name.clone();
            channel_post = h.channel_post.map(|p| p as i64);
            saved_from_msg_id = h.saved_from_msg_id.map(|p| p as i64);
            post_author = h.post_author.clone();
            date = Some(h.date as i64);
            saved_date = h.saved_date.map(|d| d as i64);
        }

        if !found_in_tl
            && let Some(fj) = &msg.forward_json
            && let Ok(val) = serde_json::from_str::<serde_json::Value>(fj)
        {
            let v = val.get("Header").unwrap_or(&val);
            if from_peer.is_none()
                && let Some(fid) = v.get("from_id")
            {
                from_peer = parse_json_peer(fid);
            }
            if saved_from_peer.is_none()
                && let Some(sid) = v.get("saved_from_peer").or_else(|| v.get("saved_from_id"))
            {
                saved_from_peer = parse_json_peer(sid);
            }
            if from_name.is_none() {
                from_name = v
                    .get("from_name")
                    .and_then(|x| x.as_str())
                    .map(ToString::to_string);
            }
            if saved_from_name.is_none() {
                saved_from_name = v
                    .get("saved_from_name")
                    .and_then(|x| x.as_str())
                    .map(ToString::to_string);
            }
            if channel_post.is_none() {
                channel_post = v.get("channel_post").and_then(|x| x.as_i64());
            }
            if saved_from_msg_id.is_none() {
                saved_from_msg_id = v.get("saved_from_msg_id").and_then(|x| x.as_i64());
            }
            if post_author.is_none() {
                post_author = v
                    .get("post_author")
                    .and_then(|x| x.as_str())
                    .map(ToString::to_string);
            }
            if date.is_none() {
                date = v.get("date").and_then(|x| x.as_i64());
            }
            if saved_date.is_none() {
                saved_date = v.get("saved_date").and_then(|x| x.as_i64());
            }
        }

        if !found_in_tl && msg.forward_json.is_none() {
            return None;
        }

        let (source_peer_id, source_peer_type) = match from_peer.or(saved_from_peer) {
            Some((p, t)) => (Some(p), Some(t)),
            None => (None, None),
        };
        let origin_channel_post = channel_post.or(saved_from_msg_id);
        let origin_date = date.or(saved_date);
        let origin_signature = post_author;

        let mut origin_name: Option<String> = None;
        let mut source_username: Option<String> = None;
        let mut is_source_archived = false;
        let mut source_chat_url: Option<String> = None;
        let mut source_avatar_markup: Option<String> = None;

        if let Some(pid) = source_peer_id {
            if let Ok(Some(peer)) = self.db.get_peer(pid) {
                origin_name = peer.name;
                source_username = peer.username;
            }

            if exported_peer_ids.contains(&pid) {
                is_source_archived = true;
                source_chat_url = Some(ArchiveUrlBuilder::chat_root_url(2, pid));
            }

            let display_name_for_avatar = origin_name.as_deref().unwrap_or("Peer");
            source_avatar_markup = Some(crate::url_builder::render_avatar_markup(
                Some(pid),
                display_name_for_avatar,
                2,
                false,
                "fwd-avatar",
                available_avatars,
            ));
        }

        if origin_name.is_none() {
            origin_name = from_name.or(saved_from_name);
            if source_avatar_markup.is_none()
                && let Some(ref name) = origin_name
            {
                source_avatar_markup = Some(crate::url_builder::render_avatar_markup(
                    source_peer_id,
                    name,
                    2,
                    false,
                    "fwd-avatar",
                    available_avatars,
                ));
            }
        }

        if origin_name.is_none()
            && let Some(pid) = source_peer_id
        {
            let label = match source_peer_type {
                Some(PeerType::Channel) => format!("channel {}", pid.raw()),
                Some(PeerType::Group) => format!("group {}", pid.raw()),
                Some(PeerType::User) => format!("user {}", pid.raw()),
                _ => format!("peer {}", pid.raw()),
            };
            origin_name = Some(label);
            if source_avatar_markup.is_none() {
                source_avatar_markup = Some(crate::url_builder::render_avatar_markup(
                    Some(pid),
                    &format!("{}", pid.raw()),
                    2,
                    false,
                    "fwd-avatar",
                    available_avatars,
                ));
            }
        }

        Some(RenderForwardInfo {
            source_peer_id,
            source_peer_type,
            origin_name,
            source_username,
            origin_channel_post,
            origin_date,
            origin_signature,
            source_avatar_markup,
            is_source_archived,
            source_chat_url,
        })
    }

    fn materialize_media(
        &self,
        staging_dir: &Path,
        peers: &[PeerRecord],
        hasher: &mut Sha256,
    ) -> RenderResult<usize> {
        let export_media_dir = staging_dir.join("media");
        fs::create_dir_all(&export_media_dir)?;

        let Some(src_base_dir) = &self.options.media_src_dir else {
            return Ok(0);
        };

        let mut copied_count = 0;
        let mut processed_hashes = HashSet::new();

        for peer in peers {
            let mut offset = 0;
            const BATCH: usize = 500;
            loop {
                let msgs = self.db.list_messages_by_peer(peer.peer_id, BATCH, offset)?;
                if msgs.is_empty() {
                    break;
                }
                let len = msgs.len();
                for msg in msgs {
                    let media_list = self
                        .db
                        .get_media_for_message(msg.key.peer_id, msg.key.message_id)?;
                    for media in media_list {
                        if let Some(rel_path) = &media.local_rel_path {
                            if !processed_hashes.insert(media.media_id.clone()) {
                                continue;
                            }

                            let clean_rel_path = validate_and_clean_media_rel_path(rel_path)?;

                            let src_candidates = [
                                src_base_dir.join("media").join(&clean_rel_path),
                                src_base_dir.join(&clean_rel_path),
                            ];
                            let src_file_opt = src_candidates.into_iter().find(|p| p.exists());

                            let dst_file = export_media_dir.join(&clean_rel_path);

                            if !dst_file.starts_with(&export_media_dir) {
                                return Err(RenderError::UnsafePath(format!(
                                    "Destination path escapes media directory: {}",
                                    dst_file.display()
                                )));
                            }

                            if let Some(parent) = dst_file.parent() {
                                fs::create_dir_all(parent)?;
                            }

                            if let Some(src_file) = src_file_opt {
                                hasher.update(media.media_id.as_bytes());
                                if let Some(ref sh) = media.sha256 {
                                    hasher.update(sh.as_bytes());
                                }

                                materialize_file(&src_file, &dst_file, self.options.media_mode)?;
                                copied_count += 1;
                            }
                        }
                    }
                }
                if len < BATCH {
                    break;
                }
                offset += len;
            }
        }

        let export_avatars_dir = export_media_dir.join("avatars");
        fs::create_dir_all(&export_avatars_dir)?;

        let find_subdir = |sub: &str| {
            [src_base_dir.join("media").join(sub), src_base_dir.join(sub)]
                .into_iter()
                .find(|p| p.is_dir())
                .or_else(|| {
                    (src_base_dir.is_dir() && src_base_dir.ends_with(sub))
                        .then(|| src_base_dir.clone())
                })
        };

        if let Some(src_av_dir) = find_subdir("avatars") {
            copied_count += materialize_dir_contents(
                &src_av_dir,
                &export_avatars_dir,
                self.options.media_mode,
                hasher,
            );
        }

        let export_reactions_dir = export_media_dir.join("reactions");
        fs::create_dir_all(&export_reactions_dir)?;

        if let Some(src_rx_dir) = find_subdir("reactions") {
            copied_count += materialize_dir_contents(
                &src_rx_dir,
                &export_reactions_dir,
                self.options.media_mode,
                hasher,
            );
        }

        Ok(copied_count)
    }
}

fn materialize_file(src: &Path, dst: &Path, mode: MediaMode) -> RenderResult<()> {
    match mode {
        MediaMode::Copy => {
            fs::copy(src, dst)?;
        }
        MediaMode::Link => {
            #[cfg(unix)]
            {
                let canonical_src = fs::canonicalize(src).unwrap_or_else(|_| src.to_path_buf());
                if fs::symlink_metadata(dst).is_ok() {
                    let _ = fs::remove_file(dst);
                }
                std::os::unix::fs::symlink(&canonical_src, dst)?;
            }
            #[cfg(not(unix))]
            {
                fs::copy(src, dst)?;
            }
        }
    }
    Ok(())
}

fn materialize_dir_contents(
    src_dir: &Path,
    dst_dir: &Path,
    mode: MediaMode,
    hasher: &mut Sha256,
) -> usize {
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(src_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && let Some(name) = path.file_name()
            {
                let dst = dst_dir.join(name);
                hasher.update(name.as_encoded_bytes());
                if materialize_file(&path, &dst, mode).is_ok() {
                    count += 1;
                }
            }
        }
    }
    count
}

fn format_service_action(action: &tl::enums::MessageAction) -> String {
    match action {
        tl::enums::MessageAction::Empty => "Empty service event".to_string(),
        tl::enums::MessageAction::ChatCreate(_) => "Group chat created".to_string(),
        tl::enums::MessageAction::ChatEditTitle(t) => {
            format!("Changed group title to \"{}\"", t.title)
        }
        tl::enums::MessageAction::ChatEditPhoto(_) => "Changed group photo".to_string(),
        tl::enums::MessageAction::ChatDeletePhoto => "Removed group photo".to_string(),
        tl::enums::MessageAction::ChatAddUser(_) => "Added user to group".to_string(),
        tl::enums::MessageAction::ChatDeleteUser(_) => "Removed user from group".to_string(),
        tl::enums::MessageAction::ChatJoinedByLink(_) => "Joined group via invite link".to_string(),
        tl::enums::MessageAction::ChannelCreate(_) => "Channel created".to_string(),
        tl::enums::MessageAction::PinMessage => "Pinned a message".to_string(),
        tl::enums::MessageAction::ContactSignUp => "Joined Telegram".to_string(),
        tl::enums::MessageAction::PhoneCall(_) => "Phone call".to_string(),
        tl::enums::MessageAction::GameScore(g) => format!("Scored {} in game", g.score),
        tl::enums::MessageAction::PaymentSent(p) => {
            format!("Payment of {} {}", p.total_amount, p.currency)
        }
        tl::enums::MessageAction::PaymentSentMe(_) => "Received payment".to_string(),
        tl::enums::MessageAction::ScreenshotTaken => "Took a screenshot".to_string(),
        tl::enums::MessageAction::CustomAction(a) => a.message.clone(),
        tl::enums::MessageAction::BotAllowed(b) => {
            if let Some(domain) = &b.domain {
                format!("Allowed bot from domain: {domain}")
            } else {
                "Allowed bot".to_string()
            }
        }
        tl::enums::MessageAction::SecureValuesSent(_) => {
            "Sent Telegram Passport values".to_string()
        }
        tl::enums::MessageAction::SecureValuesSentMe(_) => {
            "Received Telegram Passport values".to_string()
        }
        tl::enums::MessageAction::GeoProximityReached(_) => "Proximity alert reached".to_string(),
        tl::enums::MessageAction::GroupCall(_) => "Group voice/video call event".to_string(),
        tl::enums::MessageAction::InviteToGroupCall(_) => "Invited users to voice chat".to_string(),
        tl::enums::MessageAction::SetMessagesTtl(t) => {
            format!("Set auto-delete timer to {} seconds", t.period)
        }
        tl::enums::MessageAction::GroupCallScheduled(s) => {
            format!("Voice chat scheduled for timestamp {}", s.schedule_date)
        }
        tl::enums::MessageAction::SetChatTheme(_) => "Changed chat theme".to_string(),
        tl::enums::MessageAction::ChatJoinedByRequest => "Joined chat via join request".to_string(),
        tl::enums::MessageAction::WebViewDataSent(w) => {
            format!("Sent web app data: {}", w.text)
        }
        tl::enums::MessageAction::GiftCode(_) => "Telegram Premium gift code".to_string(),
        tl::enums::MessageAction::TopicCreate(t) => format!("Created topic \"{}\"", t.title),
        tl::enums::MessageAction::TopicEdit(t) => {
            if let Some(title) = &t.title {
                format!("Edited topic title to \"{title}\"")
            } else {
                "Edited topic".to_string()
            }
        }
        tl::enums::MessageAction::SuggestProfilePhoto(_) => "Suggested profile photo".to_string(),
        tl::enums::MessageAction::RequestedPeer(_) => "Shared requested peer".to_string(),
        tl::enums::MessageAction::SetChatWallPaper(_) => "Changed chat wallpaper".to_string(),
        tl::enums::MessageAction::StarGift(_) => "Telegram Star gift".to_string(),
        tl::enums::MessageAction::StarGiftUnique(_) => "Unique Telegram Star gift".to_string(),
        tl::enums::MessageAction::ChatMigrateTo(_) => "Migrated group to supergroup".to_string(),
        tl::enums::MessageAction::ChannelMigrateFrom(_) => {
            "Channel migrated from group".to_string()
        }
        tl::enums::MessageAction::HistoryClear => "Cleared chat history".to_string(),
        tl::enums::MessageAction::GiftPremium(_) => "Gifted Telegram Premium".to_string(),
        tl::enums::MessageAction::GiveawayLaunch(_) => "Launched giveaway".to_string(),
        tl::enums::MessageAction::GiveawayResults(_) => "Giveaway results".to_string(),
        tl::enums::MessageAction::PrizeStars(_) => "Won Telegram Stars prize".to_string(),
        _ => "Service event".to_string(),
    }
}

fn normalize_peer_and_type(peer: &tl::enums::Peer) -> (PeerId, PeerType) {
    match peer {
        tl::enums::Peer::User(u) => (PeerId::new(u.user_id), PeerType::User),
        tl::enums::Peer::Chat(c) => (PeerId::new(-c.chat_id), PeerType::Group),
        tl::enums::Peer::Channel(c) => (
            PeerId::new(-1_000_000_000_000 - c.channel_id),
            PeerType::Channel,
        ),
    }
}

fn parse_json_peer(val: &serde_json::Value) -> Option<(PeerId, PeerType)> {
    if let Some(uid) = val
        .get("user_id")
        .or_else(|| val.get("User").and_then(|x| x.get("user_id")))
        .and_then(|x| x.as_i64())
    {
        return Some((PeerId::new(uid), PeerType::User));
    }
    if let Some(cid) = val
        .get("channel_id")
        .or_else(|| val.get("Channel").and_then(|x| x.get("channel_id")))
        .and_then(|x| x.as_i64())
    {
        let pid = if cid > 0 {
            -1_000_000_000_000 - cid
        } else {
            cid
        };
        return Some((PeerId::new(pid), PeerType::Channel));
    }
    if let Some(chid) = val
        .get("chat_id")
        .or_else(|| val.get("Chat").and_then(|x| x.get("chat_id")))
        .and_then(|x| x.as_i64())
    {
        let pid = if chid > 0 { -chid } else { chid };
        return Some((PeerId::new(pid), PeerType::Group));
    }
    if let Some(raw) = val.as_i64() {
        let pt = if raw < -1_000_000_000_000 {
            PeerType::Channel
        } else if raw < 0 {
            PeerType::Group
        } else {
            PeerType::User
        };
        return Some((PeerId::new(raw), pt));
    }
    None
}
