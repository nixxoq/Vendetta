use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

/// Layout manager for account credentials and archives.
///
/// Directory structure:
/// base_dir/
///   accounts/
///     <account_name>/
///       session.json
///   archives/
///     <archive_name>/
///       archive.db
///       manifest.json
///       media/
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub base_dir: PathBuf,
}

impl AppConfig {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub fn accounts_dir(&self) -> PathBuf {
        self.base_dir.join("accounts")
    }

    pub fn account_dir(&self, account_name: &str) -> PathBuf {
        self.accounts_dir().join(account_name)
    }

    pub fn session_file(&self, account_name: &str) -> PathBuf {
        self.account_dir(account_name).join("session.json")
    }

    pub fn archives_dir(&self) -> PathBuf {
        self.base_dir.join("archives")
    }

    pub fn archive_dir(&self, archive_name: &str) -> PathBuf {
        self.archives_dir().join(archive_name)
    }

    pub fn archive_db_path(&self, archive_name: &str) -> PathBuf {
        self.archive_dir(archive_name).join("archive.db")
    }

    pub fn archive_media_dir(&self, archive_name: &str) -> PathBuf {
        self.archive_dir(archive_name).join("media")
    }

    pub fn archive_manifest_path(&self, archive_name: &str) -> PathBuf {
        self.archive_dir(archive_name).join("manifest.json")
    }

    pub fn ensure_account_dir(&self, account_name: &str) -> io::Result<PathBuf> {
        let dir = self.account_dir(account_name);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    pub fn ensure_archive_dir(&self, archive_name: &str) -> io::Result<PathBuf> {
        let dir = self.archive_dir(archive_name);
        fs::create_dir_all(self.archive_media_dir(archive_name))?;
        Ok(dir)
    }
}

impl<P: Into<PathBuf>> From<P> for AppConfig {
    fn from(path: P) -> Self {
        Self::new(path)
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self::new(".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_config_generates_correct_paths() {
        let config = AppConfig::new("/tmp/vendetta_test");
        assert_eq!(
            config.accounts_dir(),
            PathBuf::from("/tmp/vendetta_test/accounts")
        );
        assert_eq!(
            config.session_file("my_acc"),
            PathBuf::from("/tmp/vendetta_test/accounts/my_acc/session.json")
        );
        assert_eq!(
            config.archive_db_path("my_chat"),
            PathBuf::from("/tmp/vendetta_test/archives/my_chat/archive.db")
        );
        assert_eq!(
            config.archive_media_dir("my_chat"),
            PathBuf::from("/tmp/vendetta_test/archives/my_chat/media")
        );
        assert_eq!(
            config.archive_manifest_path("my_chat"),
            PathBuf::from("/tmp/vendetta_test/archives/my_chat/manifest.json")
        );
    }
}
