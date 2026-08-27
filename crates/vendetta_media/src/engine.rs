use std::{path::PathBuf, sync::Arc};

use vendetta_model::{
    FilterDecision, MediaDownloadStatus, MediaFilterPolicy, MediaQueueStats, MediaStats,
};
use vendetta_storage::ArchiveDb;
use vendetta_tg_adapter::TelegramAdapter;

use crate::{
    backfill::{BackfillResult, MediaBackfillPlanner},
    error::MediaEngineResult,
    policy::MediaPolicyEvaluator,
    reconciler::{ReconciliationReport, StartupReconciler},
    scheduler::{DownloadProgressEvent, MediaScheduler, SchedulerSummary},
    storage_layout::StorageLayoutManager,
    verifier::{MediaVerifier, VerificationReport},
};

pub struct MediaEngine {
    db: Arc<ArchiveDb>,
    adapter: Arc<dyn TelegramAdapter>,
    layout: StorageLayoutManager,
    scheduler: MediaScheduler,
}

impl MediaEngine {
    pub fn new(
        db: Arc<ArchiveDb>,
        adapter: Arc<dyn TelegramAdapter>,
        archive_root: impl Into<PathBuf>,
        min_workers: usize,
        max_workers: usize,
        max_dc_workers: usize,
        initial_workers: usize,
    ) -> Self {
        let layout = StorageLayoutManager::new(archive_root);
        let scheduler = MediaScheduler::new(
            Arc::clone(&db),
            Arc::clone(&adapter),
            layout.clone(),
            min_workers,
            max_workers,
            max_dc_workers,
            initial_workers,
        );

        Self {
            db,
            adapter,
            layout,
            scheduler,
        }
    }

    pub fn layout(&self) -> &StorageLayoutManager {
        &self.layout
    }

    pub fn db(&self) -> &Arc<ArchiveDb> {
        &self.db
    }

    pub fn adapter(&self) -> &Arc<dyn TelegramAdapter> {
        &self.adapter
    }

    pub fn scheduler(&self) -> &MediaScheduler {
        &self.scheduler
    }

    pub fn reconcile_startup(&self) -> MediaEngineResult<ReconciliationReport> {
        let reconciler = StartupReconciler::new(Arc::clone(&self.db), self.layout.clone());
        reconciler.reconcile()
    }

    pub fn plan_media_from_archive(
        &self,
        policy: &MediaFilterPolicy,
    ) -> MediaEngineResult<BackfillResult> {
        let planner = MediaBackfillPlanner::new(Arc::clone(&self.db));
        planner.plan_media_from_archive(policy)
    }

    pub async fn download_all_pending(&self, worker_id_prefix: &str) -> usize {
        self.scheduler.run_until_idle(worker_id_prefix).await
    }

    pub async fn download_batch(&self, worker_id_prefix: &str) -> SchedulerSummary {
        self.scheduler.run_batch(worker_id_prefix).await
    }

    pub async fn download_batch_with_progress<F>(
        &self,
        worker_id_prefix: &str,
        progress: F,
    ) -> SchedulerSummary
    where
        F: Fn(&DownloadProgressEvent) + Send + Sync + 'static,
    {
        self.scheduler
            .run_batch_with_progress(worker_id_prefix, progress)
            .await
    }

    pub fn verify_media(&self) -> MediaEngineResult<VerificationReport> {
        self.verify_media_with_progress(|_, _, _| {})
    }

    pub fn verify_media_with_progress<F>(
        &self,
        progress: F,
    ) -> MediaEngineResult<VerificationReport>
    where
        F: Fn(usize, usize, &VerificationReport),
    {
        let verifier = MediaVerifier::new(Arc::clone(&self.db), self.layout.clone());
        verifier.verify_all_completed_with_progress(progress)
    }

    pub fn get_stats(&self) -> MediaEngineResult<MediaStats> {
        Ok(self.db.get_media_stats()?)
    }

    pub fn get_queue_stats(&self) -> MediaEngineResult<MediaQueueStats> {
        Ok(self.db.get_queue_stats()?)
    }

    pub fn requeue_skipped(&self, policy: &MediaFilterPolicy) -> MediaEngineResult<usize> {
        let skipped_records = self.db.get_skipped_media()?;
        let mut newly_allowed = 0;

        for record in skipped_records {
            let (decision, reason) = MediaPolicyEvaluator::evaluate(policy, &record, None);
            let status = match decision {
                FilterDecision::Allow => {
                    newly_allowed += 1;
                    MediaDownloadStatus::Pending
                }
                FilterDecision::Skip => MediaDownloadStatus::Skipped,
            };

            self.db.update_media_filter_status(
                &record.media_id,
                status,
                decision,
                reason,
                policy.policy_version,
            )?;
        }

        Ok(newly_allowed)
    }
}
