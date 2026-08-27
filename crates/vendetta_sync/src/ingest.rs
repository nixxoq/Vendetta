use tracing::debug;
use vendetta_core::now_unix_secs;
use vendetta_model::{MessageId, PeerId, SyncStateRecord};
use vendetta_storage::ArchiveDb;
use vendetta_tg_adapter::TelegramAdapter;

use crate::error::{SyncError, SyncResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestSummary {
    pub peer_id: PeerId,
    pub batches_committed: usize,
    pub messages_ingested: usize,
    pub auxiliary_peers_ingested: usize,
    pub min_message_id: Option<MessageId>,
    pub max_message_id: Option<MessageId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryBatchProgress {
    pub peer_id: PeerId,
    pub batch_number: usize,
    pub batch_messages_count: usize,
    pub current_peer_messages_count: usize,
    pub min_message_id: Option<MessageId>,
}

pub struct HistoryIngestionPipeline {
    chunk_size: usize,
}

impl Default for HistoryIngestionPipeline {
    fn default() -> Self {
        Self { chunk_size: 100 }
    }
}

impl HistoryIngestionPipeline {
    pub fn new(chunk_size: usize) -> Self {
        Self {
            chunk_size: chunk_size.max(1),
        }
    }

    pub async fn ingest_history<A: ?Sized + TelegramAdapter>(
        &self,
        adapter: &A,
        db: &ArchiveDb,
        peer_id: PeerId,
    ) -> SyncResult<IngestSummary> {
        self.ingest_history_with_progress(adapter, db, peer_id, |_| {})
            .await
    }

    pub async fn ingest_history_with_progress<A: ?Sized + TelegramAdapter, F>(
        &self,
        adapter: &A,
        db: &ArchiveDb,
        peer_id: PeerId,
        mut on_batch: F,
    ) -> SyncResult<IngestSummary>
    where
        F: FnMut(&HistoryBatchProgress),
    {
        let existing_state = db.get_sync_state(peer_id)?;

        let mut offset_id = existing_state
            .as_ref()
            .and_then(|s| s.min_message_id)
            .map(MessageId::new);

        let mut overall_min_id = existing_state
            .as_ref()
            .and_then(|s| s.min_message_id)
            .map(MessageId::new);

        let mut overall_max_id = existing_state
            .as_ref()
            .and_then(|s| s.max_message_id)
            .map(MessageId::new);

        let mut batches_committed = 0;
        let mut total_messages = 0;
        let mut total_aux_peers = 0;

        loop {
            let page = adapter
                .get_history_page(peer_id, self.chunk_size, offset_id)
                .await?;

            if page.messages.is_empty() {
                break;
            }

            let batch_len = page.messages.len();
            let batch_min_id = page.messages.iter().map(|m| m.key.message_id).min();
            let batch_max_id = page.messages.iter().map(|m| m.key.message_id).max();

            if let (Some(cur_offset), Some(new_min)) = (offset_id, batch_min_id)
                && new_min >= cur_offset
            {
                return Err(SyncError::NonProgressingPagination {
                    peer_id,
                    offset_id: Some(cur_offset),
                    returned_min_id: Some(new_min),
                });
            }

            if let Some(b_min) = batch_min_id {
                overall_min_id = Some(overall_min_id.map_or(b_min, |cur| cur.min(b_min)));
            }

            if let Some(b_max) = batch_max_id {
                overall_max_id = Some(overall_max_id.map_or(b_max, |cur| cur.max(b_max)));
            }

            let sync_state = SyncStateRecord {
                peer_id,
                pts: page
                    .pts
                    .or_else(|| existing_state.as_ref().and_then(|s| s.pts)),
                qts: existing_state.as_ref().and_then(|s| s.qts),
                date: existing_state.as_ref().and_then(|s| s.date),
                seq: existing_state.as_ref().and_then(|s| s.seq),
                min_message_id: overall_min_id.map(|id| id.raw()),
                max_message_id: overall_max_id.map(|id| id.raw()),
                last_synced_at: now_unix_secs(),
            };

            db.ingest_history_page(
                peer_id,
                &page.messages,
                &page.auxiliary_peers,
                Some(&sync_state),
            )?;

            batches_committed += 1;
            total_messages += batch_len;
            total_aux_peers += page.auxiliary_peers.len();

            debug!(
                "Committed batch {} for peer {} ({} messages, cursor: {:?})",
                batches_committed, peer_id, batch_len, batch_min_id
            );

            on_batch(&HistoryBatchProgress {
                peer_id,
                batch_number: batches_committed,
                batch_messages_count: batch_len,
                current_peer_messages_count: total_messages,
                min_message_id: batch_min_id,
            });

            offset_id = batch_min_id;

            if batch_len < self.chunk_size {
                break;
            }
        }

        Ok(IngestSummary {
            peer_id,
            batches_committed,
            messages_ingested: total_messages,
            auxiliary_peers_ingested: total_aux_peers,
            min_message_id: overall_min_id,
            max_message_id: overall_max_id,
        })
    }
}
