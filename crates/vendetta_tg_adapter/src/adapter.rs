use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use grammers_client::Client;
use grammers_mtsender::{InvocationError, SenderPool};
use grammers_session::{
    Session,
    types::{ChannelKind, PeerAuth, PeerId as GrammersPeerId, PeerInfo},
};
use grammers_tl_types::{self as tl, Deserializable, Serializable};
use tokio::{sync::Mutex, task::JoinHandle};
use tracing::{debug, warn};
use vendetta_core::now_unix_secs;
use vendetta_model::{
    AccountSyncState, DialogFilterRecord, DialogInfo, FileRangeHash, MessageId, MessageRecord,
    PeerId, PeerRecord, PeerType,
};

use crate::{
    auth::TelegramAuthService,
    error::{AdapterError, AdapterResult},
    normalize::{
        normalize_dialog, normalize_message, normalize_peer_and_type, normalize_peer_enum,
        normalize_raw_chat, normalize_raw_user, normalize_update,
    },
    session::FileSession,
    traits::{
        ChannelDifferenceResult, CommonDifferenceResult, DialogDiscoveryResult, DialogsPage,
        HistoryPage, TelegramAdapter,
    },
};

pub struct GrammersTelegramAdapter {
    client: Arc<Client>,
    session: Arc<FileSession>,
    auth_service: Arc<TelegramAuthService>,
    peer_types: Arc<RwLock<HashMap<PeerId, PeerType>>>,
    auth_copied_to_dcs: Arc<Mutex<HashSet<i32>>>,
    _runner_handle: JoinHandle<()>,
}

impl GrammersTelegramAdapter {
    pub async fn connect(
        api_id: i32,
        api_hash: impl Into<String>,
        session: Arc<FileSession>,
    ) -> AdapterResult<Self> {
        let api_hash_str = api_hash.into();
        let pool = SenderPool::new(Arc::clone(&session), api_id);

        let runner_handle = tokio::spawn(async move {
            let runner = pool.runner;
            let _ = runner.run().await;
        });

        let client = Arc::new(Client::new(pool.handle));
        let auth_service = Arc::new(TelegramAuthService::new(
            Arc::clone(&client),
            Arc::clone(&session),
            api_id,
            api_hash_str,
        ));

        Ok(Self {
            client,
            session,
            auth_service,
            peer_types: Arc::new(RwLock::new(HashMap::new())),
            auth_copied_to_dcs: Arc::new(Mutex::new(HashSet::new())),
            _runner_handle: runner_handle,
        })
    }

    pub fn new_with_session(session: Arc<FileSession>) -> Self {
        let pool = SenderPool::new(Arc::clone(&session), 0);
        let client = Arc::new(Client::new(pool.handle));
        let auth_service = Arc::new(TelegramAuthService::new(
            Arc::clone(&client),
            Arc::clone(&session),
            0,
            String::new(),
        ));
        Self {
            client,
            session,
            auth_service,
            peer_types: Arc::new(RwLock::new(HashMap::new())),
            auth_copied_to_dcs: Arc::new(Mutex::new(HashSet::new())),
            _runner_handle: tokio::spawn(async {}),
        }
    }

    pub async fn ensure_auth_in_dc(&self, target_dc_id: i32) -> AdapterResult<()> {
        if target_dc_id == 0 {
            return Ok(());
        }

        let mut copied = self.auth_copied_to_dcs.lock().await;
        if copied.contains(&target_dc_id) {
            return Ok(());
        }

        let home_dc_id = self.session.lock_data().map(|d| d.home_dc).unwrap_or(0);
        if target_dc_id == home_dc_id {
            copied.insert(target_dc_id);
            return Ok(());
        }

        debug!(
            target: "vendetta::adapter",
            requested_dc = target_dc_id,
            home_dc = home_dc_id,
            connection_type = "cross_dc_transfer",
            has_user_auth = true,
            rpc_method = "auth.ExportAuthorization -> auth.ImportAuthorization",
            "Exporting and importing user authorization for target DC transfer connection"
        );

        let export_req = tl::functions::auth::ExportAuthorization {
            dc_id: target_dc_id,
        };
        let tl::enums::auth::ExportedAuthorization::Authorization(exported_auth) =
            self.client.invoke(&export_req).await?;

        let import_req = tl::functions::auth::ImportAuthorization {
            id: exported_auth.id,
            bytes: exported_auth.bytes,
        };
        self.client.invoke_in_dc(target_dc_id, &import_req).await?;

        copied.insert(target_dc_id);
        let _ = self.session.save();

        Ok(())
    }

    pub fn register_peer_type(&self, peer_id: PeerId, peer_type: PeerType) {
        self.peer_types.write().unwrap().insert(peer_id, peer_type);
    }

