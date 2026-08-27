pub mod css;
pub mod icons;
pub mod js;

pub use icons::SYMBOLS_SVG;

use std::{fs, path::Path};

use crate::error::RenderResult;

pub fn write_all_assets(export_dir: &Path) -> RenderResult<()> {
    let assets_dir = export_dir.join("assets");
    let css_dir = assets_dir.join("css");
    let js_dir = assets_dir.join("js");
    let icons_dir = assets_dir.join("icons");

    fs::create_dir_all(&css_dir)?;
    fs::create_dir_all(&js_dir)?;
    fs::create_dir_all(&icons_dir)?;

    fs::write(css_dir.join("theme.css"), css::THEME_CSS)?;
    fs::write(css_dir.join("main.css"), css::MAIN_CSS)?;
    fs::write(css_dir.join("telegram_like.css"), css::TELEGRAM_LIKE_CSS)?;
    fs::write(css_dir.join("archive_dense.css"), css::ARCHIVE_DENSE_CSS)?;

    fs::write(js_dir.join("app.js"), js::APP_JS)?;
    fs::write(js_dir.join("lightbox.js"), js::LIGHTBOX_JS)?;
    fs::write(js_dir.join("search.js"), js::SEARCH_JS)?;

    fs::write(icons_dir.join("symbols.svg"), icons::SYMBOLS_SVG)?;

    Ok(())
}
