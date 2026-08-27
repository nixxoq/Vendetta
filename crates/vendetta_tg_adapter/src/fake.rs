use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use grammers_tl_types::enums::Document;
use vendetta_model::{
    AccountSyncState, DialogFilterRecord, DialogInfo, FileRangeHash, MessageId, MessageRecord,
    PeerId, PeerRecord, PeerType,
};

use crate::{
    error::{AdapterError, AdapterResult},
    traits::{
        ChannelDifferenceResult, CommonDifferenceResult, DialogDiscoveryResult, DialogsPage,
        HistoryPage, TelegramAdapter,
    },
};

pub type MessageLookupCalls = Arc<Mutex<Vec<(PeerId, Vec<MessageId>)>>>;

#[derive(Clone)]
pub struct FakeTelegramAdapter {
    peers: Arc<Mutex<Vec<PeerRecord>>>,
    messages: Arc<Mutex<HashMap<PeerId, Vec<MessageRecord>>>>,
    auxiliary_peers: Arc<Mutex<HashMap<PeerId, Vec<PeerRecord>>>>,
    channel_pts: Arc<Mutex<HashMap<PeerId, i32>>>,
    account_state: Arc<Mutex<AccountSyncState>>,
    common_diff_queue: Arc<Mutex<Vec<CommonDifferenceResult>>>,
    channel_diff_queues: Arc<Mutex<HashMap<PeerId, Vec<ChannelDifferenceResult>>>>,
    dialog_filters: Arc<Mutex<Vec<DialogFilterRecord>>>,
    dialog_pages: Arc<Mutex<HashMap<i32, Vec<DialogsPage>>>>,
    peer_dialogs_map: Arc<Mutex<HashMap<PeerId, DialogInfo>>>,
    flood_wait_on_channel: Arc<Mutex<Option<(PeerId, u32)>>>,
    injected_error: Arc<Mutex<Option<String>>>,
    channel_errors: Arc<Mutex<HashMap<PeerId, String>>>,
    files: Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>>,
    file_hashes: Arc<Mutex<HashMap<Vec<u8>, Vec<FileRangeHash>>>>,
    download_errors: Arc<Mutex<Vec<String>>>,
    file_hash_errors: Arc<Mutex<Vec<String>>>,
    custom_emoji_documents: Arc<Mutex<HashMap<i64, Document>>>,
    pub channels_get_messages_calls: MessageLookupCalls,
    pub messages_get_messages_calls: MessageLookupCalls,
    pub authorized: Arc<AtomicBool>,
}

