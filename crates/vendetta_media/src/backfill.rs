use std::sync::Arc;

use grammers_tl_types::{self as tl, Deserializable};
use tracing::{debug, info, warn};
use vendetta_model::{FilterDecision, MediaDownloadStatus, MediaFilterPolicy, PeerId};
use vendetta_storage::ArchiveDb;
use vendetta_tg_adapter::extract_media_records;

use crate::{error::MediaEngineResult, policy::MediaPolicyEvaluator};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackfillResult {
    pub messages_scanned: usize,
    pub media_discovered: usize,
    pub media_eligible: usize,
    pub media_skipped: usize,
}

pub struct MediaBackfillPlanner {
    db: Arc<ArchiveDb>,
}

impl MediaBackfillPlanner {
    pub fn new(db: Arc<ArchiveDb>) -> Self {
        Self { db }
    }

    pub fn plan_media_from_archive(
        &self,
        policy: &MediaFilterPolicy,
    ) -> MediaEngineResult<BackfillResult> {
        let mut result = BackfillResult::default();

        let raw_messages = self.db.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT peer_id, raw_tl FROM messages WHERE raw_tl IS NOT NULL")?;
            let rows = stmt.query_map([], |row| {
                let pid: i64 = row.get(0)?;
                let tl: Vec<u8> = row.get(1)?;
                Ok((PeerId::new(pid), tl))
            })?;
            let list = rows.collect::<Result<Vec<_>, _>>()?;
            Ok(list)
        })?;

        result.messages_scanned = raw_messages.len();
        debug!(
            "Scanning {} archived messages for media",
            raw_messages.len()
        );

        for (peer_id, raw_tl) in raw_messages {
            let Ok(tl_msg) = tl::enums::Message::from_bytes(&raw_tl) else {
                warn!(
                    "Failed to deserialize raw TL message for peer {}",
                    peer_id.raw()
                );
                continue;
            };

            for (mut record, join) in extract_media_records(&tl_msg, Some(peer_id)) {
                result.media_discovered += 1;

                let (decision, reason) =
                    MediaPolicyEvaluator::evaluate(policy, &record, Some(peer_id));
                record.filter_decision = Some(decision);
                record.filter_reason = reason;
                record.policy_version = policy.policy_version;

                if decision == FilterDecision::Skip {
                    record.download_status = MediaDownloadStatus::Skipped;
                    result.media_skipped += 1;
                } else {
                    result.media_eligible += 1;
                }

                self.db.insert_or_update_media(&record)?;
                self.db.link_message_media(&join)?;
            }
        }

        info!(
            "Media backfill complete: {} messages scanned, {} media discovered ({} eligible, {} skipped)",
            result.messages_scanned,
            result.media_discovered,
            result.media_eligible,
            result.media_skipped
        );

        Ok(result)
    }
}
