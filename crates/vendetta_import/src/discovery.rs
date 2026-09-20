use std::{
    fs::{self, File},
    io,
    path::{Component, Path, PathBuf},
};

use tempfile::TempDir;
use tracing::{debug, info};

use crate::error::{ImportError, ImportResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TDesktopFormat {
    Html,
    Json,
}

#[derive(Debug, Clone)]
pub struct TDesktopChatSource {
    pub base_dir: PathBuf,
    pub format: TDesktopFormat,
    pub entry_files: Vec<PathBuf>,
    pub title_hint: Option<String>,
    pub internal_discriminator: Option<String>,
}

pub struct DiscoverySession {
    _extracted_tempdir: Option<TempDir>,
    pub chat_sources: Vec<TDesktopChatSource>,
}

pub fn discover_export(source_path: &Path) -> ImportResult<DiscoverySession> {
    if !source_path.exists() {
        return Err(ImportError::SourceNotFound(source_path.to_path_buf()));
    }

    let (root_dir, tempdir) = if source_path.is_file() {
        let ext = source_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext == "zip" {
            let temp = extract_zip_safely(source_path)?;
            let path = temp.path().to_path_buf();
            (path, Some(temp))
        } else {
            return Err(ImportError::UnrecognizedExportFormat(
                source_path.to_path_buf(),
            ));
        }
    } else {
        (source_path.to_path_buf(), None)
    };

    let chat_sources = discover_in_directory(&root_dir)?;
    if chat_sources.is_empty() {
        return Err(ImportError::UnrecognizedExportFormat(root_dir));
    }

    Ok(DiscoverySession {
        _extracted_tempdir: tempdir,
        chat_sources,
    })
}

fn discover_in_directory(root: &Path) -> ImportResult<Vec<TDesktopChatSource>> {
    // 1. Check for root result.json
    let root_json = root.join("result.json");
    if root_json.is_file() {
        let content = fs::read_to_string(&root_json)?;
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            // Full-account JSON: contains chats.list
            if let Some(chats_list) = val
                .get("chats")
                .and_then(|c| c.get("list"))
                .and_then(|l| l.as_array())
            {
                info!(
                    "Discovered Full-Account TDesktop JSON export with {} chats",
                    chats_list.len()
                );
                return Ok(vec![TDesktopChatSource {
                    base_dir: root.to_path_buf(),
                    format: TDesktopFormat::Json,
                    entry_files: vec![root_json],
                    title_hint: None,
                    internal_discriminator: None,
                }]);
            }
            // Single-chat JSON: root contains messages
            if val.get("messages").and_then(|m| m.as_array()).is_some() {
                let name = val
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(ToString::to_string);
                info!("Discovered Single-Chat TDesktop JSON export: {:?}", name);
                return Ok(vec![TDesktopChatSource {
                    base_dir: root.to_path_buf(),
                    format: TDesktopFormat::Json,
                    entry_files: vec![root_json],
                    title_hint: name,
                    internal_discriminator: None,
                }]);
            }
        }
    }

    // 2. Check for root messages.html (Single-Chat HTML)
    let root_html_files = collect_chunk_html_files(root);
    if !root_html_files.is_empty() {
        info!(
            "Discovered Single-Chat TDesktop HTML export with {} pages",
            root_html_files.len()
        );
        return Ok(vec![TDesktopChatSource {
            base_dir: root.to_path_buf(),
            format: TDesktopFormat::Html,
            entry_files: root_html_files,
            title_hint: None,
            internal_discriminator: None,
        }]);
    }

    // 3. Check for subdirectories (e.g. chats/ or a single subfolder containing the export)
    let mut sources = Vec::new();
    let chats_dir = root.join("chats");
    let scan_dir = if chats_dir.is_dir() {
        chats_dir
    } else {
        root.to_path_buf()
    };

    if let Ok(entries) = fs::read_dir(&scan_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let html_files = collect_chunk_html_files(&path);
                let is_html = !html_files.is_empty();
                let is_json = !is_html && path.join("result.json").is_file();

                if is_html || is_json {
                    let folder_name = path
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or("chat")
                        .to_string();

                    let (format, entry_files) = if is_html {
                        (TDesktopFormat::Html, html_files)
                    } else {
                        (TDesktopFormat::Json, vec![path.join("result.json")])
                    };

                    sources.push(TDesktopChatSource {
                        base_dir: path,
                        format,
                        entry_files,
                        title_hint: None,
                        internal_discriminator: Some(folder_name),
                    });
                }
            }
        }
    }

    if !sources.is_empty() {
        info!(
            "Discovered Multi-Chat TDesktop export with {} chats",
            sources.len()
        );
        // Sort stably by discriminator/dir
        sources.sort_by(|a, b| a.base_dir.cmp(&b.base_dir));
        return Ok(sources);
    }

    Err(ImportError::UnrecognizedExportFormat(root.to_path_buf()))
}

pub fn collect_chunk_html_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    let primary = dir.join("messages.html");
    if primary.is_file() {
        files.push(primary);
    }

    // TDesktop names sequential chunk files messages2.html, messages3.html, etc.
    let mut idx = 2;
    loop {
        let chunk = dir.join(format!("messages{idx}.html"));
        if chunk.is_file() {
            files.push(chunk);
            idx += 1;
        } else {
            break;
        }
    }

    files
}

pub fn extract_zip_safely(zip_path: &Path) -> ImportResult<TempDir> {
    let file = File::open(zip_path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| ImportError::ZipExtractionFailed(e.to_string()))?;

    let temp_dir = tempfile::Builder::new()
        .prefix("vendetta_tdesktop_zip_")
        .tempdir()?;

    for i in 0..archive.len() {
        let mut zip_file = archive
            .by_index(i)
            .map_err(|e| ImportError::ZipExtractionFailed(e.to_string()))?;

        // Guard against Zip Slip / path traversal
        let Some(enclosed_path) = zip_file.enclosed_name() else {
            return Err(ImportError::ZipExtractionFailed(format!(
                "Path traversal attempt in ZIP entry: {}",
                zip_file.name()
            )));
        };

        for component in enclosed_path.components() {
            if matches!(component, Component::ParentDir | Component::RootDir) {
                return Err(ImportError::ZipExtractionFailed(format!(
                    "Illegal path component in ZIP entry: {}",
                    zip_file.name()
                )));
            }
        }

        let out_path = temp_dir.path().join(enclosed_path);

        if zip_file.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out_file = File::create(&out_path)?;
            io::copy(&mut zip_file, &mut out_file)?;
        }
    }

    debug!(
        "Extracted ZIP export safely to temporary directory: {}",
        temp_dir.path().display()
    );
    Ok(temp_dir)
}
