use anyhow::{Context, Result};
use std::{path::Path, sync::Arc};
use vendetta_tg_adapter::{FileSession, GrammersTelegramAdapter, traits::TelegramAdapter};

pub async fn resolve_adapter(
    api_id_opt: Option<i32>,
    api_hash_opt: Option<String>,
    session_path: &Path,
) -> Result<Arc<dyn TelegramAdapter>> {
    let api_id = api_id_opt.ok_or_else(|| {
        anyhow::anyhow!(
            "Telegram API ID is required (set via --api-id, VENDETTA_API_ID, or config file)"
        )
    })?;
    let api_hash = api_hash_opt.ok_or_else(|| {
        anyhow::anyhow!(
            "Telegram API Hash is required (set via --api-hash, VENDETTA_API_HASH, or config file)"
        )
    })?;

    let session = FileSession::open(session_path)
        .with_context(|| format!("Failed to open session file at {}", session_path.display()))?;

    let adapter = GrammersTelegramAdapter::connect(api_id, &api_hash, session)
        .await
        .context("Failed to connect MTProto client to Telegram network")?;

    Ok(Arc::new(adapter))
}
