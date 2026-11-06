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

fn user_config_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/vendetta/config.json"))
}

impl CliConfig {
    pub fn candidate_config_paths() -> Vec<PathBuf> {
        let mut paths = vec![PathBuf::from("vendetta.json")];
        if let Some(p) = user_config_path() {
            paths.push(p);
        }
        paths
    }

    pub fn resolve_write_config_path(explicit: Option<&Path>) -> PathBuf {
        if let Some(p) = explicit {
            p.to_path_buf()
        } else if Path::new("vendetta.json").exists() {
            PathBuf::from("vendetta.json")
        } else {
            user_config_path().unwrap_or_else(|| PathBuf::from("vendetta.json"))
        }
    }

    pub fn load_file_or_default(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    }

    pub fn from_env() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|s| !s.trim().is_empty());
        let var_any = |keys: &[&str]| keys.iter().find_map(|&k| var(k));

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

    pub fn merge(&mut self, o: CliConfig) {
        self.api_id = o.api_id.or(self.api_id);
        self.api_hash = o.api_hash.or(self.api_hash.take());
        self.account = o.account.or(self.account.take());
        self.archive = o.archive.or(self.archive.take());
        self.session = o.session.or(self.session.take());
        self.media_dir = o.media_dir.or(self.media_dir.take());
        self.output = o.output.or(self.output.take());
        self.base_dir = o.base_dir.or(self.base_dir.take());
    }

    pub fn load(explicit_path: Option<&Path>) -> Self {
        let mut config = Self::default();
        let paths = explicit_path
            .map(|p| vec![p.to_path_buf()])
            .unwrap_or_else(Self::candidate_config_paths);

        for path in paths {
            if let Ok(c) = std::fs::read_to_string(&path)
                && let Ok(parsed) = serde_json::from_str::<CliConfig>(&c)
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
        if let Ok(meta) = std::fs::metadata(path)
            && meta.permissions().mode() & 0o077 != 0
        {
            tracing::warn!(
                "Config file {} has permissive permissions ({:03o}). Recommended: chmod 600 {}",
                path.display(),
                meta.permissions().mode() & 0o777,
                path.display()
            );
        }
    }

    pub fn apply_account_override(&mut self, cli_account: Option<impl Into<String>>) {
        if let Some(acc) = cli_account {
            self.account = Some(acc.into());
        }
    }

    pub fn resolve_credentials(
        &self,
        id: Option<i32>,
        hash: Option<String>,
    ) -> (Option<i32>, Option<String>) {
        (id.or(self.api_id), hash.or_else(|| self.api_hash.clone()))
    }

    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }
        let json = format!("{}\n", serde_json::to_string_pretty(self)?);

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)
                .with_context(|| format!("Failed to create config file at {}", path.display()))?;
            f.write_all(json.as_bytes())
                .with_context(|| format!("Failed to write config to {}", path.display()))?;
        }

        #[cfg(not(unix))]
        std::fs::write(path, json)
            .with_context(|| format!("Failed to write config to {}", path.display()))?;

        Ok(())
    }

    fn app_config(&self) -> vendetta_core::AppConfig {
        vendetta_core::AppConfig::new(self.base_dir.clone().unwrap_or_else(|| PathBuf::from(".")))
    }

    pub fn resolve_session_path(
        &self,
        cli_session: Option<PathBuf>,
        cli_account: Option<&str>,
    ) -> PathBuf {
        let account = cli_account.or(self.account.as_deref()).unwrap_or("default");
        if let Some(s) = cli_session {
            let is_cred = s.is_file()
                && std::fs::read_to_string(&s)
                    .ok()
                    .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                    .is_some_and(|v| {
                        v.get("home_dc").is_none()
                            && (v.get("api_id").is_some() || v.get("api_hash").is_some())
                    });
            if !is_cred {
                return s;
            }
        }
        self.session
            .clone()
            .unwrap_or_else(|| self.app_config().session_file(account))
    }

    pub fn resolve_archive_path(
        &self,
        cli_archive: Option<PathBuf>,
        cli_account: Option<&str>,
    ) -> PathBuf {
        let account = cli_account.or(self.account.as_deref()).unwrap_or("default");
        cli_archive
            .or_else(|| self.archive.clone())
            .unwrap_or_else(|| self.app_config().archive_db_path(account))
    }

    pub fn resolve_media_dir(&self, cli_media: Option<PathBuf>, archive_path: &Path) -> PathBuf {
        cli_media
            .or_else(|| self.media_dir.clone())
            .unwrap_or_else(|| {
                archive_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join("media")
            })
    }
}
