use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use grammers_session::{
    BoxFuture, Session, SessionData,
    types::{ChannelState, DcOption, PeerId, PeerInfo, UpdateState, UpdatesState},
};
use serde::{Deserialize, Serialize};

use crate::error::{AdapterError, AdapterResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub home_dc: i32,
    pub dc_options: Vec<DcOption>,
    pub peer_infos: Vec<PeerInfo>,
    pub updates_state: UpdatesState,
}

impl From<&SessionData> for SessionState {
    fn from(data: &SessionData) -> Self {
        Self {
            home_dc: data.home_dc,
            dc_options: data.dc_options.values().cloned().collect(),
            peer_infos: data.peer_infos.values().cloned().collect(),
            updates_state: data.updates_state.clone(),
        }
    }
}

impl From<SessionState> for SessionData {
    fn from(state: SessionState) -> Self {
        Self {
            home_dc: state.home_dc,
            dc_options: state.dc_options.into_iter().map(|dc| (dc.id, dc)).collect(),
            peer_infos: state.peer_infos.into_iter().map(|p| (p.id(), p)).collect(),
            updates_state: state.updates_state,
        }
    }
}

pub fn load_session_file(path: impl AsRef<Path>) -> AdapterResult<Option<SessionData>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)?;
    let state: SessionState = serde_json::from_str(&content)?;
    Ok(Some(state.into()))
}

pub fn save_session_file(path: impl AsRef<Path>, data: &SessionData) -> AdapterResult<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let state = SessionState::from(data);
    let json = serde_json::to_string_pretty(&state)?;

    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, json)?;
    fs::rename(tmp_path, path)?;

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum FileSessionError {
    #[error("session mutex poisoned")]
    Poisoned,
    #[error("session I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("session JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub struct FileSession {
    data: Mutex<SessionData>,
    path: Option<PathBuf>,
}

impl FileSession {
    pub fn in_memory() -> Self {
        Self {
            data: Mutex::new(SessionData::default()),
            path: None,
        }
    }

    pub fn open(path: impl AsRef<Path>) -> AdapterResult<Arc<Self>> {
        let path_buf = path.as_ref().to_path_buf();
        let session_data = load_session_file(&path_buf)?.unwrap_or_default();
        Ok(Arc::new(Self {
            data: Mutex::new(session_data),
            path: Some(path_buf),
        }))
    }

    pub fn get_session_state(&self) -> Result<SessionState, FileSessionError> {
        let guard = self.data.lock().map_err(|_| FileSessionError::Poisoned)?;
        Ok(SessionState::from(&*guard))
    }

    pub fn save(&self) -> Result<(), FileSessionError> {
        if let Some(ref path) = self.path {
            let guard = self.data.lock().map_err(|_| FileSessionError::Poisoned)?;
            save_session_file(path, &guard).map_err(|e| match e {
                AdapterError::Io(io) => FileSessionError::Io(io),
                AdapterError::Serialization(s) => FileSessionError::Json(s),
                _ => FileSessionError::Io(std::io::Error::other(e.to_string())),
            })?;
        }
        Ok(())
    }

    pub(crate) fn lock_data(&self) -> Result<MutexGuard<'_, SessionData>, FileSessionError> {
        self.data.lock().map_err(|_| FileSessionError::Poisoned)
    }
}

impl Session for FileSession {
    type Error = FileSessionError;

    fn home_dc_id(&self) -> Result<i32, Self::Error> {
        Ok(self.lock_data()?.home_dc)
    }

    fn set_home_dc_id(&self, dc_id: i32) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            let mut data = self.lock_data()?;
            data.home_dc = dc_id;
            Ok(())
        })
    }

    fn dc_option(&self, dc_id: i32) -> Result<Option<DcOption>, Self::Error> {
        Ok(self.lock_data()?.dc_options.get(&dc_id).cloned())
    }

    fn set_dc_option(&self, dc_option: &DcOption) -> BoxFuture<'_, Result<(), Self::Error>> {
        let dc_option = dc_option.clone();
        Box::pin(async move {
            let mut data = self.lock_data()?;
            data.dc_options.insert(dc_option.id, dc_option);
            Ok(())
        })
    }

    fn peer(&self, peer: PeerId) -> BoxFuture<'_, Result<Option<PeerInfo>, Self::Error>> {
        Box::pin(async move { Ok(self.lock_data()?.peer_infos.get(&peer).cloned()) })
    }

    fn cache_peer(&self, peer: &PeerInfo) -> BoxFuture<'_, Result<(), Self::Error>> {
        let peer = peer.clone();
        Box::pin(async move {
            let mut data = self.lock_data()?;
            data.peer_infos
                .entry(peer.id())
                .or_insert_with(|| peer.clone())
                .extend_info(&peer);
            Ok(())
        })
    }

    fn updates_state(&self) -> BoxFuture<'_, Result<UpdatesState, Self::Error>> {
        Box::pin(async move { Ok(self.lock_data()?.updates_state.clone()) })
    }

    fn set_update_state(&self, update: UpdateState) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            let mut data = self.lock_data()?;
            match update {
                UpdateState::All(updates_state) => {
                    data.updates_state = updates_state;
                }
                UpdateState::Primary { pts, date, seq } => {
                    data.updates_state.pts = pts;
                    data.updates_state.date = date;
                    data.updates_state.seq = seq;
                }
                UpdateState::Secondary { qts } => {
                    data.updates_state.qts = qts;
                }
                UpdateState::Channel { id, pts } => {
                    data.updates_state.channels.retain(|c| c.id != id);
                    data.updates_state.channels.push(ChannelState { id, pts });
                }
            }
            Ok(())
        })
    }
}
