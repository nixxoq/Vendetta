use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{sync::Semaphore, time::sleep};
use tracing::{debug, error, info, warn};
use vendetta_core::now_unix_secs;
use vendetta_model::MediaDownloadStatus;
use vendetta_storage::ArchiveDb;
use vendetta_tg_adapter::TelegramAdapter;

use crate::{
    downloader::SingleMediaDownloader, error::RetryAction, refresher::FileReferenceRefresher,
    storage_layout::StorageLayoutManager,
};

#[derive(Debug)]
pub struct DynamicConcurrencyController {
    min_concurrency: usize,
    max_concurrency: usize,
    max_dc_concurrency: usize,
    current_concurrency: AtomicUsize,
    success_counter: AtomicUsize,
    increase_threshold: usize,
    dc_cooldowns: Mutex<HashMap<i32, i64>>,
    dc_semaphores: Mutex<HashMap<i32, Arc<Semaphore>>>,
}

impl DynamicConcurrencyController {
    pub fn new(
        min_concurrency: usize,
        max_concurrency: usize,
        max_dc_concurrency: usize,
        initial: usize,
    ) -> Self {
        Self {
            min_concurrency: min_concurrency.max(1),
            max_concurrency: max_concurrency.max(min_concurrency),
            max_dc_concurrency: max_dc_concurrency.max(1),
            current_concurrency: AtomicUsize::new(initial.clamp(min_concurrency, max_concurrency)),
            success_counter: AtomicUsize::new(0),
            increase_threshold: 5,
            dc_cooldowns: Mutex::new(HashMap::new()),
            dc_semaphores: Mutex::new(HashMap::new()),
        }
    }

    pub fn current_concurrency(&self) -> usize {
        self.current_concurrency.load(Ordering::Relaxed)
    }

    pub fn max_dc_concurrency(&self) -> usize {
        self.max_dc_concurrency
    }

    pub fn get_dc_semaphore(&self, dc_id: i32) -> Arc<Semaphore> {
        let mut guard = self.dc_semaphores.lock().unwrap();
        guard
            .entry(dc_id)
            .or_insert_with(|| Arc::new(Semaphore::new(self.max_dc_concurrency)))
            .clone()
    }

    pub fn record_success(&self) {
        let count = self.success_counter.fetch_add(1, Ordering::Relaxed) + 1;
        if count >= self.increase_threshold {
            self.success_counter.store(0, Ordering::Relaxed);
            let current = self.current_concurrency.load(Ordering::Relaxed);
            if current < self.max_concurrency {
                self.current_concurrency
                    .store(current + 1, Ordering::Relaxed);
                debug!(
                    "AIMD: Increasing concurrency to {} after {} successes",
                    current + 1,
                    self.increase_threshold
                );
            }
        }
    }

    pub fn record_backoff(&self) {
        self.success_counter.store(0, Ordering::Relaxed);
        let current = self.current_concurrency.load(Ordering::Relaxed);
        let new_val = (current / 2).max(self.min_concurrency);
        self.current_concurrency.store(new_val, Ordering::Relaxed);
        debug!(
            "AIMD: Decreasing concurrency from {} to {}",
            current, new_val
        );
    }

    pub fn set_dc_cooldown(&self, dc_id: i32, duration_secs: u32) {
        let now = now_unix_secs();
        let until = now + duration_secs as i64;
        self.dc_cooldowns.lock().unwrap().insert(dc_id, until);
        info!(
            "DC {} entered cooldown for {}s (until {})",
            dc_id, duration_secs, until
        );
    }

