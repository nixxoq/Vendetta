use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use vendetta_core::sanitize_file_name;
use vendetta_model::PeerId;

#[derive(Debug, Clone)]
pub struct StorageLayoutManager {
    base_dir: PathBuf,
}

impl StorageLayoutManager {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    pub fn media_dir(&self) -> PathBuf {
        if self.base_dir.ends_with("media") {
            self.base_dir.clone()
        } else {
            self.base_dir.join("media")
        }
    }

    pub fn avatars_dir(&self) -> PathBuf {
        self.media_dir().join("avatars")
    }

    pub fn reactions_dir(&self) -> PathBuf {
        self.media_dir().join("reactions")
    }

    pub fn temp_dir(&self) -> PathBuf {
        if self.base_dir.ends_with("media") {
            self.base_dir
                .parent()
                .unwrap_or(Path::new("."))
                .join("media-tmp")
        } else {
            self.base_dir.join("media-tmp")
        }
    }

    pub fn ensure_dirs(&self) -> io::Result<()> {
        fs::create_dir_all(self.avatars_dir())?;
        fs::create_dir_all(self.reactions_dir())?;
        fs::create_dir_all(self.temp_dir())?;
        Ok(())
    }

    pub fn reaction_rel_path(document_id: i64) -> String {
        format!("media/reactions/{document_id}.webp")
    }

    pub fn reaction_path(&self, document_id: i64) -> PathBuf {
        self.reactions_dir().join(format!("{document_id}.webp"))
    }

    pub fn temp_part_path(&self, media_id: &str) -> PathBuf {
        let safe_id = sanitize_file_name(media_id);
        self.temp_dir().join(format!("{safe_id}.part"))
    }

    pub fn content_addressed_rel_path(sha256: &str, file_name: Option<&str>) -> String {
        let prefix = if sha256.len() >= 2 {
            &sha256[..2]
        } else {
            "00"
        };

        let ext = file_name
            .and_then(|name| Path::new(name).extension())
            .and_then(|e| e.to_str())
            .map(sanitize_file_name)
            .filter(|e| !e.is_empty());

        if let Some(e) = ext {
            format!("media/{prefix}/{sha256}.{e}")
        } else {
            format!("media/{prefix}/{sha256}")
        }
    }

    pub fn avatar_rel_path(peer_id: PeerId) -> String {
        format!(
            "media/avatars/{}",
            crate::avatar::peer_avatar_file_name(peer_id)
        )
    }

    pub fn avatar_path(&self, peer_id: PeerId) -> PathBuf {
        self.avatars_dir()
            .join(crate::avatar::peer_avatar_file_name(peer_id))
    }

    pub fn resolve_canonical_path(&self, rel_path: &str) -> PathBuf {
        let clean_rel = rel_path.strip_prefix("media/").unwrap_or(rel_path);
        self.media_dir().join(clean_rel)
    }

    pub fn resolve_path(&self, rel_path: &str) -> PathBuf {
        let canonical = self.resolve_canonical_path(rel_path);
        if canonical.exists() {
            return canonical;
        }

        let clean_rel = rel_path.strip_prefix("media/").unwrap_or(rel_path);
        let legacy = self.media_dir().join("media").join(clean_rel);
        if legacy.exists() {
            return legacy;
        }

        canonical
    }

    pub fn finalize_temp_file(
        &self,
        temp_path: &Path,
        final_dest: &Path,
        expected_hash: &str,
        expected_size: i64,
    ) -> io::Result<()> {
        if let Some(parent) = final_dest.parent() {
            fs::create_dir_all(parent)?;
        }

        let check_existing_valid = |path: &Path| -> bool {
            if let Ok(mut dest_file) = File::open(path)
                && let Ok(metadata) = dest_file.metadata()
                && metadata.len() as i64 == expected_size
            {
                let mut dest_hasher = Sha256::new();
                let mut buf = [0u8; 65536];
                while let Ok(n) = dest_file.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    dest_hasher.update(&buf[..n]);
                }
                return format!("{:x}", dest_hasher.finalize()) == expected_hash;
            }
            false
        };

        if final_dest.exists() {
            if check_existing_valid(final_dest) {
                let _ = fs::remove_file(temp_path);
                return Ok(());
            }
            let _ = fs::remove_file(final_dest);
        }

        match fs::rename(temp_path, final_dest) {
            Ok(()) => Ok(()),
            Err(e) => {
                if final_dest.exists() && check_existing_valid(final_dest) {
                    let _ = fs::remove_file(temp_path);
                    Ok(())
                } else {
                    Err(e)
                }
            }
        }
    }
}
