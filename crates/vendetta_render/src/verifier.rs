use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
};

use crate::{
    error::{RenderError, RenderResult},
    manifest::HtmlExportManifest,
    search::shard_writer::{SearchManifest, SearchShard},
};

#[derive(Debug, Clone, Default)]
pub struct VerificationReport {
    pub total_pages_checked: usize,
    pub total_anchors_checked: usize,
    pub total_links_checked: usize,
    pub total_media_checked: usize,
    pub total_search_entries_checked: usize,
    pub errors: Vec<String>,
}

impl VerificationReport {
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }
}

pub struct HtmlArchiveVerifier {
    export_dir: PathBuf,
}

impl HtmlArchiveVerifier {
    pub fn new(export_dir: impl Into<PathBuf>) -> Self {
        Self {
            export_dir: export_dir.into(),
        }
    }

    pub fn verify(&self) -> RenderResult<VerificationReport> {
        let mut report = VerificationReport::default();

        if !self.export_dir.exists() {
            return Err(RenderError::VerificationFailed(format!(
                "Export directory does not exist: {}",
                self.export_dir.display()
            )));
        }

        let export_canonical = normalize_path(&self.export_dir);

        let manifest_path = self.export_dir.join("manifest.json");
        let manifest = if !manifest_path.exists() {
            report.errors.push("Missing manifest.json".to_string());
            None
        } else {
            match HtmlExportManifest::read_from_file(&manifest_path) {
                Ok(m) => {
                    if m.source_fingerprint.source_digest.is_empty() {
                        report
                            .errors
                            .push("manifest.json has empty source_fingerprint digest".to_string());
                    }
                    if m.export_config_fingerprint.is_empty() {
                        report
                            .errors
                            .push("manifest.json has empty export_config_fingerprint".to_string());
                    }
                    Some(m)
                }
                Err(e) => {
                    report.errors.push(format!("Corrupted manifest.json: {e}"));
                    None
                }
            }
        };

        let index_path = self.export_dir.join("index.html");
        if !index_path.exists() {
            report.errors.push("Missing root index.html".to_string());
        }

        let required_assets = [
            "assets/css/theme.css",
            "assets/css/main.css",
            "assets/css/telegram_like.css",
            "assets/css/archive_dense.css",
            "assets/js/app.js",
            "assets/js/lightbox.js",
            "assets/js/search.js",
            "assets/icons/symbols.svg",
        ];
        for asset in required_assets {
            if !self.export_dir.join(asset).exists() {
                report.errors.push(format!("Missing asset file: {asset}"));
            }
        }

        let search_manifest_path = self.export_dir.join("search/manifest.js");
        if search_manifest_path.exists() {
            self.verify_search_subsystem(&mut report)?;
        } else if let Some(ref m) = manifest
            && m.summary.search_shards_count > 0
        {
            report.errors.push("Missing search/manifest.js".to_string());
        }

        let mut global_anchors = HashSet::new();
        let mut global_message_anchors = HashSet::new();
        let mut all_chat_pages = HashSet::new();
        let mut discovered_dialogs = 0;
        let mut pending_links = Vec::new();

        let symbols_path = self.export_dir.join("assets/icons/symbols.svg");
        if let Ok(sym_content) = fs::read_to_string(&symbols_path) {
            for part in sym_content.split(" id=\"").skip(1) {
                if let Some((id_val, _)) = part.split_once('"') {
                    global_anchors.insert(id_val.to_string());
                }
            }
        }

        let chats_dir = self.export_dir.join("chats");
        if chats_dir.exists() {
            let chat_entries = fs::read_dir(&chats_dir)?;
            for chat_entry in chat_entries.flatten() {
                if chat_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    discovered_dialogs += 1;
                    let page_entries = fs::read_dir(chat_entry.path())?;
                    for page in page_entries.flatten() {
                        let path = page.path();
                        if path.extension().and_then(|s| s.to_str()) == Some("html") {
                            report.total_pages_checked += 1;
                            all_chat_pages.insert(path.clone());
                            self.scan_html_file(
                                &path,
                                &mut global_anchors,
                                &mut global_message_anchors,
                                &mut pending_links,
                                &mut report,
                                manifest.as_ref().map(|m| m.media_mode.as_str()),
                            )?;
                        }
                    }
                }
            }
        }

        if index_path.exists() {
            report.total_pages_checked += 1;
            self.scan_html_file(
                &index_path,
                &mut global_anchors,
                &mut global_message_anchors,
                &mut pending_links,
                &mut report,
                manifest.as_ref().map(|m| m.media_mode.as_str()),
            )?;
        }

