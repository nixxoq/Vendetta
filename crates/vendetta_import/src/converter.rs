use std::path::{Path, PathBuf};

use tracing::info;
use vendetta_render::{
    ExportOptions, HtmlArchiveExporter, MediaMode, PresentationMode, ThemeMode,
    verifier::HtmlArchiveVerifier,
};
use vendetta_storage::ArchiveDb;

use crate::{
    discovery::{TDesktopFormat, discover_export},
    error::{ImportError, ImportResult},
    ingest::ingest_chats_into_db,
    model::ImportChat,
    parser_html::parse_tdesktop_html,
    parser_json::parse_tdesktop_json,
};

#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub source_path: PathBuf,
    pub archive_path: PathBuf,
    pub media_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub source_path: PathBuf,
    pub output_dir: PathBuf,
    pub presentation_mode: PresentationMode,
    pub theme: ThemeMode,
    pub chunk_size: usize,
    pub replace: bool,
    pub split_by: vendetta_render::SplitBy,
    pub date_structure: vendetta_render::DateStructure,
    pub readable_names: bool,
    pub date_bounds: (Option<i64>, Option<i64>),
    pub build_search_index: bool,
    pub build_date_index: bool,
    pub disable_forum_render: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            source_path: PathBuf::new(),
            output_dir: PathBuf::new(),
            presentation_mode: PresentationMode::TelegramLike,
            theme: ThemeMode::System,
            chunk_size: 250,
            replace: false,
            split_by: vendetta_render::SplitBy::Messages,
            date_structure: vendetta_render::DateStructure::Flat,
            readable_names: false,
            date_bounds: (None, None),
            build_search_index: true,
            build_date_index: true,
            disable_forum_render: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ImportSummary {
    pub archive_path: PathBuf,
    pub media_dir: PathBuf,
    pub chats_count: usize,
    pub messages_count: usize,
    pub media_copied_count: usize,
    pub media_skipped_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct ConvertSummary {
    pub destination: PathBuf,
    pub dialogs_count: usize,
    pub messages_count: usize,
    pub chunks_count: usize,
    pub media_copied_count: usize,
    pub manifest_path: PathBuf,
}

pub fn import_tdesktop(options: &ImportOptions) -> ImportResult<ImportSummary> {
    if options.archive_path.exists() {
        return Err(ImportError::DestinationArchiveExists(
            options.archive_path.clone(),
        ));
    }

    if let Some(parent) = options.archive_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let all_chats = parse_all_chats(&options.source_path)?;

    let db = ArchiveDb::open(&options.archive_path)?;
    let resolved_media_dir = options.media_dir.clone().unwrap_or_else(|| {
        options
            .archive_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("media")
    });

    let ingest_summary = ingest_chats_into_db(&db, &resolved_media_dir, &all_chats)?;

    Ok(ImportSummary {
        archive_path: options.archive_path.clone(),
        media_dir: resolved_media_dir,
        chats_count: ingest_summary.chats_count,
        messages_count: ingest_summary.messages_count,
        media_copied_count: ingest_summary.media_copied_count,
        media_skipped_count: ingest_summary.media_skipped_count,
    })
}

pub fn convert_tdesktop(options: &ConvertOptions) -> ImportResult<ConvertSummary> {
    if options.output_dir.exists() && !options.replace {
        return Err(ImportError::DestinationOutputExists(
            options.output_dir.clone(),
        ));
    }

    let all_chats = parse_all_chats(&options.source_path)?;

    // Allocate isolated ephemeral environment
    let ephemeral_dir = tempfile::Builder::new()
        .prefix("vendetta_convert_")
        .tempdir()?;
    let ephemeral_db_path = ephemeral_dir.path().join("ephemeral_archive.db");
    let ephemeral_media_dir = ephemeral_dir.path().join("media");

    let db = ArchiveDb::open(&ephemeral_db_path)?;
    ingest_chats_into_db(&db, &ephemeral_media_dir, &all_chats)?;

    info!(
        "Ingested into ephemeral DB, running HTML exporter targeting: {}",
        options.output_dir.display()
    );

    let export_options = ExportOptions {
        output_dir: options.output_dir.clone(),
        presentation_mode: options.presentation_mode,
        media_mode: MediaMode::Copy,
        theme: options.theme,
        chunk_size: options.chunk_size,
        replace: options.replace,
        media_src_dir: Some(ephemeral_media_dir),
        include_service_messages: true,
        include_deleted_messages: true,
        include_edit_history: true,
        build_search_index: options.build_search_index,
        build_date_index: options.build_date_index,
        target_peers: None,
    };

    let exporter = HtmlArchiveExporter::new(&db, export_options)
        .with_disable_forum_render(options.disable_forum_render)
        .with_readable_names(options.readable_names)
        .with_date_range(options.date_bounds.0, options.date_bounds.1)
        .with_split_by(options.split_by)
        .with_date_structure(options.date_structure);

    let render_summary = exporter.export()?;

    // Verify output directory
    let verifier = HtmlArchiveVerifier::new(&options.output_dir);
    let verify_report = verifier.verify()?;
    if !verify_report.is_success() {
        return Err(ImportError::Render(
            vendetta_render::RenderError::VerificationFailed(format!(
                "Converted HTML export verification failed with {} errors:\n{}",
                verify_report.errors.len(),
                verify_report.errors.join("\n")
            )),
        ));
    }

    info!(
        "Direct HTML conversion verified and completed successfully at {}",
        options.output_dir.display()
    );

    Ok(ConvertSummary {
        destination: options.output_dir.clone(),
        dialogs_count: render_summary.dialogs_count,
        messages_count: render_summary.messages_count,
        chunks_count: render_summary.chunks_count,
        media_copied_count: render_summary.media_copied_count,
        manifest_path: options.output_dir.join("manifest.json"),
    })
}

fn parse_all_chats(source_path: &Path) -> ImportResult<Vec<ImportChat>> {
    let discovery = discover_export(source_path)?;
    let mut all_chats = Vec::new();

    for source in &discovery.chat_sources {
        match source.format {
            TDesktopFormat::Html => {
                all_chats.push(parse_tdesktop_html(source)?);
            }
            TDesktopFormat::Json => {
                all_chats.extend(parse_tdesktop_json(source)?);
            }
        }
    }

    let resolver = crate::self_identity::SelfIdentityResolver::new(source_path);
    resolver.resolve_and_apply(&mut all_chats);

    Ok(all_chats)
}