    pub fn auth(&self) -> &TelegramAuthService {
        &self.auth_service
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn session(&self) -> &FileSession {
        &self.session
    }

    pub fn resolve_canonical_peer_type(
        &self,
        peer_id: PeerId,
        peer_type_hint: Option<PeerType>,
    ) -> AdapterResult<PeerType> {
        if let Some(pt) = peer_type_hint {
            return Ok(pt);
        }
        self.peer_types
            .read()
            .unwrap()
            .get(&peer_id)
            .copied()
            .ok_or(AdapterError::UnknownPeerType(peer_id))
    }

    pub fn decode_user_id(peer_id: PeerId) -> i64 {
        peer_id.decode_user_id()
    }

    pub fn decode_group_id(peer_id: PeerId) -> i64 {
        peer_id.decode_group_id()
    }

    pub fn decode_channel_id(peer_id: PeerId) -> i64 {
        peer_id.decode_channel_id()
    }

    pub async fn resolve_input_peer(&self, peer_id: PeerId) -> AdapterResult<tl::enums::InputPeer> {
        let peer_type = self.resolve_canonical_peer_type(peer_id, None)?;
        let raw = peer_id.raw();

        let (input_peer, decoded_id, has_auth) = match peer_type {
            PeerType::Group if raw <= -1_000_000_000_000 => {
                let channel_id = Self::decode_channel_id(peer_id);
                let grammers_peer_id = GrammersPeerId::channel_unchecked(channel_id);
                if let Some(peer_ref) = self
                    .session
                    .peer_ref(grammers_peer_id)
                    .await
                    .map_err(|e| AdapterError::Session(e.to_string()))?
                {
                    let ip: tl::enums::InputPeer = peer_ref.into();
                    (ip, channel_id, true)
                } else {
                    return Err(AdapterError::PeerNotFoundOrUncached(peer_id));
                }
            }
            PeerType::Group => {
                let chat_id = Self::decode_group_id(peer_id);
                (
                    tl::enums::InputPeer::Chat(tl::types::InputPeerChat { chat_id }),
                    chat_id,
                    true,
                )
            }
            PeerType::Channel => {
                let channel_id = Self::decode_channel_id(peer_id);
                let grammers_peer_id = GrammersPeerId::channel_unchecked(channel_id);
                if let Some(peer_ref) = self
                    .session
                    .peer_ref(grammers_peer_id)
                    .await
                    .map_err(|e| AdapterError::Session(e.to_string()))?
                {
                    let ip: tl::enums::InputPeer = peer_ref.into();
                    (ip, channel_id, true)
                } else {
                    return Err(AdapterError::PeerNotFoundOrUncached(peer_id));
                }
            }
            PeerType::User => {
                let user_id = Self::decode_user_id(peer_id);
                let grammers_peer_id = GrammersPeerId::user_unchecked(user_id);
                if let Some(peer_ref) = self
                    .session
                    .peer_ref(grammers_peer_id)
                    .await
                    .map_err(|e| AdapterError::Session(e.to_string()))?
                {
                    let ip: tl::enums::InputPeer = peer_ref.into();
                    (ip, user_id, true)
                } else {
                    return Err(AdapterError::PeerNotFoundOrUncached(peer_id));
                }
            }
        };

        debug!(
            target: "vendetta::adapter",
            peer_id = peer_id.raw(),
            peer_type = ?peer_type,
            decoded_id = decoded_id,
            input_peer_constructor = match &input_peer {
                tl::enums::InputPeer::User(_) => "InputPeer::User",
                tl::enums::InputPeer::Chat(_) => "InputPeer::Chat",
                tl::enums::InputPeer::Channel(_) => "InputPeer::Channel",
                _ => "InputPeer::Other",
            },
            has_access_hash = has_auth,
            "Resolved InputPeer for RPC"
        );

        Ok(input_peer)
    }

    pub async fn resolve_input_channel(
        &self,
        peer_id: PeerId,
    ) -> AdapterResult<tl::enums::InputChannel> {
        let peer_type = self.resolve_canonical_peer_type(peer_id, None)?;
        let raw = peer_id.raw();

        if peer_type != PeerType::Channel
            && (peer_type != PeerType::Group || raw > -1_000_000_000_000)
        {
            return Err(AdapterError::InvalidPeerType {
                peer_id,
                expected: "Channel or Supergroup",
            });
        }

        let channel_id = Self::decode_channel_id(peer_id);
        let grammers_peer_id = GrammersPeerId::channel_unchecked(channel_id);

        if let Some(peer_ref) = self
            .session
            .peer_ref(grammers_peer_id)
            .await
            .map_err(|e| AdapterError::Session(e.to_string()))?
        {
            let ic: tl::enums::InputChannel = match peer_ref.into() {
                tl::enums::InputPeer::Channel(c) => {
                    tl::enums::InputChannel::Channel(tl::types::InputChannel {
                        channel_id: c.channel_id,
                        access_hash: c.access_hash,
                    })
                }
                _ => {
                    return Err(AdapterError::InvalidPeerType {
                        peer_id,
                        expected: "Channel or Supergroup",
                    });
                }
            };

            debug!(
                target: "vendetta::adapter",
                peer_id = peer_id.raw(),
                peer_type = ?peer_type,
                decoded_id = channel_id,
                input_channel_constructor = "InputChannel::Channel",
                has_access_hash = true,
                "Resolved InputChannel for RPC"
            );

            Ok(ic)
        } else {
            Err(AdapterError::PeerNotFoundOrUncached(peer_id))
        }
    }

    async fn cache_auxiliary_peers(
        &self,
        users: &[tl::enums::User],
        chats: &[tl::enums::Chat],
    ) -> AdapterResult<Vec<PeerRecord>> {
        let mut normalized_peers = Vec::new();

        for user in users {
            if let tl::enums::User::User(u) = user
                && let Some(access_hash) = u.access_hash
            {
                let info = PeerInfo::User {
                    id: u.id,
                    auth: Some(PeerAuth::from_hash(access_hash)),
                    bot: Some(u.bot),
                    is_self: Some(u.is_self),
                };
                let _ = self.session.cache_peer(&info).await;
            }
            if let Some(norm) = normalize_raw_user(user) {
                self.peer_types
                    .write()
                    .unwrap()
                    .insert(norm.peer_id, norm.peer_type);
                normalized_peers.push(norm);
            }
        }

        for chat in chats {
            match chat {
                tl::enums::Chat::Channel(c) => {
                    if let Some(access_hash) = c.access_hash {
                        let info = PeerInfo::Channel {
                            id: c.id,
                            auth: Some(PeerAuth::from_hash(access_hash)),
                            kind: if c.broadcast {
                                Some(ChannelKind::Broadcast)
                            } else {
                                Some(ChannelKind::Megagroup)
                            },
                        };
                        let _ = self.session.cache_peer(&info).await;
                    }
                }
                tl::enums::Chat::Chat(c) => {
                    let info = PeerInfo::Chat { id: c.id };
                    let _ = self.session.cache_peer(&info).await;
                }
                _ => {}
            }
            if let Some(norm) = normalize_raw_chat(chat) {
                self.peer_types
                    .write()
                    .unwrap()
                    .insert(norm.peer_id, norm.peer_type);
                normalized_peers.push(norm);
            }
        }

        Ok(normalized_peers)
    }
}

fn map_input_peer_to_peer_id(p: &tl::enums::InputPeer) -> Option<PeerId> {
    match p {
        tl::enums::InputPeer::User(u) => Some(PeerId::new(u.user_id)),
        tl::enums::InputPeer::Chat(c) => Some(PeerId::new(-c.chat_id)),
        tl::enums::InputPeer::Channel(c) => Some(PeerId::new(-1_000_000_000_000 - c.channel_id)),
        _ => None,
    }
}

fn unpack_messages_response(
    resp: tl::enums::messages::Messages,
) -> (
    Vec<tl::enums::Message>,
    Vec<tl::enums::Chat>,
    Vec<tl::enums::User>,
) {
    match resp {
        tl::enums::messages::Messages::Messages(m) => (m.messages, m.chats, m.users),
        tl::enums::messages::Messages::Slice(s) => (s.messages, s.chats, s.users),
        tl::enums::messages::Messages::ChannelMessages(c) => (c.messages, c.chats, c.users),
        tl::enums::messages::Messages::NotModified(_) => (Vec::new(), Vec::new(), Vec::new()),
    }
}

#[async_trait]
impl TelegramAdapter for GrammersTelegramAdapter {
    async fn get_dialogs(&self) -> AdapterResult<Vec<PeerRecord>> {
        let mut iter = self.client.iter_dialogs();
        let mut seen = HashSet::new();
        let mut records = Vec::new();

        while let Some(dialog) = iter.next().await? {
            let record = normalize_dialog(&dialog);
            if seen.insert(record.peer_id) {
                self.peer_types
                    .write()
                    .unwrap()
                    .insert(record.peer_id, record.peer_type);
                records.push(record);
            }
        }

        let _ = self.session.save();
        Ok(records)
    }

