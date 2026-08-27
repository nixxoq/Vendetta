use async_trait::async_trait;
use grammers_tl_types::enums::Document;
use vendetta_model::{
    AccountSyncState, DialogFilterRecord, DialogInfo, FileRangeHash, MessageId, MessageRecord,
    NormalizedUpdate, PeerId, PeerRecord, PeerType,
};

use crate::error::{AdapterError, AdapterResult};

#[derive(Debug, Clone, Default)]
pub struct HistoryPage {
    pub messages: Vec<MessageRecord>,
    pub auxiliary_peers: Vec<PeerRecord>,
    pub pts: Option<i32>,
    pub count: Option<i32>,
    pub raw_topics: Vec<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub enum CommonDifferenceResult {
    Empty {
        date: i32,
        seq: i32,
    },
    Slice {
        new_messages: Vec<MessageRecord>,
        other_updates: Vec<NormalizedUpdate>,
        auxiliary_peers: Vec<PeerRecord>,
        intermediate_state: AccountSyncState,
    },
    Difference {
        new_messages: Vec<MessageRecord>,
        other_updates: Vec<NormalizedUpdate>,
        auxiliary_peers: Vec<PeerRecord>,
        state: AccountSyncState,
    },
    TooLong {
        pts: i32,
    },
}

#[derive(Debug, Clone)]
pub enum ChannelDifferenceResult {
    Empty {
        final_state: bool,
        pts: i32,
        timeout: Option<i32>,
    },
    Difference {
        final_state: bool,
        pts: i32,
        timeout: Option<i32>,
        new_messages: Vec<MessageRecord>,
        other_updates: Vec<NormalizedUpdate>,
        auxiliary_peers: Vec<PeerRecord>,
    },
    TooLong {
        final_state: bool,
        timeout: Option<i32>,
        dialog_pts: i32,
        top_message: Option<MessageId>,
        messages: Vec<MessageRecord>,
        auxiliary_peers: Vec<PeerRecord>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct DialogsPage {
    pub dialogs: Vec<DialogInfo>,
    pub auxiliary_peers: Vec<PeerRecord>,
    pub is_last_page: bool,
    pub next_offset_date: i32,
    pub next_offset_id: i32,
    pub next_offset_peer: Option<PeerId>,
}

#[derive(Debug, Clone, Default)]
pub struct DialogDiscoveryResult {
    pub discovered_dialogs: Vec<DialogInfo>,
    pub auxiliary_peers: Vec<PeerRecord>,
    pub custom_filters: Vec<DialogFilterRecord>,
    pub is_complete: bool,
}

#[async_trait]
pub trait TelegramAdapter: Send + Sync {
    async fn get_dialogs(&self) -> AdapterResult<Vec<PeerRecord>>;

    async fn get_state(&self) -> AdapterResult<AccountSyncState> {
        Err(AdapterError::Invocation(
            "get_state not implemented".to_string(),
        ))
    }

    async fn get_difference(
        &self,
        _pts: i32,
        _date: i32,
        _qts: i32,
    ) -> AdapterResult<CommonDifferenceResult> {
        Err(AdapterError::Invocation(
            "get_difference not implemented".to_string(),
        ))
    }

    async fn get_channel_difference(
        &self,
        _channel_id: PeerId,
        _pts: i32,
        _limit: usize,
    ) -> AdapterResult<ChannelDifferenceResult> {
        Err(AdapterError::Invocation(
            "get_channel_difference not implemented".to_string(),
        ))
    }

    async fn get_dialog_filters(&self) -> AdapterResult<Vec<DialogFilterRecord>> {
        Ok(Vec::new())
    }

    async fn get_dialogs_paginated(
        &self,
        _folder_id: i32,
        _offset_date: i32,
        _offset_id: i32,
        _offset_peer: Option<PeerId>,
        _limit: usize,
    ) -> AdapterResult<DialogsPage> {
        Ok(DialogsPage::default())
    }

    async fn get_peer_dialogs(&self, _peers: &[PeerId]) -> AdapterResult<Vec<DialogInfo>> {
        Ok(Vec::new())
    }

    async fn get_all_dialogs_complete(
        &self,
        _local_channels: &[PeerId],
    ) -> AdapterResult<DialogDiscoveryResult> {
        Ok(DialogDiscoveryResult {
            discovered_dialogs: Vec::new(),
            auxiliary_peers: Vec::new(),
            custom_filters: Vec::new(),
            is_complete: true,
        })
    }

    async fn get_history_page(
        &self,
        peer_id: PeerId,
        limit: usize,
        offset_id: Option<MessageId>,
    ) -> AdapterResult<HistoryPage>;

    async fn get_history_page_filtered(
        &self,
        peer_id: PeerId,
        _min_id: Option<MessageId>,
        _max_id: Option<MessageId>,
        offset_id: Option<MessageId>,
        limit: usize,
    ) -> AdapterResult<HistoryPage> {
        self.get_history_page(peer_id, limit, offset_id).await
    }

    async fn get_history_bounded(
        &self,
        peer_id: PeerId,
        min_id: Option<MessageId>,
        max_id: Option<MessageId>,
        offset_id: Option<MessageId>,
        limit: usize,
    ) -> AdapterResult<Vec<MessageRecord>> {
        let page = self
            .get_history_page_filtered(peer_id, min_id, max_id, offset_id, limit)
            .await?;
        Ok(page.messages)
    }

    async fn get_history(
        &self,
        peer_id: PeerId,
        limit: usize,
        offset_id: Option<MessageId>,
    ) -> AdapterResult<Vec<MessageRecord>> {
        let page = self.get_history_page(peer_id, limit, offset_id).await?;
        Ok(page.messages)
    }

    async fn get_messages(
        &self,
        _peer_id: PeerId,
        _peer_type: Option<PeerType>,
        _message_ids: &[MessageId],
    ) -> AdapterResult<Vec<MessageRecord>> {
        Ok(Vec::new())
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
        _location_tl: &[u8],
        _dc_id: i32,
        _offset: i64,
        _limit: i32,
    ) -> AdapterResult<Vec<u8>> {
        Err(AdapterError::Invocation(
            "download_file_chunk not implemented".to_string(),
        ))
    }

    async fn get_file_hashes(
        &self,
        _location_tl: &[u8],
        _dc_id: i32,
        _offset: i64,
    ) -> AdapterResult<Vec<FileRangeHash>> {
        Ok(Vec::new())
    }

    async fn get_custom_emoji_documents(
        &self,
        _document_ids: &[i64],
    ) -> AdapterResult<Vec<Document>> {
        Ok(Vec::new())
    }

    async fn is_authorized(&self) -> AdapterResult<bool> {
        Ok(true)
    }
}
