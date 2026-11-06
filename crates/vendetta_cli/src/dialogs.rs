use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use vendetta_model::PeerType;

use crate::client::resolve_adapter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogItemSummary {
    pub peer_id: i64,
    pub peer_type: String,
    pub name: Option<String>,
    pub username: Option<String>,
    pub phone: Option<String>,
    pub updated_at: i64,
}

fn peer_type_to_str(peer_type: PeerType) -> &'static str {
    match peer_type {
        PeerType::User => "user",
        PeerType::Group => "group",
        PeerType::Channel => "channel",
    }
}

fn print_table(dialogs: &[DialogItemSummary]) {
    println!("==================================================");
    println!("TELEGRAM DIALOGS ({})", dialogs.len());
    println!(
        "{:<14} {:<10} {:<30} USERNAME",
        "PEER ID", "TYPE", "TITLE / NAME"
    );
    println!("{:-<14} {:-<10} {:-<30} {:-<15}", "", "", "", "");

    for d in dialogs {
        let name = d.name.as_deref().unwrap_or("-");
        let username = match &d.username {
            Some(u) => format!("@{u}"),
            None => "-".to_string(),
        };
        println!(
            "{:<14} {:<10} {:<30} {}",
            d.peer_id, d.peer_type, name, username
        );
    }
}

fn print_json(dialogs: &[DialogItemSummary]) -> Result<()> {
    let out = serde_json::json!({
        "schema_version": 1,
        "command": "dialogs",
        "status": "completed",
        "count": dialogs.len(),
        "dialogs": dialogs,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

pub async fn run_dialogs(
    api_id: Option<i32>,
    api_hash: Option<String>,
    session_path: &Path,
    limit: Option<usize>,
    peer_type_filter: Option<PeerType>,
    json: bool,
) -> Result<Vec<DialogItemSummary>> {
    let adapter = resolve_adapter(api_id, api_hash, session_path).await?;
    let raw_dialogs = adapter
        .get_dialogs()
        .await
        .context("Failed to retrieve dialogs from Telegram adapter")?;

    let mut dialogs: Vec<DialogItemSummary> = raw_dialogs
        .into_iter()
        .filter(|p| peer_type_filter.is_none_or(|ft| p.peer_type == ft))
        .map(|p| DialogItemSummary {
            peer_id: p.peer_id.raw(),
            peer_type: peer_type_to_str(p.peer_type).to_string(),
            name: p.name,
            username: p.username,
            phone: p.phone,
            updated_at: p.updated_at,
        })
        .collect();

    if let Some(limit) = limit {
        dialogs.truncate(limit);
    }

    if json {
        print_json(&dialogs)?;
    } else {
        print_table(&dialogs);
    }

    Ok(dialogs)
}
