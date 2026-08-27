use std::sync::Arc;

use grammers_tl_types::{self as tl, Deserializable};
use tracing::{debug, warn};
use vendetta_model::MediaRecord;
use vendetta_storage::ArchiveDb;
use vendetta_tg_adapter::{TelegramAdapter, extract_media_records};

use crate::error::{MediaEngineError, MediaEngineResult};

pub struct FileReferenceRefresher {
    db: Arc<ArchiveDb>,
    adapter: Arc<dyn TelegramAdapter>,
}

impl FileReferenceRefresher {
    pub fn new(db: Arc<ArchiveDb>, adapter: Arc<dyn TelegramAdapter>) -> Self {
        Self { db, adapter }
    }

    pub async fn refresh_file_reference(&self, record: &mut MediaRecord) -> MediaEngineResult<()> {
        self.refresh_file_reference_internal(record, None).await
    }

    pub async fn refresh_file_reference_while_claimed(
        &self,
        record: &mut MediaRecord,
        worker_id: &str,
    ) -> MediaEngineResult<()> {
        self.refresh_file_reference_internal(record, Some(worker_id))
            .await
    }

    async fn refresh_file_reference_internal(
        &self,
        record: &mut MediaRecord,
        worker_id: Option<&str>,
    ) -> MediaEngineResult<()> {
        let refs = self
            .db
            .get_referencing_messages_for_media(&record.media_id)?;

        if refs.is_empty() {
            return Err(MediaEngineError::FileReferenceExpired(format!(
                "No referencing messages found in database for media {}",
                record.media_id
            )));
        }

        debug!(
            "Attempting to refresh file reference for {} across {} referencing message(s)",
            record.media_id,
            refs.len()
        );

        for (peer_id, message_id) in refs {
            let fetched_msgs = match self
                .adapter
                .get_messages(peer_id, None, &[message_id])
                .await
            {
                Ok(m) => m,
                Err(e) => {
                    warn!(
                        "Failed to fetch referencing message ({}, {}) from Telegram: {}",
                        peer_id.raw(),
                        message_id.raw(),
                        e
                    );
                    continue;
                }
            };

            let Some(msg_record) = fetched_msgs.into_iter().next() else {
                continue;
            };
            let Some(raw_tl) = msg_record.raw_tl.as_deref() else {
                continue;
            };
            let Ok(tl_msg) = tl::enums::Message::from_bytes(raw_tl) else {
                continue;
            };

            for (new_rec, _) in extract_media_records(&tl_msg, Some(peer_id)) {
                if new_rec.media_id == record.media_id
                    && let Some(new_ref) = new_rec.file_reference
                {
                    let new_loc = new_rec.source_location_tl;
                    if let Some(wid) = worker_id {
                        self.db.update_media_file_reference_while_claimed(
                            &record.media_id,
                            &new_ref,
                            new_loc.as_deref(),
                            wid,
                        )?;
                    } else {
                        self.db.update_media_file_reference(
                            &record.media_id,
                            &new_ref,
                            new_loc.as_deref(),
                        )?;
                    }

                    record.file_reference = Some(new_ref);
                    if let Some(loc) = new_loc {
                        record.source_location_tl = Some(loc);
                    }

                    debug!(
                        "Successfully refreshed file reference for {} via message ({}, {})",
                        record.media_id,
                        peer_id.raw(),
                        message_id.raw()
                    );
                    return Ok(());
                }
            }
        }

        Err(MediaEngineError::FileReferenceExpired(format!(
            "Failed to refresh file reference for {} after trying all referencing messages",
            record.media_id
        )))
    }
}