        if let Some(ref m) = manifest {
            if m.summary.chunks_count != all_chat_pages.len() {
                report.errors.push(format!(
                    "Chunk count mismatch: manifest expected {} chunk pages, but found {}",
                    m.summary.chunks_count,
                    all_chat_pages.len()
                ));
            }
            if m.summary.dialogs_count != discovered_dialogs {
                report.errors.push(format!(
                    "Dialog count mismatch: manifest expected {} dialogs, but found {}",
                    m.summary.dialogs_count, discovered_dialogs
                ));
            }
        }

        for (src_file, target_url) in pending_links {
            report.total_links_checked += 1;
            if target_url.starts_with('#') {
                let anchor = target_url.trim_start_matches('#');
                if anchor != "blocked-unsafe-url" && !global_anchors.contains(anchor) {
                    report.errors.push(format!(
                        "Broken anchor in {}: #{}",
                        src_file.display(),
                        anchor
                    ));
                }
            } else if !target_url.starts_with("http://")
                && !target_url.starts_with("https://")
                && !target_url.starts_with("mailto:")
                && !target_url.starts_with("tel:")
                && !target_url.starts_with("tg:")
                && !target_url.starts_with("ton:")
                && !target_url.starts_with("ftp://")
                && !target_url.starts_with("ftps://")
            {
                if target_url.contains('\0') || target_url.contains(':') {
                    report.errors.push(format!(
                        "Link traversal escape detected in {}: invalid characters in '{}'",
                        src_file.display(),
                        target_url
                    ));
                    continue;
                }

                let (rel_path, anchor_opt) = if let Some((p, a)) = target_url.split_once('#') {
                    (p, Some(a))
                } else {
                    (target_url.as_str(), None)
                };

                let parent = src_file.parent().unwrap_or(&self.export_dir);
                let resolved_target = normalize_path(&parent.join(rel_path));

                if !resolved_target.starts_with(&export_canonical) {
                    report.errors.push(format!(
                        "Link traversal escape detected in {}: resolves outside export directory: {}",
                        src_file.display(),
                        resolved_target.display()
                    ));
                } else if !resolved_target.exists() {
                    report.errors.push(format!(
                        "Broken relative link in {}: target does not exist: {}",
                        src_file.display(),
                        resolved_target.display()
                    ));
                } else if let Some(anchor) = anchor_opt
                    && !global_anchors.contains(anchor)
                {
                    report.errors.push(format!(
                        "Broken link anchor in {}: target file exists but anchor #{} is missing",
                        src_file.display(),
                        anchor
                    ));
                }
            }
        }

        if !report.is_success() {
            return Err(RenderError::VerificationFailed(format!(
                "Verification encountered {} errors:\n{}",
                report.errors.len(),
                report.errors.join("\n")
            )));
        }

