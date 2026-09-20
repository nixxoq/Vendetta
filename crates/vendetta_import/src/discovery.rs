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
    pub basedir: PathBuf,
    pub format: TDesktopFormat,
    pub files: Vec<PathBuf>,
    pub title: Option<String>,
    pub internal_discriminator: Option<String>,
}

pub struct DiscoverySession {
    _tempdir: Option<TempDir>,
    pub chat_sources: Vec<TDesktopChatSource>,
}

pub fn discover_export(source_path: &Path) -> ImportResult<DiscoverySession> {
    if !source_path.exists() {
        return Err(ImportError::SourceNotFound(source_path.to_path_buf()));
    }

    let (root_dir, tempdir) = if source_path.is_file() {
        let is_zip = source_path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"));

        if is_zip {
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

    let chat_sources = discover_chats(&root_dir)?;
    if chat_sources.is_empty() {
        return Err(ImportError::UnrecognizedExportFormat(root_dir));
    }

    Ok(DiscoverySession {
        _tempdir: tempdir,
        chat_sources,
    })
}

fn discover_chats(root: &Path) -> ImportResult<Vec<TDesktopChatSource>> {
    let root_json = root.join("result.json");

    if root_json.is_file()
        && let Some(source) = discover_json_source(root, &root_json)?
    {
        return Ok(vec![source]);
    }

    let root_html_files = collect_html(root);
    if !root_html_files.is_empty() {
        info!(
            "Discovered Single-Chat TDesktop HTML export with {} pages",
            root_html_files.len()
        );

        return Ok(vec![TDesktopChatSource {
            basedir: root.to_path_buf(),
            format: TDesktopFormat::Html,
            files: root_html_files,
            title: None,
            internal_discriminator: None,
        }]);
    }

    let chats_dir = root.join("chats");
    let scan_dir = if chats_dir.is_dir() {
        chats_dir
    } else {
        root.to_path_buf()
    };

    let mut sources = fs::read_dir(scan_dir)?
        .filter_map(Result::ok)
        .filter_map(|entry| discover_chat_source(entry.path()))
        .collect::<Vec<_>>();

    if sources.is_empty() {
        return Err(ImportError::UnrecognizedExportFormat(root.to_path_buf()));
    }

    sources.sort_by(|a, b| a.basedir.cmp(&b.basedir));

    info!(
        "Discovered Multi-Chat TDesktop export with {} chats",
        sources.len()
    );

    Ok(sources)
}

fn discover_json_source(root: &Path, root_json: &Path) -> ImportResult<Option<TDesktopChatSource>> {
    let content = fs::read_to_string(root_json)?;
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Ok(None);
    };

    if let Some(chats) = value
        .get("chats")
        .and_then(|chats| chats.get("list"))
        .and_then(serde_json::Value::as_array)
    {
        info!(
            "Discovered Full-Account TDesktop JSON export with {} chats",
            chats.len()
        );

        return Ok(Some(TDesktopChatSource {
            basedir: root.to_path_buf(),
            format: TDesktopFormat::Json,
            files: vec![root_json.to_path_buf()],
            title: None,
            internal_discriminator: None,
        }));
    }

    let Some(messages) = value.get("messages").and_then(serde_json::Value::as_array) else {
        return Ok(None);
    };

    let title = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);

    info!(
        "Discovered Single-Chat TDesktop JSON export with {} messages",
        messages.len()
    );

    Ok(Some(TDesktopChatSource {
        basedir: root.to_path_buf(),
        format: TDesktopFormat::Json,
        files: vec![root_json.to_path_buf()],
        title,
        internal_discriminator: None,
    }))
}

fn discover_chat_source(path: PathBuf) -> Option<TDesktopChatSource> {
    if !path.is_dir() {
        return None;
    }

    let internal_discriminator = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned);

    let html_files = collect_html(&path);
    if !html_files.is_empty() {
        return Some(TDesktopChatSource {
            basedir: path,
            format: TDesktopFormat::Html,
            files: html_files,
            title: None,
            internal_discriminator,
        });
    }

    let result_json = path.join("result.json");
    result_json.is_file().then(|| TDesktopChatSource {
        basedir: path,
        format: TDesktopFormat::Json,
        files: vec![result_json],
        title: None,
        internal_discriminator,
    })
}

pub fn collect_html(dir: &Path) -> Vec<PathBuf> {
    let primary = dir.join("messages.html");
    let primary_iter = primary.is_file().then_some(primary).into_iter();

    let chunks_iter = (2..)
        .map(|idx| dir.join(format!("messages{idx}.html")))
        .take_while(|path| path.is_file());

    primary_iter.chain(chunks_iter).collect()
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
