use std::{collections::HashSet, fs, path::Path};

use sha2::{Digest, Sha256};
use vendetta_model::PeerRecord;
use vendetta_storage::ArchiveDb;

use crate::{
    error::{RenderError, RenderResult},
    media::validate_and_clean_media_rel_path,
    model::MediaMode,
};

pub fn materialize_media(
    db: &ArchiveDb,
    staging_dir: &Path,
    peers: &[PeerRecord],
    media_src_dir: Option<&Path>,
    media_mode: MediaMode,
    hasher: &mut Sha256,
) -> RenderResult<usize> {
    let export_media_dir = staging_dir.join("media");
    fs::create_dir_all(&export_media_dir)?;

    let Some(src_base_dir) = media_src_dir else {
        return Ok(0);
    };

    let mut copied_count = 0;
    let mut processed_hashes = HashSet::new();

    for peer in peers {
        let mut offset = 0;
        const BATCH: usize = 500;
        loop {
            let msgs = db.list_messages_by_peer(peer.peer_id, BATCH, offset)?;
            if msgs.is_empty() {
                break;
            }
            let len = msgs.len();
            for msg in msgs {
                let media_list = db.get_media_for_message(msg.key.peer_id, msg.key.message_id)?;
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

                            materialize_file(&src_file, &dst_file, media_mode)?;
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

    let find_subdir = |sub: &str| {
        [src_base_dir.join("media").join(sub), src_base_dir.join(sub)]
            .into_iter()
            .find(|p| p.is_dir())
            .or_else(|| {
                (src_base_dir.is_dir() && src_base_dir.ends_with(sub))
                    .then(|| src_base_dir.to_path_buf())
            })
    };

    for sub in ["avatars", "reactions", "icons"] {
        let export_sub_dir = export_media_dir.join(sub);
        fs::create_dir_all(&export_sub_dir)?;
        if let Some(src_sub_dir) = find_subdir(sub) {
            copied_count +=
                materialize_dir_contents(&src_sub_dir, &export_sub_dir, media_mode, hasher);
        }
    }

    Ok(copied_count)
}

pub fn materialize_file(src: &Path, dst: &Path, mode: MediaMode) -> RenderResult<()> {
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

pub fn materialize_dir_contents(
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

#[derive(Debug, Clone, Default)]
pub struct ChatMediaManifest {
    pub media_files: Vec<String>,
    pub avatar_files: Vec<String>,
    pub reaction_files: Vec<String>,
    pub topic_assets: Vec<String>,
    pub total_copied: usize,
}

#[allow(clippy::too_many_arguments)]
pub fn materialize_chat_scope(
    db: &ArchiveDb,
    chat_dir: &Path,
    peer_id: vendetta_model::PeerId,
    message_ids: &[vendetta_model::MessageId],
    media_src_dir: Option<&Path>,
    media_mode: MediaMode,
    participants: &HashSet<vendetta_model::PeerId>,
    reaction_doc_ids: &HashSet<i64>,
    topic_icon_files: &[String],
    hasher: &mut Sha256,
) -> RenderResult<ChatMediaManifest> {
    crate::assets::write_all_assets(chat_dir)?;

    let media_dir = chat_dir.join("media");
    let avatars_dir = chat_dir.join("avatars");
    let reactions_dir = chat_dir.join("reactions");
    fs::create_dir_all(&media_dir)?;
    fs::create_dir_all(&avatars_dir)?;
    fs::create_dir_all(&reactions_dir)?;

    if !topic_icon_files.is_empty() {
        let topic_assets_dir = chat_dir.join("topics/assets");
        fs::create_dir_all(&topic_assets_dir)?;
    }

    let mut manifest = ChatMediaManifest::default();
    let Some(src_base_dir) = media_src_dir else {
        return Ok(manifest);
    };

    let mut processed_media_ids = HashSet::new();

    for mid in message_ids {
        if let Ok(media_list) = db.get_media_for_message(peer_id, *mid) {
            for m in media_list {
                if let Some(rel_path) = &m.local_rel_path {
                    if !processed_media_ids.insert(m.media_id.clone()) {
                        continue;
                    }

                    if let Ok(clean_rel_path) = validate_and_clean_media_rel_path(rel_path) {
                        let file_rel = clean_rel_path
                            .strip_prefix("media/")
                            .unwrap_or(&clean_rel_path);
                        let dst_file = media_dir.join(file_rel);

                        if !dst_file.starts_with(&media_dir) {
                            continue;
                        }

                        let src_candidates = [
                            src_base_dir.join("media").join(file_rel),
                            src_base_dir.join(file_rel),
                            src_base_dir.join(&clean_rel_path),
                        ];

                        if let Some(src_file) = src_candidates.into_iter().find(|p| p.is_file()) {
                            if let Some(parent) = dst_file.parent() {
                                let _ = fs::create_dir_all(parent);
                            }
                            if materialize_file(&src_file, &dst_file, media_mode).is_ok() {
                                hasher.update(m.media_id.as_bytes());
                                if let Some(ref sh) = m.sha256 {
                                    hasher.update(sh.as_bytes());
                                }
                                manifest
                                    .media_files
                                    .push(file_rel.to_string_lossy().to_string());
                                manifest.total_copied += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    for pid in participants {
        let token = crate::url_builder::ArchiveUrlBuilder::peer_token(*pid);
        let avatar_name = format!("{token}.jpg");
        let dst_file = avatars_dir.join(&avatar_name);

        let src_candidates = [
            src_base_dir.join("avatars").join(&avatar_name),
            src_base_dir.join("media/avatars").join(&avatar_name),
            src_base_dir.join(&avatar_name),
        ];

        if let Some(src_file) = src_candidates.into_iter().find(|p| p.is_file())
            && materialize_file(&src_file, &dst_file, media_mode).is_ok()
        {
            hasher.update(avatar_name.as_bytes());
            manifest.avatar_files.push(avatar_name);
            manifest.total_copied += 1;
        }
    }

    for doc_id in reaction_doc_ids {
        let rx_name = format!("{doc_id}.webp");
        let dst_file = reactions_dir.join(&rx_name);

        let src_candidates = [
            src_base_dir.join("reactions").join(&rx_name),
            src_base_dir.join("media/reactions").join(&rx_name),
            src_base_dir.join(&rx_name),
        ];

        if let Some(src_file) = src_candidates.into_iter().find(|p| p.is_file())
            && materialize_file(&src_file, &dst_file, media_mode).is_ok()
        {
            hasher.update(rx_name.as_bytes());
            manifest.reaction_files.push(rx_name);
            manifest.total_copied += 1;
        }
    }

    if !topic_icon_files.is_empty() {
        let topic_assets_dir = chat_dir.join("topics/assets");
        for icon_file in topic_icon_files {
            let clean_name = icon_file
                .trim_start_matches('/')
                .strip_prefix("assets/")
                .unwrap_or(icon_file);
            let dst_file = topic_assets_dir.join(clean_name);

            let src_candidates = [
                src_base_dir.join("topics/assets").join(clean_name),
                src_base_dir.join("icons").join(clean_name),
                src_base_dir.join("media/icons").join(clean_name),
                src_base_dir.join(clean_name),
            ];

            if let Some(src_file) = src_candidates.into_iter().find(|p| p.is_file()) {
                if let Some(parent) = dst_file.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if materialize_file(&src_file, &dst_file, media_mode).is_ok() {
                    hasher.update(clean_name.as_bytes());
                    manifest.topic_assets.push(clean_name.to_string());
                    manifest.total_copied += 1;
                }
            }
        }
    }

    manifest.media_files.sort();
    manifest.avatar_files.sort();
    manifest.reaction_files.sort();
    manifest.topic_assets.sort();

    Ok(manifest)
}