    async fn get_state(&self) -> AdapterResult<AccountSyncState> {
        let request = tl::functions::updates::GetState {};
        let response = self.client.invoke(&request).await?;
        let now = now_unix_secs();

        match response {
            tl::enums::updates::State::State(s) => Ok(AccountSyncState {
                account_id: "default".to_string(),
                pts: s.pts,
                qts: s.qts,
                date: s.date,
                seq: s.seq,
                sync_uncertain: false,
                last_synced_at: now,
            }),
        }
    }

    async fn get_difference(
        &self,
        pts: i32,
        date: i32,
        qts: i32,
    ) -> AdapterResult<CommonDifferenceResult> {
        let request = tl::functions::updates::GetDifference {
            pts,
            pts_total_limit: None,
            pts_limit: None,
            qts_limit: None,
            date,
            qts,
        };

        let response = self.client.invoke(&request).await?;
        let now = now_unix_secs();

        match response {
            tl::enums::updates::Difference::Empty(e) => Ok(CommonDifferenceResult::Empty {
                date: e.date,
                seq: e.seq,
            }),
            tl::enums::updates::Difference::Slice(s) => {
                let auxiliary_peers = self.cache_auxiliary_peers(&s.users, &s.chats).await?;
                let new_messages = s
                    .new_messages
                    .into_iter()
                    .map(|msg| normalize_message(&msg, None))
                    .collect();
                let other_updates = s.other_updates.iter().map(normalize_update).collect();
                let intermediate_state = match s.intermediate_state {
                    tl::enums::updates::State::State(st) => AccountSyncState {
                        account_id: "default".to_string(),
                        pts: st.pts,
                        qts: st.qts,
                        date: st.date,
                        seq: st.seq,
                        sync_uncertain: false,
                        last_synced_at: now,
                    },
                };
                let _ = self.session.save();
                Ok(CommonDifferenceResult::Slice {
                    new_messages,
                    other_updates,
                    auxiliary_peers,
                    intermediate_state,
                })
            }
            tl::enums::updates::Difference::Difference(d) => {
                let auxiliary_peers = self.cache_auxiliary_peers(&d.users, &d.chats).await?;
                let new_messages = d
                    .new_messages
                    .into_iter()
                    .map(|msg| normalize_message(&msg, None))
                    .collect();
                let other_updates = d.other_updates.iter().map(normalize_update).collect();
                let state = match d.state {
                    tl::enums::updates::State::State(st) => AccountSyncState {
                        account_id: "default".to_string(),
                        pts: st.pts,
                        qts: st.qts,
                        date: st.date,
                        seq: st.seq,
                        sync_uncertain: false,
                        last_synced_at: now,
                    },
                };
                let _ = self.session.save();
                Ok(CommonDifferenceResult::Difference {
                    new_messages,
                    other_updates,
                    auxiliary_peers,
                    state,
                })
            }
            tl::enums::updates::Difference::TooLong(t) => {
                Ok(CommonDifferenceResult::TooLong { pts: t.pts })
            }
        }
    }

