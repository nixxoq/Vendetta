use anyhow::{Context, Result};
use std::path::Path;
use tracing::{debug, info};
use vendetta_render::{
    ExportOptions, ExportSummary, HtmlArchiveExporter, HtmlArchiveVerifier, VerificationReport,
};
use vendetta_storage::ArchiveDb;

pub fn run_export_html(
    archive_db_path: &Path,
    options: ExportOptions,
    disable_forum_render: bool,
) -> Result<ExportSummary> {
    info!(
        "Opening SQLite archive database at {}",
        archive_db_path.display()
    );
    let db = ArchiveDb::open(archive_db_path).context("Failed to open archive database")?;

    info!(
        "Starting static HTML export (Mode: {:?}, Media: {:?}, Destination: {})",
        options.presentation_mode,
        options.media_mode,
        options.output_dir.display()
    );

    let exporter = HtmlArchiveExporter::new(&db, options)
        .with_disable_forum_render(disable_forum_render);
    let summary = exporter
        .export_with_progress(|stage, current, total| {
            if total > 0 && current > 0 {
                debug!("{}: {} / {}", stage, current, total);
            } else {
                debug!("{}", stage);
            }
        })
        .context("HTML export failed")?;

    info!(
        "HTML export completed successfully: {} dialogs, {} messages, {} chunks, {} media files copied",
        summary.dialogs_count,
        summary.messages_count,
        summary.chunks_count,
        summary.media_copied_count
    );

    Ok(summary)
}

pub fn run_verify_html(export_dir: &Path) -> Result<VerificationReport> {
    info!(
        "Starting offline HTML integrity verification on {}",
        export_dir.display()
    );

    let verifier = HtmlArchiveVerifier::new(export_dir);
    let report = verifier
        .verify()
        .context("HTML archive verification failed")?;

    info!(
        "Verification passed: {} pages, {} anchors, {} links, {} media verified (0 errors)",
        report.total_pages_checked,
        report.total_anchors_checked,
        report.total_links_checked,
        report.total_media_checked
    );

    Ok(report)
}
