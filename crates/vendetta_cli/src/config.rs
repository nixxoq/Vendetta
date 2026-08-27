use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CliConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_dir: Option<PathBuf>,
}

impl CliConfig {
    pub fn default_config_path() -> PathBuf {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".config/vendetta/config.json"))
            .unwrap_or_else(|| PathBuf::from("vendetta.json"))
    }

    pub fn candidate_config_paths() -> Vec<PathBuf> {
        let mut paths = vec![PathBuf::from("vendetta.json")];
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(PathBuf::from(home).join(".config/vendetta/config.json"));
        }
        paths
    }

    pub fn resolve_write_config_path(explicit_config_path: Option<&Path>) -> PathBuf {
        if let Some(p) = explicit_config_path {
            p.to_path_buf()
        } else if Path::new("vendetta.json").exists() {
            PathBuf::from("vendetta.json")
        } else {
            Self::default_config_path()
        }
    }

    pub fn load_file_or_default(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|s| !s.trim().is_empty());
        let var_any = |names: &[&str]| names.iter().find_map(|&name| var(name));

        Self {
            api_id: var_any(&["VENDETTA_API_ID", "TG_API_ID"]).and_then(|v| v.parse().ok()),
            api_hash: var_any(&["VENDETTA_API_HASH", "TG_API_HASH"]),
            account: var("VENDETTA_ACCOUNT"),
            archive: var("VENDETTA_ARCHIVE").map(PathBuf::from),
            session: var("VENDETTA_SESSION").map(PathBuf::from),
            media_dir: var("VENDETTA_MEDIA_DIR").map(PathBuf::from),
            output: var("VENDETTA_OUTPUT").map(PathBuf::from),
            base_dir: var("VENDETTA_BASE_DIR").map(PathBuf::from),
        }
    }

    pub fn merge(&mut self, other: CliConfig) {
        if other.api_id.is_some() {
            self.api_id = other.api_id;
        }
        if other.api_hash.is_some() {
            self.api_hash = other.api_hash;
        }
        if other.account.is_some() {
            self.account = other.account;
        }
        if other.archive.is_some() {
            self.archive = other.archive;
        }
        if other.session.is_some() {
            self.session = other.session;
        }
        if other.media_dir.is_some() {
            self.media_dir = other.media_dir;
        }
        if other.output.is_some() {
            self.output = other.output;
        }
        if other.base_dir.is_some() {
            self.base_dir = other.base_dir;
        }
    }

    /// Defaults -> Config File -> Environment
    pub fn load(explicit_config_path: Option<&Path>) -> Self {
        let mut config = Self::default();

        let paths = match explicit_config_path {
            Some(p) => vec![p.to_path_buf()],
            None => Self::candidate_config_paths(),
        };

        for path in paths {
            if let Ok(content) = std::fs::read_to_string(&path)
                && let Ok(parsed) = serde_json::from_str::<CliConfig>(&content)
            {
                #[cfg(unix)]
                if parsed.api_hash.is_some() {
                    Self::warn_if_insecure_permissions(&path);
                }

                config.merge(parsed);
                break;
            }
        }

        config.merge(Self::from_env());
        config
    }

    #[cfg(unix)]
    fn warn_if_insecure_permissions(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                tracing::warn!(
                    "Config file {} has permissive permissions ({:03o}). Recommended: chmod 600 {}",
                    path.display(),
                    mode & 0o777,
                    path.display()
                );
            }
        }
    }

    pub fn apply_account_override(&mut self, cli_account: Option<impl Into<String>>) {
        if let Some(acc) = cli_account {
            self.account = Some(acc.into());
        }
    }

    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }

        let json_str = serde_json::to_string_pretty(self)
            .context("Failed to serialize configuration to JSON")?;

        #[cfg(unix)]
        {
            use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt};

            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)
                .with_context(|| format!("Failed to create config file at {}", path.display()))?;

            file.write_all(json_str.as_bytes())
                .and_then(|_| file.write_all(b"\n"))
                .with_context(|| format!("Failed to write config to {}", path.display()))?;
        }

        #[cfg(not(unix))]
        {
            std::fs::write(path, format!("{}\n", json_str))
                .with_context(|| format!("Failed to write config to {}", path.display()))?;
        }

        Ok(())
    }

    pub fn resolve_session_path(
        &self,
        cli_session: Option<PathBuf>,
        cli_account: Option<&str>,
    ) -> PathBuf {
        if let Some(s) = cli_session {
            return s;
        }
        if let Some(ref s) = self.session {
            return s.clone();
        }
        let account = cli_account.or(self.account.as_deref()).unwrap_or("default");

        let base = self.base_dir.clone().unwrap_or_else(|| PathBuf::from("."));
        let app_config = vendetta_core::AppConfig::new(base);
        app_config.session_file(account)
    }

    pub fn resolve_archive_path(
        &self,
        cli_archive: Option<PathBuf>,
        cli_account: Option<&str>,
    ) -> PathBuf {
        if let Some(a) = cli_archive {
            return a;
        }
        if let Some(ref a) = self.archive {
            return a.clone();
        }
        let name = cli_account.or(self.account.as_deref()).unwrap_or("default");

        let base = self.base_dir.clone().unwrap_or_else(|| PathBuf::from("."));
        let app_config = vendetta_core::AppConfig::new(base);
        app_config.archive_db_path(name)
    }

    pub fn resolve_media_dir(&self, cli_media: Option<PathBuf>, archive_path: &Path) -> PathBuf {
        if let Some(m) = cli_media {
            return m;
        }
        if let Some(ref m) = self.media_dir {
            return m.clone();
        }
        archive_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("media")
    }

    pub fn sanitized_view(&self) -> SanitizedConfigView {
        SanitizedConfigView {
            api_id: self.api_id,
            api_hash_configured: self.api_hash.is_some(),
            api_hash_redacted: self.api_hash.as_ref().map(|_| "*** (REDACTED)".to_string()),
            account: self.account.clone(),
            archive: self.archive.clone(),
            session: self.session.clone(),
            media_dir: self.media_dir.clone(),
            output: self.output.clone(),
            base_dir: self.base_dir.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanitizedConfigView {
    pub api_id: Option<i32>,
    pub api_hash_configured: bool,
    pub api_hash_redacted: Option<String>,
    pub account: Option<String>,
    pub archive: Option<PathBuf>,
    pub session: Option<PathBuf>,
    pub media_dir: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub base_dir: Option<PathBuf>,
}