    pub fn is_dc_in_cooldown(&self, dc_id: i32) -> bool {
        let now = now_unix_secs();
        let guard = self.dc_cooldowns.lock().unwrap();
        guard.get(&dc_id).is_some_and(|&until| now < until)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchedulerSummary {
    pub completed_count: usize,
    pub retry_wait_count: usize,
    pub permanently_failed_count: usize,
    pub needs_reauth_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct DownloadProgressEvent {
    pub completed_count: usize,
    pub retry_wait_count: usize,
    pub needs_reauth_count: usize,
    pub permanently_failed_count: usize,
    pub downloaded_bytes: u64,
}

pub struct MediaScheduler {
    db: Arc<ArchiveDb>,
    adapter: Arc<dyn TelegramAdapter>,
    layout: StorageLayoutManager,
    controller: Arc<DynamicConcurrencyController>,
}

impl MediaScheduler {
    pub fn new(
        db: Arc<ArchiveDb>,
        adapter: Arc<dyn TelegramAdapter>,
        layout: StorageLayoutManager,
        min_workers: usize,
        max_workers: usize,
        max_dc_workers: usize,
        initial_workers: usize,
    ) -> Self {
        let controller = Arc::new(DynamicConcurrencyController::new(
            min_workers,
            max_workers,
            max_dc_workers,
            initial_workers,
        ));
        Self {
            db,
            adapter,
            layout,
            controller,
        }
    }

    pub fn controller(&self) -> &Arc<DynamicConcurrencyController> {
        &self.controller
    }

    pub async fn run_until_idle(&self, worker_id_prefix: &str) -> usize {
        let summary = self.run_batch(worker_id_prefix).await;
        summary.completed_count
    }

    pub async fn run_batch(&self, worker_id_prefix: &str) -> SchedulerSummary {
        self.run_batch_with_progress(worker_id_prefix, |_| {}).await
    }

    pub async fn run_batch_with_progress<F>(
        &self,
        worker_id_prefix: &str,
        progress: F,
    ) -> SchedulerSummary
    where
        F: Fn(&DownloadProgressEvent) + Send + Sync + 'static,
    {
        let mut summary = SchedulerSummary::default();
        let mut progress_event = DownloadProgressEvent::default();

        let downloader = Arc::new(SingleMediaDownloader::new(
            Arc::clone(&self.db),
            Arc::clone(&self.adapter),
            self.layout.clone(),
        ));
        let refresher = Arc::new(FileReferenceRefresher::new(
            Arc::clone(&self.db),
            Arc::clone(&self.adapter),
        ));

        loop {
            let concurrency = self.controller.current_concurrency();
            let now = now_unix_secs();

            let mut claimed = Vec::new();
            for i in 0..concurrency {
                let worker_id = format!("{}_{}", worker_id_prefix, i);
                match self.db.claim_next_pending_media(&worker_id) {
                    Ok(Some(item)) => claimed.push(item),
                    Ok(None) => break,
                    Err(e) => {
                        error!("Failed to claim pending media from DB: {e}");
                        break;
                    }
                }
            }

            if claimed.is_empty() {
                debug!("No eligible media items to download. Scheduler cycle complete.");
                break;
            }

            let mut tasks = Vec::new();

            for mut record in claimed {
                if self.controller.is_dc_in_cooldown(record.dc_id) {
                    debug!(
                        "DC {} is in cooldown; setting RetryWait for {}",
                        record.dc_id, record.media_id
                    );
                    let _ = self.db.update_media_status(
                        &record.media_id,
                        MediaDownloadStatus::RetryWait,
                        Some("DC in cooldown"),
                        Some(now + 5),
                    );
                    summary.retry_wait_count += 1;
                    continue;
                }

                let dc_sem = self.controller.get_dc_semaphore(record.dc_id);
                let dl = Arc::clone(&downloader);
                let rf = Arc::clone(&refresher);
                let ctrl = Arc::clone(&self.controller);
                let db_ref = Arc::clone(&self.db);
                let worker_id_str = record
                    .worker_id
                    .clone()
                    .unwrap_or_else(|| format!("{worker_id_prefix}_worker"));

                tasks.push(tokio::spawn(async move {
                    let _dc_permit = dc_sem.acquire().await.unwrap();
                    let mut attempts = 0;
                    const MAX_IMMEDIATE_ATTEMPTS: usize = 3;

                    loop {
                        attempts += 1;
                        let size_hint = record.size_bytes.unwrap_or(0).max(0) as u64;
                        match dl.download_item(&mut record).await {
                            Ok(_) => {
                                ctrl.record_success();
                                return (true, None, size_hint);
                            }
                            Err(err) => {
                                let action = err.classify_retry_action();
                                warn!(
                                    "Download error for {} (attempt {}): {} -> {:?}",
                                    record.media_id, attempts, err, action
                                );

                                match action {
                                    RetryAction::RefreshAndRetry => {
                                        if attempts <= MAX_IMMEDIATE_ATTEMPTS
                                            && rf
                                                .refresh_file_reference_while_claimed(
                                                    &mut record,
                                                    &worker_id_str,
                                                )
                                                .await
                                                .is_ok()
                                        {
                                            continue;
                                        }
                                        let _ = db_ref.update_media_status(
                                            &record.media_id,
                                            MediaDownloadStatus::RetryWait,
                                            Some(&err.to_string()),
                                            Some(now_unix_secs() + 5),
                                        );
                                        return (false, Some(MediaDownloadStatus::RetryWait), 0);
                                    }
                                    RetryAction::MigrateAndRetry { new_dc } => {
                                        let _ = db_ref.update_media_dc_while_claimed(
                                            &record.media_id,
                                            new_dc,
                                            &worker_id_str,
                                        );
                                        record.dc_id = new_dc;
                                        if attempts <= MAX_IMMEDIATE_ATTEMPTS {
                                            continue;
                                        }
                                        let _ = db_ref.update_media_status(
                                            &record.media_id,
                                            MediaDownloadStatus::RetryWait,
                                            Some(&err.to_string()),
                                            Some(now_unix_secs() + 2),
                                        );
                                        return (false, Some(MediaDownloadStatus::RetryWait), 0);
                                    }
                                    RetryAction::RetryAfterDelay { seconds } => {
                                        ctrl.record_backoff();
                                        ctrl.set_dc_cooldown(record.dc_id, seconds);
                                        let _ = db_ref.update_media_status(
                                            &record.media_id,
                                            MediaDownloadStatus::RetryWait,
                                            Some(&err.to_string()),
                                            Some(now_unix_secs() + seconds as i64),
                                        );
                                        return (false, Some(MediaDownloadStatus::RetryWait), 0);
                                    }
                                    RetryAction::RetryImmediately => {
                                        if attempts <= MAX_IMMEDIATE_ATTEMPTS {
                                            sleep(Duration::from_millis(100)).await;
                                            continue;
                                        }
                                        let _ = db_ref.update_media_status(
                                            &record.media_id,
                                            MediaDownloadStatus::RetryWait,
                                            Some(&err.to_string()),
                                            Some(now_unix_secs() + 3),
                                        );
                                        return (false, Some(MediaDownloadStatus::RetryWait), 0);
                                    }
                                    RetryAction::PauseForAuth => {
                                        let _ = db_ref.update_media_status(
                                            &record.media_id,
                                            MediaDownloadStatus::NeedsReauth,
                                            Some(&err.to_string()),
                                            None,
                                        );
                                        return (false, Some(MediaDownloadStatus::NeedsReauth), 0);
                                    }
                                    RetryAction::PermanentFailure => {
                                        let _ = db_ref.update_media_status(
                                            &record.media_id,
                                            MediaDownloadStatus::PermanentlyFailed,
                                            Some(&err.to_string()),
                                            None,
                                        );
                                        return (
                                            false,
                                            Some(MediaDownloadStatus::PermanentlyFailed),
                                            0,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }));
            }

            for t in tasks {
                if let Ok((success, status, bytes)) = t.await {
                    if success {
                        summary.completed_count += 1;
                        progress_event.completed_count += 1;
                        progress_event.downloaded_bytes += bytes;
                    } else if let Some(st) = status {
                        match st {
                            MediaDownloadStatus::RetryWait => {
                                summary.retry_wait_count += 1;
                                progress_event.retry_wait_count += 1;
                            }
                            MediaDownloadStatus::PermanentlyFailed => {
                                summary.permanently_failed_count += 1;
                                progress_event.permanently_failed_count += 1;
                            }
                            MediaDownloadStatus::NeedsReauth => {
                                summary.needs_reauth_count += 1;
                                progress_event.needs_reauth_count += 1;
                            }
                            _ => {}
                        }
                    }
                    progress(&progress_event);
                }
            }
        }

        summary
    }
}
