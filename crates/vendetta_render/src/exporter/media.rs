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