        Ok(report)
    }

    fn verify_search_subsystem(&self, report: &mut VerificationReport) -> RenderResult<()> {
        let manifest_path = self.export_dir.join("search/manifest.js");
        let manifest_content = fs::read_to_string(&manifest_path)?;

        let prefix = "window.__VENDETTA_SEARCH_MANIFEST__ = ";
        if !manifest_content.starts_with(prefix) {
            report.errors.push(
                "Invalid search/manifest.js: missing window.__VENDETTA_SEARCH_MANIFEST__ wrapper"
                    .to_string(),
            );
            return Ok(());
        }

        let json_str = manifest_content[prefix.len()..]
            .trim()
            .trim_end_matches(';');
        let search_manifest: SearchManifest = match serde_json::from_str(json_str) {
            Ok(m) => m,
            Err(e) => {
                report
                    .errors
                    .push(format!("Corrupt search/manifest.js JSON payload: {e}"));
                return Ok(());
            }
        };

        let shards_dir = self.export_dir.join("search/shards");
        if !shards_dir.exists() && !search_manifest.shards.is_empty() {
            report
                .errors
                .push("Missing search/shards directory".to_string());
            return Ok(());
        }

        let mut declared_shard_files = HashSet::new();
        let mut declared_shard_ids = HashSet::new();
        let mut sum_entries = 0;

        for meta in &search_manifest.shards {
            if !declared_shard_ids.insert(meta.shard_id) {
                report
                    .errors
                    .push(format!("Duplicate shard_id detected: {}", meta.shard_id));
            }
            declared_shard_files.insert(meta.file_name.clone());

            let shard_path = shards_dir.join(&meta.file_name);
            if !shard_path.exists() {
                report.errors.push(format!(
                    "Declared search shard file missing on disk: {}",
                    meta.file_name
                ));
                continue;
            }

            let shard_content = fs::read_to_string(&shard_path)?;
            let shard_prefix = "window.__VENDETTA_REGISTER_SEARCH_SHARD__(";
            if !shard_content.starts_with(shard_prefix) {
                report.errors.push(format!(
                    "Invalid search shard wrapper in {}",
                    meta.file_name
                ));
                continue;
            }

            let s_json = shard_content[shard_prefix.len()..]
                .trim()
                .trim_end_matches([';', ')']);
            let shard: SearchShard = match serde_json::from_str(s_json) {
                Ok(s) => s,
                Err(e) => {
                    report.errors.push(format!(
                        "Corrupt search shard payload in {}: {e}",
                        meta.file_name
                    ));
                    continue;
                }
            };

            if shard.shard_id != meta.shard_id {
                report.errors.push(format!(
                    "Shard ID mismatch in {}: declared {}, found {}",
                    meta.file_name, meta.shard_id, shard.shard_id
                ));
            }

            if shard.entries.len() != meta.entries_count {
                report.errors.push(format!(
                    "Shard entries count mismatch in {}: declared {}, found {}",
                    meta.file_name,
                    meta.entries_count,
                    shard.entries.len()
                ));
            }

            if shard.entries_count != shard.entries.len() {
                report.errors.push(format!(
                    "Internal shard entries count mismatch in {}: payload claims {}, found {}",
                    meta.file_name,
                    shard.entries_count,
                    shard.entries.len()
                ));
            }

            sum_entries += shard.entries.len();
            report.total_search_entries_checked += shard.entries.len();
        }

        if shards_dir.exists() {
            for entry in fs::read_dir(&shards_dir)?.flatten() {
                let fname = entry.file_name().to_string_lossy().to_string();
                if fname.ends_with(".js") && !declared_shard_files.contains(&fname) {
                    report.errors.push(format!(
                        "Undeclared search shard file present in shards directory: {fname}"
                    ));
                }
            }
        }

        if sum_entries != search_manifest.total_entries {
            report.errors.push(format!(
                "Total search entries mismatch: manifest declares {}, sum of shards is {}",
                search_manifest.total_entries, sum_entries
            ));
        }

        Ok(())
    }

    fn scan_html_file(
        &self,
        path: &Path,
        global_anchors: &mut HashSet<String>,
        global_message_anchors: &mut HashSet<String>,
        pending_links: &mut Vec<(PathBuf, String)>,
        report: &mut VerificationReport,
        media_mode: Option<&str>,
    ) -> RenderResult<()> {
        let content = fs::read_to_string(path)?;
        let mut page_anchors = HashSet::new();
        let export_canonical = normalize_path(&self.export_dir);

        for part in content.split(" id=\"").skip(1) {
            if let Some((id_val, _)) = part.split_once('"')
                && !id_val.is_empty()
            {
                report.total_anchors_checked += 1;

                if !page_anchors.insert(id_val.to_string()) {
                    report.errors.push(format!(
                        "Duplicate anchor detected in {}: #{}",
                        path.display(),
                        id_val
                    ));
                }

                if id_val.starts_with("m-p_") && !global_message_anchors.insert(id_val.to_string())
                {
                    report.errors.push(format!(
                        "Duplicate message anchor detected across pages in {}: #{}",
                        path.display(),
                        id_val
                    ));
                }

                global_anchors.insert(id_val.to_string());
            }
        }

        for part in content.split(" href=\"").skip(1) {
            if let Some((href_val, _)) = part.split_once('"')
                && !href_val.is_empty()
                && !href_val.starts_with('#')
            {
                pending_links.push((path.to_path_buf(), href_val.to_string()));
            }
        }

        for part in content.split(" src=\"").skip(1) {
            if let Some((src_val, _)) = part.split_once('"')
                && src_val.contains("media/")
            {
                report.total_media_checked += 1;

                if src_val.contains('\0') || src_val.contains(':') {
                    report.errors.push(format!(
                        "Media path traversal escape detected in {}: invalid characters in '{}'",
                        path.display(),
                        src_val
                    ));
                    continue;
                }

                let parent = path.parent().unwrap_or(&self.export_dir);
                let resolved = normalize_path(&parent.join(src_val));

                if !resolved.starts_with(&export_canonical) {
                    report.errors.push(format!(
                        "Media path escape detected in {}: resolves outside export directory: {}",
                        path.display(),
                        resolved.display()
                    ));
                } else if !resolved.exists() {
                    report.errors.push(format!(
                        "Referenced media file missing in {}: {}",
                        path.display(),
                        resolved.display()
                    ));
                } else if media_mode == Some("copy")
                    && let (Ok(can_res), Ok(can_exp)) =
                        (resolved.canonicalize(), self.export_dir.canonicalize())
                    && !can_res.starts_with(&can_exp)
                {
                    report.errors.push(format!(
                        "External symlink escape detected in copy mode in {}: target resolves to {}",
                        path.display(),
                        can_res.display()
                    ));
                }
            }
        }

        Ok(())
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::ParentDir => {
                out.pop();
            }
            Component::RootDir => out.push("/"),
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::CurDir => {}
        }
    }
    out
}