    async fn get_channel_difference(
        &self,
        channel_id: PeerId,
        pts: i32,
        limit: usize,
    ) -> AdapterResult<ChannelDifferenceResult> {
        let input_channel = self.resolve_input_channel(channel_id).await?;
        let request = tl::functions::updates::GetChannelDifference {
            force: false,
            channel: input_channel,
            filter: tl::enums::ChannelMessagesFilter::Empty,
            pts,
            limit: limit as i32,
        };

        let response = self.client.invoke(&request).await?;

        match response {
            tl::enums::updates::ChannelDifference::Empty(e) => Ok(ChannelDifferenceResult::Empty {
                final_state: e.r#final,
                pts: e.pts,
                timeout: e.timeout,
            }),
            tl::enums::updates::ChannelDifference::Difference(d) => {
                let auxiliary_peers = self.cache_auxiliary_peers(&d.users, &d.chats).await?;
                let new_messages = d
                    .new_messages
                    .into_iter()
                    .map(|msg| normalize_message(&msg, Some(channel_id)))
                    .collect();
                let other_updates = d.other_updates.iter().map(normalize_update).collect();
                let _ = self.session.save();
                Ok(ChannelDifferenceResult::Difference {
                    final_state: d.r#final,
                    pts: d.pts,
                    timeout: d.timeout,
                    new_messages,
                    other_updates,
                    auxiliary_peers,
                })
            }
            tl::enums::updates::ChannelDifference::TooLong(t) => {
                let auxiliary_peers = self.cache_auxiliary_peers(&t.users, &t.chats).await?;
                let messages = t
                    .messages
                    .into_iter()
                    .map(|msg| normalize_message(&msg, Some(channel_id)))
                    .collect();
                let (dialog_pts, top_message) = match &t.dialog {
                    tl::enums::Dialog::Dialog(d) => (
                        d.pts.unwrap_or(0),
                        Some(MessageId::new(d.top_message as i64)),
                    ),
                    tl::enums::Dialog::Community(_) => (0, None),
                    tl::enums::Dialog::Folder(_) => (0, None),
                };
                let _ = self.session.save();
                Ok(ChannelDifferenceResult::TooLong {
                    final_state: t.r#final,
                    timeout: t.timeout,
                    dialog_pts,
                    top_message,
                    messages,
                    auxiliary_peers,
                })
            }
        }
    }

    async fn get_dialog_filters(&self) -> AdapterResult<Vec<DialogFilterRecord>> {
        let request = tl::functions::messages::GetDialogFilters {};
        let response = self.client.invoke(&request).await?;

        let filters_vec = match response {
            tl::enums::messages::DialogFilters::Filters(df) => df.filters,
        };

        let mut filters = Vec::new();
        for filter in filters_vec {
            match filter {
                tl::enums::DialogFilter::Filter(f) => {
                    let pinned_peers: Vec<PeerId> = f
                        .pinned_peers
                        .iter()
                        .filter_map(map_input_peer_to_peer_id)
                        .collect();
                    let include_peers = f
                        .include_peers
                        .iter()
                        .filter_map(map_input_peer_to_peer_id)
                        .collect();
                    let exclude_peers = f
                        .exclude_peers
                        .iter()
                        .filter_map(map_input_peer_to_peer_id)
                        .collect();

                    let title = match f.title {
                        tl::enums::TextWithEntities::Entities(e) => e.text,
                    };

                    filters.push(DialogFilterRecord {
                        id: f.id,
                        title,
                        pinned_peers,
                        include_peers,
                        exclude_peers,
                    });
                }
                tl::enums::DialogFilter::Chatlist(c) => {
                    let pinned_peers = c
                        .pinned_peers
                        .iter()
                        .filter_map(map_input_peer_to_peer_id)
                        .collect();
                    let include_peers = c
                        .include_peers
                        .iter()
                        .filter_map(map_input_peer_to_peer_id)
                        .collect();

                    let title = match c.title {
                        tl::enums::TextWithEntities::Entities(e) => e.text,
                    };

                    filters.push(DialogFilterRecord {
                        id: c.id,
                        title,
                        pinned_peers,
                        include_peers,
                        exclude_peers: Vec::new(),
                    });
                }
                tl::enums::DialogFilter::Default => {}
            }
        }

        Ok(filters)
    }

