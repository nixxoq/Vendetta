use std::sync::Arc;

use grammers_client::{
    Client, SignInError,
    client::{LoginToken, PasswordToken},
};
use tokio::sync::Mutex;
use vendetta_model::{PeerId, PeerRecord, PeerType};

use crate::{
    error::{AdapterError, AdapterResult},
    normalize::normalize_user,
    session::FileSession,
};

#[derive(Debug)]
pub enum AuthPrompt {
    CodeRequired { phone: String },
    PasswordRequired { hint: Option<String> },
    AlreadyAuthorized(PeerRecord),
}

pub struct TelegramAuthService {
    client: Arc<Client>,
    session: Arc<FileSession>,
    _api_id: i32,
    api_hash: String,
    login_token: Mutex<Option<LoginToken>>,
    password_token: Mutex<Option<PasswordToken>>,
}

impl TelegramAuthService {
    pub fn new(
        client: Arc<Client>,
        session: Arc<FileSession>,
        api_id: i32,
        api_hash: impl Into<String>,
    ) -> Self {
        Self {
            client,
            session,
            _api_id: api_id,
            api_hash: api_hash.into(),
            login_token: Mutex::new(None),
            password_token: Mutex::new(None),
        }
    }

    pub async fn is_authorized(&self) -> AdapterResult<bool> {
        let authorized = self.client.is_authorized().await?;
        Ok(authorized)
    }

    pub async fn start_auth(&self, phone: &str) -> AdapterResult<AuthPrompt> {
        if self.is_authorized().await? {
            return Ok(AuthPrompt::AlreadyAuthorized(PeerRecord {
                peer_id: PeerId::new(0),
                peer_type: PeerType::User,
                name: Some("Authorized User".to_string()),
                username: None,
                phone: Some(phone.to_string()),
                raw_tl: None,
                updated_at: 0,
            }));
        }

        let token = self
            .client
            .request_login_code(phone, &self.api_hash)
            .await?;

        let prompt = AuthPrompt::CodeRequired {
            phone: phone.to_string(),
        };

        *self.login_token.lock().await = Some(token);
        *self.password_token.lock().await = None;

        Ok(prompt)
    }

    pub async fn submit_code(&self, code: &str) -> AdapterResult<AuthPrompt> {
        let token = {
            let mut guard = self.login_token.lock().await;
            guard.take().ok_or_else(|| {
                AdapterError::InvalidAuthState("No active login code request".to_string())
            })?
        };

        match self.client.sign_in(&token, code.trim()).await {
            Ok(user) => {
                *self.login_token.lock().await = None;
                *self.password_token.lock().await = None;
                let _ = self.session.save();
                let record = normalize_user(&user);
                Ok(AuthPrompt::AlreadyAuthorized(record))
            }
            Err(SignInError::PasswordRequired(pw_token)) => {
                let hint = pw_token.hint().map(|s| s.to_string());
                *self.password_token.lock().await = Some(pw_token);
                Ok(AuthPrompt::PasswordRequired { hint })
            }
            Err(other) => {
                *self.login_token.lock().await = Some(token);
                Err(other.into())
            }
        }
    }

    pub async fn submit_password(&self, password: &str) -> AdapterResult<PeerRecord> {
        let pw_token = {
            let mut guard = self.password_token.lock().await;
            guard.take().ok_or_else(|| {
                AdapterError::InvalidAuthState("No active 2FA challenge".to_string())
            })?
        };

        match self
            .client
            .check_password(pw_token, password.as_bytes())
            .await
        {
            Ok(user) => {
                *self.login_token.lock().await = None;
                let _ = self.session.save();
                Ok(normalize_user(&user))
            }
            Err(SignInError::InvalidPassword(pw_token)) => {
                *self.password_token.lock().await = Some(pw_token);
                Err(AdapterError::InvalidPassword)
            }
            Err(other) => Err(other.into()),
        }
    }

    pub async fn sign_out(&self) -> AdapterResult<()> {
        self.client.sign_out().await?;
        *self.login_token.lock().await = None;
        *self.password_token.lock().await = None;
        let _ = self.session.save();
        Ok(())
    }
}
