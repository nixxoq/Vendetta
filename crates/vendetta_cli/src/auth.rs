use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::info;
use vendetta_model::PeerRecord;
use vendetta_tg_adapter::GrammersTelegramAdapter;
use vendetta_tg_adapter::auth::AuthPrompt;
use vendetta_tg_adapter::session::FileSession;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatusOutput {
    pub schema_version: u32,
    pub command: String,
    pub status: String,
    pub session_path: String,
    pub user: Option<AuthUserSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthUserSummary {
    pub peer_id: i64,
    pub name: Option<String>,
    pub username: Option<String>,
    pub phone: Option<String>,
}

// TODO: i need to find a better option than this crap I quickly wrote
pub fn prompt_line(prompt: &str) -> io::Result<String> {
    eprint!("{prompt}");
    io::stderr().flush()?;
    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

// TODO: same todo as in :29
pub fn prompt_secret(prompt: &str) -> io::Result<String> {
    eprint!("{prompt}");
    io::stderr().flush()?;
    let secret = rpassword::read_password()?;
    Ok(secret.trim().to_string())
}

pub async fn connect_adapter(
    api_id: i32,
    api_hash: &str,
    session_path: &Path,
) -> Result<Arc<GrammersTelegramAdapter>> {
    let session = FileSession::open(session_path)
        .with_context(|| format!("Failed to open session file at {}", session_path.display()))?;

    let adapter = GrammersTelegramAdapter::connect(api_id, api_hash, session)
        .await
        .context("Failed to connect MTProto client to Telegram network")?;

    Ok(Arc::new(adapter))
}

pub async fn run_auth(
    api_id_opt: Option<i32>,
    api_hash_opt: Option<String>,
    session_path: PathBuf,
    phone_arg: Option<String>,
    force_login: bool,
    json: bool,
    explicit_config_path: Option<&Path>,
) -> Result<()> {
    let api_id = match api_id_opt {
        Some(id) if id > 0 => id,
        _ => {
            if json {
                bail!(
                    "Telegram API ID is required (set via --api-id, VENDETTA_API_ID, or config file)"
                );
            }
            let input = prompt_line("Enter Telegram API ID: ")?;
            input.parse::<i32>().context("Invalid numeric API ID")?
        }
    };

    let api_hash = match api_hash_opt {
        Some(h) if !h.trim().is_empty() => h,
        _ => {
            if json {
                bail!(
                    "Telegram API Hash is required (set via --api-hash, VENDETTA_API_HASH, or config file)"
                );
            }
            prompt_secret("Enter Telegram API Hash: ")?
        }
    };

    let adapter = connect_adapter(api_id, &api_hash, &session_path).await?;
    let auth = adapter.auth();

    let is_auth = auth
        .is_authorized()
        .await
        .context("Failed to check authorization status")?;

    if is_auth && !force_login {
        if json {
            let out = AuthStatusOutput {
                schema_version: 1,
                command: "auth".to_string(),
                status: "authorized".to_string(),
                session_path: session_path.display().to_string(),
                user: None,
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            // TODO: perhaps it's better to use logging stuff to output smth like that?
            println!("==================================================");
            println!("TELEGRAM ACCOUNT AUTHORIZED");
            println!("==================================================");
            println!("Session Path:      {}", session_path.display());
            println!("Status:            Authorized");
        }
        return Ok(());
    }

    let phone = match phone_arg {
        Some(p) if !p.trim().is_empty() => p,
        _ => prompt_line("Enter phone number (international format e.g. +1234567890): ")?,
    };

    eprintln!("Requesting login verification code for {phone}...");
    let prompt = auth
        .start_auth(&phone)
        .await
        .context("Failed to request login code from Telegram")?;

    let final_user: PeerRecord = match prompt {
        AuthPrompt::AlreadyAuthorized(u) => u,
        AuthPrompt::CodeRequired { .. } => {
            let code = prompt_line("Enter verification code sent by Telegram: ")?;
            match auth
                .submit_code(&code)
                .await
                .context("Failed to submit verification code")?
            {
                AuthPrompt::AlreadyAuthorized(u) => u,
                AuthPrompt::PasswordRequired { hint } => {
                    if let Some(h) = hint {
                        eprintln!("2FA Password Hint: {h}");
                    }
                    let password = prompt_secret("Enter 2FA account password: ")?;
                    auth.submit_password(&password)
                        .await
                        .context("Failed to submit 2FA password")?
                }
                _ => bail!("Unexpected authorization state after submitting code"),
            }
        }
        AuthPrompt::PasswordRequired { hint } => {
            if let Some(h) = hint {
                eprintln!("2FA Password Hint: {h}");
            }
            let password = prompt_secret("Enter 2FA account password: ")?;
            auth.submit_password(&password)
                .await
                .context("Failed to submit 2FA password")?
        }
    };

    adapter
        .session()
        .save()
        .context("Failed to persist authorized session file")?;

    let user_summary = AuthUserSummary {
        peer_id: final_user.peer_id.raw(),
        name: final_user.name,
        username: final_user.username,
        phone: final_user.phone,
    };

    if json {
        let out = AuthStatusOutput {
            schema_version: 1,
            command: "auth".to_string(),
            status: "authorized".to_string(),
            session_path: session_path.display().to_string(),
            user: Some(user_summary),
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        // TODO: perhaps it's better to use logging stuff to output smth like that?
        println!("==================================================");
        println!("AUTHENTICATION SUCCESSFUL");
        println!("==================================================");
        println!("Session saved:     {}", session_path.display());
        println!("User ID:           {}", user_summary.peer_id);
        println!(
            "Name:              {}",
            user_summary.name.unwrap_or_else(|| "-".to_string())
        );
        println!(
            "Username:          {}",
            user_summary
                .username
                .map(|s| format!("@{s}"))
                .unwrap_or_else(|| "-".to_string())
        );
        println!(
            "Phone:             {}",
            user_summary.phone.unwrap_or_else(|| "-".to_string())
        );

        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            let target_config_path =
                crate::config::CliConfig::resolve_write_config_path(explicit_config_path);
            let existing = crate::config::CliConfig::load_file_or_default(&target_config_path);
            let already_saved = existing.api_id == Some(api_id)
                && existing.api_hash.as_deref() == Some(api_hash.as_str());

            if !already_saved {
                let ans = prompt_line("\nSave API credentials to Vendetta config? [Y/n] ")?;
                let ans_lower = ans.trim().to_lowercase();
                if ans_lower.is_empty() || ans_lower == "y" || ans_lower == "yes" {
                    let mut updated = existing;
                    updated.api_id = Some(api_id);
                    updated.api_hash = Some(api_hash);
                    updated.save_to_file(&target_config_path)?;
                    println!("API credentials saved to {}", target_config_path.display());
                }
            }
        }
    }

    Ok(())
}

/// Checks and reports authorization status
/// Samilar to run_auth func
pub async fn run_auth_status(
    api_id_opt: Option<i32>,
    api_hash_opt: Option<String>,
    session_path: PathBuf,
    json: bool,
) -> Result<bool> {
    if !session_path.exists() {
        if json {
            let out = AuthStatusOutput {
                schema_version: 1,
                command: "auth".to_string(),
                status: "unauthorized".to_string(),
                session_path: session_path.display().to_string(),
                user: None,
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            println!(
                "Status: Unauthorized (session file does not exist at {})",
                session_path.display()
            );
        }
        return Ok(false);
    }

    let api_id = api_id_opt.unwrap_or(0);
    let api_hash = api_hash_opt.unwrap_or_default();

    let session = FileSession::open(&session_path)?;
    let adapter = if api_id > 0 && !api_hash.is_empty() {
        connect_adapter(api_id, &api_hash, &session_path).await?
    } else {
        Arc::new(GrammersTelegramAdapter::new_with_session(session))
    };

    let is_auth = adapter.auth().is_authorized().await.unwrap_or(false);

    // TODO: perhaps it's better to use logging stuff to output smth like that?
    if is_auth {
        if json {
            let out = AuthStatusOutput {
                schema_version: 1,
                command: "auth".to_string(),
                status: "authorized".to_string(),
                session_path: session_path.display().to_string(),
                user: None,
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            println!("Status:  Authorized");
            println!("Session: {}", session_path.display());
        }
        Ok(true)
    } else {
        if json {
            let out = AuthStatusOutput {
                schema_version: 1,
                command: "auth".to_string(),
                status: "unauthorized".to_string(),
                session_path: session_path.display().to_string(),
                user: None,
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            println!("Status:  Unauthorized");
            println!("Session: {}", session_path.display());
        }
        Ok(false)
    }
}

pub async fn run_auth_logout(
    api_id_opt: Option<i32>,
    api_hash_opt: Option<String>,
    session_path: &Path,
    local_only: bool,
    json: bool,
) -> Result<()> {
    if !session_path.exists() {
        if json {
            let out = AuthStatusOutput {
                schema_version: 1,
                command: "auth".to_string(),
                status: "logged_out".to_string(),
                session_path: session_path.display().to_string(),
                user: None,
            };
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            println!("No local session file exists at {}", session_path.display());
        }
        return Ok(());
    }

    if !local_only {
        let api_id = api_id_opt
            .ok_or_else(|| anyhow::anyhow!("Telegram API ID is required for remote sign-out"))?;
        let api_hash = api_hash_opt
            .ok_or_else(|| anyhow::anyhow!("Telegram API Hash is required for remote sign-out"))?;

        let adapter = connect_adapter(api_id, &api_hash, session_path)
            .await
            .context("Failed to connect to Telegram for remote sign-out")?;

        adapter
            .auth()
            .sign_out()
            .await
            .context("Telegram server rejected sign_out request")?;
    }

    std::fs::remove_file(session_path).with_context(|| {
        format!(
            "Failed to delete session file at {}",
            session_path.display()
        )
    })?;
    info!("Deleted session file at {}", session_path.display());

    if json {
        let out = AuthStatusOutput {
            schema_version: 1,
            command: "auth".to_string(),
            status: "logged_out".to_string(),
            session_path: session_path.display().to_string(),
            user: None,
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if local_only {
        println!(
            "Local session credentials removed (local-only): {}",
            session_path.display()
        );
    } else {
        println!(
            "Remotely signed out and deleted session file: {}",
            session_path.display()
        );
    }

    Ok(())
}