impl Default for FakeTelegramAdapter {
    fn default() -> Self {
        Self {
            peers: Arc::new(Mutex::new(Vec::new())),
            messages: Arc::new(Mutex::new(HashMap::new())),
            auxiliary_peers: Arc::new(Mutex::new(HashMap::new())),
            channel_pts: Arc::new(Mutex::new(HashMap::new())),
            account_state: Arc::new(Mutex::new(AccountSyncState {
                account_id: "default".to_string(),
                pts: 100,
                qts: 10,
                date: 1700000000,
                seq: 1,
                sync_uncertain: false,
                last_synced_at: 1700000000,
            })),
            common_diff_queue: Arc::new(Mutex::new(Vec::new())),
            channel_diff_queues: Arc::new(Mutex::new(HashMap::new())),
            dialog_filters: Arc::new(Mutex::new(Vec::new())),
            dialog_pages: Arc::new(Mutex::new(HashMap::new())),
            peer_dialogs_map: Arc::new(Mutex::new(HashMap::new())),
            flood_wait_on_channel: Arc::new(Mutex::new(None)),
            injected_error: Arc::new(Mutex::new(None)),
            channel_errors: Arc::new(Mutex::new(HashMap::new())),
            files: Arc::new(Mutex::new(HashMap::new())),
            file_hashes: Arc::new(Mutex::new(HashMap::new())),
            download_errors: Arc::new(Mutex::new(Vec::new())),
            file_hash_errors: Arc::new(Mutex::new(Vec::new())),
            custom_emoji_documents: Arc::new(Mutex::new(HashMap::new())),
            channels_get_messages_calls: Arc::new(Mutex::new(Vec::new())),
            messages_get_messages_calls: Arc::new(Mutex::new(Vec::new())),
            authorized: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl FakeTelegramAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_authorized(&self, auth: bool) {
        self.authorized.store(auth, Ordering::Relaxed);
    }

    pub fn add_peer(&self, peer: PeerRecord) {
        let mut peers = self.peers.lock().unwrap();
        peers.retain(|p| p.peer_id != peer.peer_id);
        peers.push(peer);
    }

    pub fn resolve_canonical_peer_type(
        &self,
        peer_id: PeerId,
        peer_type_hint: Option<PeerType>,
    ) -> AdapterResult<PeerType> {
        if let Some(pt) = peer_type_hint {
            return Ok(pt);
        }
        self.peers
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.peer_id == peer_id)
            .map(|p| p.peer_type)
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

    pub fn add_raw_peer(&self, peer: PeerRecord) {
        let mut peers = self.peers.lock().unwrap();
        peers.push(peer);
    }

    pub fn add_message(&self, message: MessageRecord) {
        let mut messages = self.messages.lock().unwrap();
        let peer_msgs = messages.entry(message.key.peer_id).or_default();
        peer_msgs.retain(|m| m.key.message_id != message.key.message_id);
        peer_msgs.push(message);
    }

    pub fn add_messages(&self, messages: impl IntoIterator<Item = MessageRecord>) {
        for msg in messages {
            self.add_message(msg);
        }
    }

    pub fn set_auxiliary_peers(&self, peer_id: PeerId, aux: Vec<PeerRecord>) {
        self.auxiliary_peers.lock().unwrap().insert(peer_id, aux);
    }

    pub fn set_channel_pts(&self, peer_id: PeerId, pts: i32) {
        self.channel_pts.lock().unwrap().insert(peer_id, pts);
    }

    pub fn set_account_state(&self, state: AccountSyncState) {
        *self.account_state.lock().unwrap() = state;
    }

    pub fn enqueue_common_difference(&self, result: CommonDifferenceResult) {
        self.common_diff_queue.lock().unwrap().push(result);
    }

    pub fn enqueue_channel_difference(&self, channel_id: PeerId, result: ChannelDifferenceResult) {
        self.channel_diff_queues
            .lock()
            .unwrap()
            .entry(channel_id)
            .or_default()
            .push(result);
    }

    pub fn set_dialog_filters(&self, filters: Vec<DialogFilterRecord>) {
        *self.dialog_filters.lock().unwrap() = filters;
    }

    pub fn set_dialog_pages(&self, folder_id: i32, pages: Vec<DialogsPage>) {
        self.dialog_pages.lock().unwrap().insert(folder_id, pages);
    }

    pub fn set_peer_dialog(&self, peer_id: PeerId, info: DialogInfo) {
        self.peer_dialogs_map.lock().unwrap().insert(peer_id, info);
    }

    pub fn inject_flood_wait(&self, channel_id: PeerId, seconds: u32) {
        *self.flood_wait_on_channel.lock().unwrap() = Some((channel_id, seconds));
    }

    pub fn inject_error(&self, error_desc: impl Into<String>) {
        *self.injected_error.lock().unwrap() = Some(error_desc.into());
    }

    pub fn clear_error(&self) {
        *self.injected_error.lock().unwrap() = None;
    }

    pub fn inject_channel_error(&self, peer_id: PeerId, error_desc: impl Into<String>) {
        self.channel_errors
            .lock()
            .unwrap()
            .insert(peer_id, error_desc.into());
    }

    pub fn clear_channel_error(&self, peer_id: PeerId) {
        self.channel_errors.lock().unwrap().remove(&peer_id);
    }

    fn check_injected_error(&self) -> Option<AdapterError> {
        let err = self.injected_error.lock().unwrap().clone()?;
        if let Some(secs_str) = err.strip_prefix("FLOOD_WAIT_")
            && let Ok(secs) = secs_str.parse::<u32>()
        {
            return Some(AdapterError::FloodWait { seconds: secs });
        }
        if err == "UNAUTHORIZED" {
            return Some(AdapterError::NotAuthenticated);
        }
        Some(AdapterError::Invocation(err))
    }

    pub fn add_file(&self, location_tl: Vec<u8>, bytes: Vec<u8>) {
        self.files.lock().unwrap().insert(location_tl, bytes);
    }

    pub fn add_file_hashes(&self, location_tl: Vec<u8>, hashes: Vec<FileRangeHash>) {
        self.file_hashes.lock().unwrap().insert(location_tl, hashes);
    }

    pub fn inject_download_error(&self, err_name: impl Into<String>) {
        self.download_errors.lock().unwrap().push(err_name.into());
    }

    pub fn inject_file_hash_error(&self, err_name: impl Into<String>) {
        self.file_hash_errors.lock().unwrap().push(err_name.into());
    }

    pub fn set_custom_emoji_document(&self, doc: Document) {
        if let Document::Document(ref d) = doc {
            self.custom_emoji_documents
                .lock()
                .unwrap()
                .insert(d.id, doc);
        }
    }
}

#[async_trait]
impl TelegramAdapter for FakeTelegramAdapter {
    async fn get_dialogs(&self) -> AdapterResult<Vec<PeerRecord>> {
        if let Some(err) = self.check_injected_error() {
            return Err(err);
        }
        let peers = self.peers.lock().unwrap().clone();
        let mut seen = HashSet::new();
        let mut deduped = Vec::new();
        for p in peers {
            if seen.insert(p.peer_id) {
                deduped.push(p);
            }
        }
        Ok(deduped)
    }

    async fn get_state(&self) -> AdapterResult<AccountSyncState> {
        if let Some(err) = self.check_injected_error() {
            return Err(err);
        }
        Ok(self.account_state.lock().unwrap().clone())
    }

    async fn get_difference(
        &self,
        pts: i32,
        _date: i32,
        _qts: i32,
    ) -> AdapterResult<CommonDifferenceResult> {
        if let Some(err) = self.check_injected_error() {
            return Err(err);
        }

        let mut queue = self.common_diff_queue.lock().unwrap();
        if !queue.is_empty() {
            return Ok(queue.remove(0));
        }

        let state = self.account_state.lock().unwrap().clone();
        if state.pts == pts {
            Ok(CommonDifferenceResult::Empty {
                date: state.date,
                seq: state.seq,
            })
        } else {
            Ok(CommonDifferenceResult::Difference {
                new_messages: Vec::new(),
                other_updates: Vec::new(),
                auxiliary_peers: Vec::new(),
                state,
            })
        }
    }

    async fn get_channel_difference(
        &self,
        channel_id: PeerId,
        pts: i32,
        _limit: usize,
    ) -> AdapterResult<ChannelDifferenceResult> {
        if let Some(err) = self.check_injected_error() {
            return Err(err);
        }

        if let Some(err) = self.channel_errors.lock().unwrap().get(&channel_id) {
            return Err(AdapterError::Invocation(err.clone()));
        }

        if let Some((target_chan, seconds)) = self.flood_wait_on_channel.lock().unwrap().take()
            && target_chan == channel_id
        {
            return Err(AdapterError::FloodWait { seconds });
        }

        let mut queues = self.channel_diff_queues.lock().unwrap();
        if let Some(queue) = queues.get_mut(&channel_id)
            && !queue.is_empty()
        {
            return Ok(queue.remove(0));
        }

        let chan_pts = self
            .channel_pts
            .lock()
            .unwrap()
            .get(&channel_id)
            .copied()
            .unwrap_or(pts);
        Ok(ChannelDifferenceResult::Empty {
            final_state: true,
            pts: chan_pts,
            timeout: Some(30),
        })
    }

    async fn get_dialog_filters(&self) -> AdapterResult<Vec<DialogFilterRecord>> {
        if let Some(err) = self.check_injected_error() {
            return Err(err);
        }
        Ok(self.dialog_filters.lock().unwrap().clone())
    }

    async fn get_dialogs_paginated(
        &self,
        folder_id: i32,
        _offset_date: i32,
        _offset_id: i32,
        _offset_peer: Option<PeerId>,
        _limit: usize,
    ) -> AdapterResult<DialogsPage> {
        if let Some(err) = self.check_injected_error() {
            return Err(err);
        }

        let mut pages_map = self.dialog_pages.lock().unwrap();
        if let Some(pages) = pages_map.get_mut(&folder_id)
            && !pages.is_empty()
        {
            return Ok(pages.remove(0));
        }

        let peers = self.peers.lock().unwrap().clone();
        let dialogs = peers
            .iter()
            .map(|p| DialogInfo {
                peer_id: p.peer_id,
                peer_type: Some(p.peer_type),
                pts: self.channel_pts.lock().unwrap().get(&p.peer_id).copied(),
                top_message: None,
                unread_count: 0,
                is_pinned: false,
                folder_id: Some(folder_id),
                is_unresolved: false,
            })
            .collect();

        Ok(DialogsPage {
            dialogs,
            auxiliary_peers: Vec::new(),
            is_last_page: true,
            next_offset_date: 0,
            next_offset_id: 0,
            next_offset_peer: None,
        })
    }

    async fn get_peer_dialogs(&self, peers: &[PeerId]) -> AdapterResult<Vec<DialogInfo>> {
        if let Some(err) = self.check_injected_error() {
            return Err(err);
        }

        let map = self.peer_dialogs_map.lock().unwrap();
        let mut results = Vec::new();
        for &peer_id in peers {
            if let Some(info) = map.get(&peer_id) {
                results.push(info.clone());
            } else {
                let pts = self.channel_pts.lock().unwrap().get(&peer_id).copied();
                let peer_type = self
                    .peers
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|p| p.peer_id == peer_id)
                    .map(|p| p.peer_type);
                results.push(DialogInfo {
                    peer_id,
                    peer_type,
                    pts,
                    top_message: None,
                    unread_count: 0,
                    is_pinned: false,
                    folder_id: None,
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
        if let Some(err) = self.check_injected_error() {
            return Err(err);
        }

        let mut all_dialogs = Vec::new();
        let mut seen_peers = HashSet::new();

        let mut offset_date = 0;
        let mut offset_id = 0;
        let mut offset_peer = None;
        loop {
            let page = self
                .get_dialogs_paginated(0, offset_date, offset_id, offset_peer, 100)
                .await?;
            let is_last = page.is_last_page;
            offset_date = page.next_offset_date;
            offset_id = page.next_offset_id;
            offset_peer = page.next_offset_peer;
            for d in page.dialogs {
                if seen_peers.insert(d.peer_id) {
                    all_dialogs.push(d);
                }
            }
            if is_last {
                break;
            }
        }

        let mut is_complete = true;

        let mut offset_date = 0;
        let mut offset_id = 0;
        let mut offset_peer = None;
        loop {
            match self
                .get_dialogs_paginated(1, offset_date, offset_id, offset_peer, 100)
                .await
            {
                Ok(page) => {
                    let is_last = page.is_last_page;
                    offset_date = page.next_offset_date;
                    offset_id = page.next_offset_id;
                    offset_peer = page.next_offset_peer;
                    for d in page.dialogs {
                        if seen_peers.insert(d.peer_id) {
                            all_dialogs.push(d);
                        }
                    }
                    if is_last {
                        break;
                    }
                }
                Err(_) => {
                    if self.dialog_pages.lock().unwrap().contains_key(&1) {
                        is_complete = false;
                    }
                    break;
                }
            }
        }

        let custom_filters = match self.get_dialog_filters().await {
            Ok(f) => f,
            Err(_) => {
                is_complete = false;
                Vec::new()
            }
        };

        for filter in &custom_filters {
            let excluded: HashSet<PeerId> = filter.exclude_peers.iter().copied().collect();
            for &pid in filter
                .pinned_peers
                .iter()
                .chain(filter.include_peers.iter())
            {
                if !excluded.contains(&pid) && seen_peers.insert(pid) {
                    let peer_type = self
                        .peers
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|p| p.peer_id == pid)
                        .map(|p| p.peer_type)
                        .or(Some(PeerType::Channel));
                    all_dialogs.push(DialogInfo {
                        peer_id: pid,
                        peer_type,
                        pts: self.channel_pts.lock().unwrap().get(&pid).copied(),
                        top_message: None,
                        unread_count: 0,
                        is_pinned: filter.pinned_peers.contains(&pid),
                        folder_id: Some(filter.id),
                        is_unresolved: false,
                    });
                }
            }
        }

        let missing: Vec<PeerId> = local_channels
            .iter()
            .copied()
            .filter(|p| !seen_peers.contains(p))
            .collect();
        if !missing.is_empty() {
            match self.get_peer_dialogs(&missing).await {
                Ok(dormant_dialogs) => {
                    for d in dormant_dialogs {
                        if seen_peers.insert(d.peer_id) {
                            all_dialogs.push(d);
                        }
                    }
                }
                Err(_) => {
                    is_complete = false;
                    for pid in missing {
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
            auxiliary_peers: Vec::new(),
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
        if let Some(err) = self.check_injected_error() {
            return Err(err);
        }

        let messages_map = self.messages.lock().unwrap();
        let all_msgs = messages_map.get(&peer_id).cloned().unwrap_or_default();

        let mut sorted = all_msgs;
        sorted.sort_by_key(|m| std::cmp::Reverse(m.key.message_id));

        if let Some(offset) = offset_id {
            sorted.retain(|m| m.key.message_id < offset);
        }

        if let Some(min) = min_id {
            sorted.retain(|m| m.key.message_id > min);
        }

        if let Some(max) = max_id {
            sorted.retain(|m| m.key.message_id < max);
        }

        let total_matching = sorted.len();
        let paged: Vec<MessageRecord> = sorted.into_iter().take(limit).collect();
        let aux = self
            .auxiliary_peers
            .lock()
            .unwrap()
            .get(&peer_id)
            .cloned()
            .unwrap_or_default();
        let pts = self.channel_pts.lock().unwrap().get(&peer_id).copied();

        Ok(HistoryPage {
            messages: paged,
            auxiliary_peers: aux,
            pts,
            count: Some(total_matching as i32),
            raw_topics: Vec::new(),
        })
    }

    async fn get_messages(
        &self,
        peer_id: PeerId,
        peer_type: Option<PeerType>,
        message_ids: &[MessageId],
    ) -> AdapterResult<Vec<MessageRecord>> {
        if let Some(err) = self.check_injected_error() {
            return Err(err);
        }

        let effective_peer_type = self.resolve_canonical_peer_type(peer_id, peer_type)?;
        let is_channel = effective_peer_type == PeerType::Channel
            || (effective_peer_type == PeerType::Group && peer_id.raw() <= -1_000_000_000_000);

        if is_channel {
            self.channels_get_messages_calls
                .lock()
                .unwrap()
                .push((peer_id, message_ids.to_vec()));
        } else {
            self.messages_get_messages_calls
                .lock()
                .unwrap()
                .push((peer_id, message_ids.to_vec()));
        }

        let id_set: HashSet<MessageId> = message_ids.iter().copied().collect();
        let messages_map = self.messages.lock().unwrap();
        let peer_msgs = messages_map.get(&peer_id).cloned().unwrap_or_default();

        let found: Vec<MessageRecord> = peer_msgs
            .into_iter()
            .filter(|m| id_set.contains(&m.key.message_id))
            .collect();

        Ok(found)
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
        _dc_id: i32,
        offset: i64,
        limit: i32,
    ) -> AdapterResult<Vec<u8>> {
        let injected = {
            let mut errs = self.download_errors.lock().unwrap();
            if !errs.is_empty() {
                Some(errs.remove(0))
            } else {
                None
            }
        };

        if let Some(err_name) = injected {
            return Err(match err_name.as_str() {
                "FILE_REFERENCE_EXPIRED" => AdapterError::FileReferenceExpired,
                "FILE_REFERENCE_INVALID" => AdapterError::FileReferenceInvalid,
                "CDN_REDIRECT" => AdapterError::CdnRedirectUnsupported {
                    dc_id: 2,
                    file_token: vec![1, 2, 3],
                },
                s if s.starts_with("FILE_MIGRATE_") => {
                    let dc: i32 = s["FILE_MIGRATE_".len()..].parse().unwrap_or(2);
                    AdapterError::FileMigrate(dc)
                }
                s if s.starts_with("FLOOD_PREMIUM_WAIT_") => {
                    let sec: u32 = s["FLOOD_PREMIUM_WAIT_".len()..].parse().unwrap_or(5);
                    AdapterError::FloodPremiumWait { seconds: sec }
                }
                s if s.starts_with("FLOOD_WAIT_") => {
                    let sec: u32 = s["FLOOD_WAIT_".len()..].parse().unwrap_or(5);
                    AdapterError::FloodWait { seconds: sec }
                }
                _ => AdapterError::Invocation(err_name),
            });
        }

        let files = self.files.lock().unwrap();
        let bytes = files
            .get(location_tl)
            .ok_or_else(|| AdapterError::NotFound("File not found in fake adapter".to_string()))?;

        let start = (offset as usize).min(bytes.len());
        let end = ((offset + limit as i64) as usize).min(bytes.len());

        Ok(bytes[start..end].to_vec())
    }

    async fn get_file_hashes(
        &self,
        location_tl: &[u8],
        _dc_id: i32,
        offset: i64,
    ) -> AdapterResult<Vec<FileRangeHash>> {
        let injected = {
            let mut errs = self.file_hash_errors.lock().unwrap();
            if !errs.is_empty() {
                Some(errs.remove(0))
            } else {
                None
            }
        };

        if let Some(err_name) = injected {
            return Err(match err_name.as_str() {
                "FILE_REFERENCE_EXPIRED" => AdapterError::FileReferenceExpired,
                "FILE_REFERENCE_INVALID" => AdapterError::FileReferenceInvalid,
                s if s.starts_with("FLOOD_WAIT_") => {
                    let sec: u32 = s["FLOOD_WAIT_".len()..].parse().unwrap_or(5);
                    AdapterError::FloodWait { seconds: sec }
                }
                s if s.starts_with("FLOOD_PREMIUM_WAIT_") => {
                    let sec: u32 = s["FLOOD_PREMIUM_WAIT_".len()..].parse().unwrap_or(5);
                    AdapterError::FloodPremiumWait { seconds: sec }
                }
                _ => AdapterError::Invocation(err_name),
            });
        }

        let hashes = self.file_hashes.lock().unwrap();
        if let Some(list) = hashes.get(location_tl) {
            let matched: Vec<FileRangeHash> = list
                .iter()
                .filter(|h| h.offset >= offset)
                .cloned()
                .collect();
            Ok(matched)
        } else {
            Ok(Vec::new())
        }
    }

    async fn get_custom_emoji_documents(
        &self,
        document_ids: &[i64],
    ) -> AdapterResult<Vec<Document>> {
        if let Some(err) = self.check_injected_error() {
            return Err(err);
        }
        let docs = self.custom_emoji_documents.lock().unwrap();
        let out = document_ids
            .iter()
            .filter_map(|id| docs.get(id).cloned())
            .collect();
        Ok(out)
    }

    async fn is_authorized(&self) -> AdapterResult<bool> {
        Ok(self.authorized.load(Ordering::Relaxed))
    }
}