    async fn get_dialogs_paginated(
        &self,
        folder_id: i32,
        offset_date: i32,
        offset_id: i32,
        offset_peer: Option<PeerId>,
        limit: usize,
    ) -> AdapterResult<DialogsPage> {
        let input_offset_peer = if let Some(peer_id) = offset_peer {
            self.resolve_input_peer(peer_id).await?
        } else {
            tl::enums::InputPeer::Empty
        };

        let request = tl::functions::messages::GetDialogs {
            exclude_pinned: false,
            folder_id: if folder_id == 0 {
                None
            } else {
                Some(folder_id)
            },
            offset_date,
            offset_id,
            offset_peer: input_offset_peer,
            limit: limit as i32,
            hash: 0,
        };

        let response = self.client.invoke(&request).await?;

        let (dialogs, messages, chats, users, is_last_page) = match response {
            tl::enums::messages::Dialogs::Dialogs(d) => {
                (d.dialogs, d.messages, d.chats, d.users, true)
            }
            tl::enums::messages::Dialogs::Slice(s) => {
                (s.dialogs, s.messages, s.chats, s.users, false)
            }
            tl::enums::messages::Dialogs::NotModified(_) => {
                (Vec::new(), Vec::new(), Vec::new(), Vec::new(), true)
            }
        };

        let auxiliary_peers = self.cache_auxiliary_peers(&users, &chats).await?;

        let mut dialog_infos = Vec::new();
        for d in &dialogs {
            if let tl::enums::Dialog::Dialog(diag) = d {
                let (peer_id, peer_type) = normalize_peer_and_type(&diag.peer);
                dialog_infos.push(DialogInfo {
                    peer_id,
                    peer_type: Some(peer_type),
                    pts: diag.pts,
                    top_message: Some(MessageId::new(diag.top_message as i64)),
                    unread_count: diag.unread_count,
                    is_pinned: diag.pinned,
                    folder_id: diag.folder_id,
                    is_unresolved: false,
                });
            }
        }

        let (next_offset_date, next_offset_id, next_offset_peer) =
            if let Some(last_msg) = messages.last() {
                match last_msg {
                    tl::enums::Message::Message(m) => {
                        let pid = normalize_peer_enum(&m.peer_id);
                        (m.date, m.id, Some(pid))
                    }
                    tl::enums::Message::Service(s) => {
                        let pid = normalize_peer_enum(&s.peer_id);
                        (s.date, s.id, Some(pid))
                    }
                    tl::enums::Message::Empty(e) => (0, e.id, None),
                }
            } else {
                (0, 0, None)
            };

        Ok(DialogsPage {
            dialogs: dialog_infos,
            auxiliary_peers,
            is_last_page,
            next_offset_date,
            next_offset_id,
            next_offset_peer,
        })
    }

    async fn get_peer_dialogs(&self, peers: &[PeerId]) -> AdapterResult<Vec<DialogInfo>> {
        let mut input_dialog_peers = Vec::new();
        for &peer_id in peers {
            if let Ok(input_peer) = self.resolve_input_peer(peer_id).await {
                input_dialog_peers.push(tl::enums::InputDialogPeer::Peer(
                    tl::types::InputDialogPeer { peer: input_peer },
                ));
            }
        }

        if input_dialog_peers.is_empty() {
            return Ok(Vec::new());
        }

        let request = tl::functions::messages::GetPeerDialogs {
            peers: input_dialog_peers,
        };

        let response = self.client.invoke(&request).await?;
        let (dialogs, chats, users) = match response {
            tl::enums::messages::PeerDialogs::Dialogs(d) => (d.dialogs, d.chats, d.users),
        };

        let _ = self.cache_auxiliary_peers(&users, &chats).await;

        let mut results = Vec::new();
        for d in dialogs {
            if let tl::enums::Dialog::Dialog(diag) = d {
                let (peer_id, peer_type) = normalize_peer_and_type(&diag.peer);
                results.push(DialogInfo {
                    peer_id,
                    peer_type: Some(peer_type),
                    pts: diag.pts,
                    top_message: Some(MessageId::new(diag.top_message as i64)),
                    unread_count: diag.unread_count,
                    is_pinned: diag.pinned,
                    folder_id: diag.folder_id,
                    is_unresolved: false,
                });
            }
        }

        Ok(results)
    }

    async fn get_all_dialogs_complete(
        &self,
        local_channels: &[PeerId],
    ) -> AdapterResult<DialogDiscoveryResult> {
        let mut all_dialogs = Vec::new();
        let mut all_auxiliary_peers = Vec::new();
        let mut seen_peers = HashSet::new();
        let mut is_complete = true;

        let mut offset_date = 0;
        let mut offset_id = 0;
        let mut offset_peer = None;
        loop {
            let page = self
                .get_dialogs_paginated(0, offset_date, offset_id, offset_peer, 100)
                .await?;
            for diag in page.dialogs {
                if seen_peers.insert(diag.peer_id) {
                    all_dialogs.push(diag);
                }
            }
            all_auxiliary_peers.extend(page.auxiliary_peers);

            if page.is_last_page || (page.next_offset_id == 0 && page.next_offset_date == 0) {
                break;
            }
            offset_date = page.next_offset_date;
            offset_id = page.next_offset_id;
            offset_peer = page.next_offset_peer;
        }

        offset_date = 0;
        offset_id = 0;
        offset_peer = None;
        loop {
            let page = match self
                .get_dialogs_paginated(1, offset_date, offset_id, offset_peer, 100)
                .await
            {
                Ok(p) => p,
                Err(e) => {
                    warn!("Archived dialog folder 1 enumeration returned error: {e}");
                    is_complete = false;
                    break;
                }
            };
            for diag in page.dialogs {
                if seen_peers.insert(diag.peer_id) {
                    all_dialogs.push(diag);
                }
            }
            all_auxiliary_peers.extend(page.auxiliary_peers);

            if page.is_last_page || (page.next_offset_id == 0 && page.next_offset_date == 0) {
                break;
            }
            offset_date = page.next_offset_date;
            offset_id = page.next_offset_id;
            offset_peer = page.next_offset_peer;
        }

        let custom_filters = match self.get_dialog_filters().await {
            Ok(f) => f,
            Err(e) => {
                warn!("Failed to fetch custom dialog filters: {e}");
                is_complete = false;
                Vec::new()
            }
        };

        let mut filter_peers_to_query = Vec::new();
        for filter in &custom_filters {
            let excluded: HashSet<PeerId> = filter.exclude_peers.iter().copied().collect();
            for &pid in filter
                .pinned_peers
                .iter()
                .chain(filter.include_peers.iter())
            {
                if !excluded.contains(&pid) && seen_peers.insert(pid) {
                    filter_peers_to_query.push(pid);
                }
            }
        }
        if !filter_peers_to_query.is_empty() {
            match self.get_peer_dialogs(&filter_peers_to_query).await {
                Ok(peer_dialogs) => {
                    for diag in peer_dialogs {
                        all_dialogs.push(diag);
                    }
                }
                Err(e) => {
                    warn!("Failed to fetch peer dialogs for custom filter peers: {e}");
                    is_complete = false;
                    for pid in filter_peers_to_query {
                        all_dialogs.push(DialogInfo {
                            peer_id: pid,
                            peer_type: None,
                            pts: None,
                            top_message: None,
                            unread_count: 0,
                            is_pinned: false,
                            folder_id: None,
                            is_unresolved: true,
                        });
                    }
                }
            }
        }

        let missing_channels: Vec<PeerId> = local_channels
            .iter()
            .copied()
            .filter(|p| !seen_peers.contains(p))
            .collect();

        if !missing_channels.is_empty() {
            match self.get_peer_dialogs(&missing_channels).await {
                Ok(peer_dialogs) => {
                    for diag in peer_dialogs {
                        if seen_peers.insert(diag.peer_id) {
                            all_dialogs.push(diag);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to fetch peer dialogs for dormant channels: {e}");
                    is_complete = false;
                    for pid in missing_channels {
                        if seen_peers.insert(pid) {
                            all_dialogs.push(DialogInfo {
                                peer_id: pid,
                                peer_type: Some(PeerType::Channel),
                                pts: None,
                                top_message: None,
                                unread_count: 0,
                                is_pinned: false,
                                folder_id: None,
                                is_unresolved: true,
                            });
                        }
                    }
                }
            }
        }

        Ok(DialogDiscoveryResult {
            discovered_dialogs: all_dialogs,
            auxiliary_peers: all_auxiliary_peers,
            custom_filters,
            is_complete,
        })
    }

    async fn get_history_page(
        &self,
        peer_id: PeerId,
        limit: usize,
        offset_id: Option<MessageId>,
    ) -> AdapterResult<HistoryPage> {
        self.get_history_page_filtered(peer_id, None, None, offset_id, limit)
            .await
    }

    async fn get_history_page_filtered(
        &self,
        peer_id: PeerId,
        min_id: Option<MessageId>,
        max_id: Option<MessageId>,
        offset_id: Option<MessageId>,
        limit: usize,
    ) -> AdapterResult<HistoryPage> {
        let input_peer = self.resolve_input_peer(peer_id).await?;

        debug!(
            target: "vendetta::adapter",
            peer_id = peer_id.raw(),
            rpc_method = "messages.GetHistory",
            "Invoking messages.GetHistory RPC"
        );

        let request = tl::functions::messages::GetHistory {
            peer: input_peer,
            offset_id: offset_id.map(|id| id.raw() as i32).unwrap_or(0),
            offset_date: 0,
            add_offset: 0,
            limit: limit as i32,
            max_id: max_id.map(|id| id.raw() as i32).unwrap_or(0),
            min_id: min_id.map(|id| id.raw() as i32).unwrap_or(0),
            hash: 0,
        };

        let response = self.client.invoke(&request).await?;

        let (messages, chats, users, pts, count, topics) = match response {
            tl::enums::messages::Messages::Messages(m) => {
                (m.messages, m.chats, m.users, None, None, Vec::new())
            }
            tl::enums::messages::Messages::Slice(s) => (
                s.messages,
                s.chats,
                s.users,
                None,
                Some(s.count),
                Vec::new(),
            ),
            tl::enums::messages::Messages::ChannelMessages(c) => (
                c.messages,
                c.chats,
                c.users,
                Some(c.pts),
                Some(c.count),
                c.topics,
            ),
            tl::enums::messages::Messages::NotModified(_) => {
                (Vec::new(), Vec::new(), Vec::new(), None, None, Vec::new())
            }
        };

        let auxiliary_peers = self.cache_auxiliary_peers(&users, &chats).await?;
        let raw_topics = topics.iter().map(|t| t.to_bytes()).collect();

        let message_records = messages
            .into_iter()
            .map(|msg| normalize_message(&msg, Some(peer_id)))
            .collect();

        let _ = self.session.save();

        Ok(HistoryPage {
            messages: message_records,
            auxiliary_peers,
            pts,
            count,
            raw_topics,
        })
    }

    async fn get_messages(
        &self,
        peer_id: PeerId,
        peer_type: Option<PeerType>,
        message_ids: &[MessageId],
    ) -> AdapterResult<Vec<MessageRecord>> {
        let input_messages: Vec<tl::enums::InputMessage> = message_ids
            .iter()
            .map(|id| {
                tl::enums::InputMessage::Id(tl::types::InputMessageId {
                    id: id.raw() as i32,
                })
            })
            .collect();

        let effective_peer_type = self.resolve_canonical_peer_type(peer_id, peer_type)?;
        let is_channel = effective_peer_type == PeerType::Channel
            || (effective_peer_type == PeerType::Group && peer_id.raw() <= -1_000_000_000_000);

        let (messages, chats, users) = if is_channel {
            let input_channel = self.resolve_input_channel(peer_id).await?;
            debug!(
                target: "vendetta::adapter",
                peer_id = peer_id.raw(),
                rpc_method = "channels.GetMessages",
                "Invoking channels.GetMessages RPC"
            );
            let request = tl::functions::channels::GetMessages {
                channel: input_channel,
                id: input_messages,
            };
            unpack_messages_response(self.client.invoke(&request).await?)
        } else {
            debug!(
                target: "vendetta::adapter",
                peer_id = peer_id.raw(),
                rpc_method = "messages.GetMessages",
                "Invoking messages.GetMessages RPC"
            );
            let request = tl::functions::messages::GetMessages { id: input_messages };
            unpack_messages_response(self.client.invoke(&request).await?)
        };

        let _ = self.cache_auxiliary_peers(&users, &chats).await;

        let records = messages
            .into_iter()
            .map(|msg| normalize_message(&msg, Some(peer_id)))
            .collect();

        Ok(records)
    }

    async fn resolve_reply_target(
        &self,
        source_peer: PeerId,
        source_peer_type: Option<PeerType>,
        target_peer: Option<PeerId>,
        target_peer_type: Option<PeerType>,
        target_msg_id: MessageId,
    ) -> AdapterResult<Option<MessageRecord>> {
        let target_peer_id = target_peer.unwrap_or(source_peer);
        let target_type = target_peer_type.or(if target_peer.is_none() {
            source_peer_type
        } else {
            None
        });
        let msgs = self
            .get_messages(target_peer_id, target_type, &[target_msg_id])
            .await?;
        Ok(msgs.into_iter().find(|m| m.key.message_id == target_msg_id))
    }

    async fn download_file_chunk(
        &self,
        location_tl: &[u8],
        dc_id: i32,
        offset: i64,
        limit: i32,
    ) -> AdapterResult<Vec<u8>> {
        let location = tl::enums::InputFileLocation::from_bytes(location_tl).map_err(|e| {
            AdapterError::Invocation(format!("Failed to deserialize InputFileLocation: {e}"))
        })?;

        let mut current_dc = dc_id;
        let mut retry_auth_once = true;

        loop {
            if current_dc != 0 {
                self.ensure_auth_in_dc(current_dc).await?;
            }

            debug!(
                target: "vendetta::adapter",
                requested_dc = current_dc,
                connection_type = if current_dc != 0 { "per_dc_transfer" } else { "home_dc_transfer" },
                has_user_auth = true,
                offset = offset,
                limit = limit,
                rpc_method = "upload.getFile",
                "Invoking upload.getFile RPC"
            );

            let request = tl::functions::upload::GetFile {
                precise: true,
                cdn_supported: false,
                location: location.clone(),
                offset,
                limit,
            };

            let response = if current_dc != 0 {
                self.client.invoke_in_dc(current_dc, &request).await
            } else {
                self.client.invoke(&request).await
            };

            match response {
                Ok(tl::enums::upload::File::File(f)) => return Ok(f.bytes),
                Ok(tl::enums::upload::File::CdnRedirect(r)) => {
                    return Err(AdapterError::CdnRedirectUnsupported {
                        dc_id: r.dc_id,
                        file_token: r.file_token,
                    });
                }
                Err(InvocationError::Rpc(rpc))
                    if (rpc.code == 303 || rpc.name.starts_with("FILE_MIGRATE_"))
                        && rpc.value.is_some() =>
                {
                    let new_dc = rpc.value.unwrap() as i32;
                    debug!(
                        target: "vendetta::adapter",
                        from_dc = current_dc,
                        to_dc = new_dc,
                        "File migrated to new DC, switching DC and retrying"
                    );
                    current_dc = new_dc;
                    continue;
                }
                Err(InvocationError::Rpc(rpc))
                    if (rpc.code == 401
                        || rpc.name == "AUTH_KEY_UNREGISTERED"
                        || rpc.name == "USER_DEACTIVATED")
                        && current_dc != 0
                        && retry_auth_once =>
                {
                    warn!(
                        target: "vendetta::adapter",
                        dc_id = current_dc,
                        rpc_error = ?rpc,
                        "Received auth error on transfer DC, re-exporting authorization and retrying"
                    );
                    self.auth_copied_to_dcs.lock().await.remove(&current_dc);
                    retry_auth_once = false;
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    async fn get_file_hashes(
        &self,
        location_tl: &[u8],
        dc_id: i32,
        offset: i64,
    ) -> AdapterResult<Vec<FileRangeHash>> {
        let location = tl::enums::InputFileLocation::from_bytes(location_tl).map_err(|e| {
            AdapterError::Invocation(format!("Failed to deserialize InputFileLocation: {e}"))
        })?;

        let mut current_dc = dc_id;
        let mut retry_auth_once = true;

        loop {
            if current_dc != 0 {
                self.ensure_auth_in_dc(current_dc).await?;
            }

            debug!(
                target: "vendetta::adapter",
                requested_dc = current_dc,
                connection_type = if current_dc != 0 { "per_dc_transfer" } else { "home_dc_transfer" },
                has_user_auth = true,
                offset = offset,
                rpc_method = "upload.getFileHashes",
                "Invoking upload.getFileHashes RPC"
            );

            let request = tl::functions::upload::GetFileHashes {
                location: location.clone(),
                offset,
            };

            let response = if current_dc != 0 {
                self.client.invoke_in_dc(current_dc, &request).await
            } else {
                self.client.invoke(&request).await
            };

            match response {
                Ok(hashes) => {
                    let out = hashes
                        .into_iter()
                        .map(|tl::enums::FileHash::Hash(hash)| FileRangeHash {
                            offset: hash.offset,
                            limit: hash.limit,
                            hash: hash.hash,
                        })
                        .collect();
                    return Ok(out);
                }
                Err(InvocationError::Rpc(rpc))
                    if (rpc.code == 303 || rpc.name.starts_with("FILE_MIGRATE_"))
                        && rpc.value.is_some() =>
                {
                    let new_dc = rpc.value.unwrap() as i32;
                    debug!(
                        target: "vendetta::adapter",
                        from_dc = current_dc,
                        to_dc = new_dc,
                        "File migrated to new DC in get_file_hashes, switching DC and retrying"
                    );
                    current_dc = new_dc;
                    continue;
                }
                Err(InvocationError::Rpc(rpc))
                    if (rpc.code == 401 || rpc.name == "AUTH_KEY_UNREGISTERED")
                        && current_dc != 0
                        && retry_auth_once =>
                {
                    warn!(
                        target: "vendetta::adapter",
                        dc_id = current_dc,
                        rpc_error = ?rpc,
                        "Received auth error on transfer DC in get_file_hashes, re-exporting authorization and retrying"
                    );
                    self.auth_copied_to_dcs.lock().await.remove(&current_dc);
                    retry_auth_once = false;
                    continue;
                }
                Err(e) => {
                    let mapped: AdapterError = e.into();
                    match &mapped {
                        AdapterError::Invocation(msg)
                            if msg.contains("LOCATION_INVALID")
                                || msg.contains("FILE_ID_INVALID")
                                || msg.contains("METHOD_NOT_SUPPORTED") =>
                        {
                            debug!(
                                "upload.getFileHashes not available for this location: {mapped}"
                            );
                            return Ok(Vec::new());
                        }
                        _ => return Err(mapped),
                    }
                }
            }
        }
    }

    async fn get_custom_emoji_documents(
        &self,
        document_ids: &[i64],
    ) -> AdapterResult<Vec<tl::enums::Document>> {
        if document_ids.is_empty() {
            return Ok(Vec::new());
        }
        let res = self
            .client
            .invoke(&tl::functions::messages::GetCustomEmojiDocuments {
                document_id: document_ids.to_vec(),
            })
            .await
            .map_err(AdapterError::from)?;
        Ok(res)
    }

    async fn is_authorized(&self) -> AdapterResult<bool> {
        self.client.is_authorized().await.map_err(|e| e.into())
    }
}
