use std::{
    io::{IsTerminal, Write},
    time::{Duration, Instant},
};

pub struct Throttle {
    interval: Duration,
    last: Instant,
}

impl Throttle {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: Instant::now() - interval,
        }
    }

    pub fn should_render(&mut self, now: Instant) -> bool {
        if now.duration_since(self.last) >= self.interval {
            self.last = now;
            true
        } else {
            false
        }
    }
}

pub struct CliProgress {
    quiet: bool,
    json: bool,
    start_time: Instant,
}

impl CliProgress {
    pub fn new(quiet: bool, json: bool) -> Self {
        Self {
            quiet,
            json,
            start_time: Instant::now(),
        }
    }

    pub fn is_quiet(&self) -> bool {
        self.quiet
    }

    pub fn stage(&mut self, name: &str) {
        if self.quiet || self.json {
            return;
        }
        let elapsed = self.start_time.elapsed().as_secs();
        eprintln!("[{:>4}s] ==> {}", elapsed, name);
        let _ = std::io::stderr().flush();
    }

    pub fn update(&mut self, msg: impl AsRef<str>) {
        if self.quiet || self.json {
            return;
        }
        let elapsed = self.start_time.elapsed().as_secs();
        eprintln!("        [{:>4}s] {}", elapsed, msg.as_ref());
        let _ = std::io::stderr().flush();
    }

    pub fn finish(&self, summary: &str) {
        if self.quiet || self.json {
            return;
        }
        let elapsed = self.start_time.elapsed();
        eprintln!(
            "[INFO] Finished in {}.{:02}s: {}",
            elapsed.as_secs(),
            elapsed.subsec_millis() / 10,
            summary
        );
        let _ = std::io::stderr().flush();
    }
}

fn render_progress_line(line: &str, is_tty: bool) {
    if is_tty {
        eprint!("\r\x1b[2K        {}", line);
    } else {
        eprintln!("        {}", line);
    }
    let _ = std::io::stderr().flush();
}

fn clear_progress_line(is_tty: bool) {
    if is_tty {
        eprint!("\r\x1b[2K");
        let _ = std::io::stderr().flush();
    }
}

pub struct MediaDownloadProgressTracker {
    quiet: bool,
    json: bool,
    is_tty: bool,
    total_eligible: usize,
    total_bytes_known: Option<u64>,
    start_time: Instant,
    throttle: Throttle,
    last_reported_count: usize,
}

impl MediaDownloadProgressTracker {
    pub fn new(
        total_eligible: usize,
        total_bytes_known: Option<u64>,
        quiet: bool,
        json: bool,
    ) -> Self {
        let is_tty = std::io::stderr().is_terminal();
        let interval = if is_tty {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(5)
        };

        Self {
            quiet,
            json,
            is_tty,
            total_eligible,
            total_bytes_known,
            start_time: Instant::now(),
            throttle: Throttle::new(interval),
            last_reported_count: 0,
        }
    }

    pub fn on_progress(&mut self, event: &vendetta_media::DownloadProgressEvent) {
        if self.quiet || self.json {
            return;
        }

        let now = Instant::now();
        let is_done = event.completed_count >= self.total_eligible;
        let count_delta = event
            .completed_count
            .saturating_sub(self.last_reported_count);

        if !is_done && self.is_tty && !self.throttle.should_render(now) {
            return;
        }

        if !is_done && !self.is_tty && count_delta < 100 && !self.throttle.should_render(now) {
            return;
        }

        self.last_reported_count = event.completed_count;

        let elapsed_secs = self.start_time.elapsed().as_secs_f64().max(0.001);
        let throughput_bps = (event.downloaded_bytes as f64) / elapsed_secs;
        let throughput_str = format_bytes(throughput_bps as u64) + "/s";

        let percent = if self.total_eligible > 0 {
            ((event.completed_count as f64 / self.total_eligible as f64) * 100.0).min(100.0)
        } else {
            100.0
        };

        let eta_str = if event.completed_count >= self.total_eligible {
            "done".to_string()
        } else if let Some(total_bytes) = self.total_bytes_known {
            if total_bytes > event.downloaded_bytes && throughput_bps > 0.0 {
                let remaining_bytes = total_bytes.saturating_sub(event.downloaded_bytes);
                let eta_secs = (remaining_bytes as f64 / throughput_bps) as u64;
                format_duration(eta_secs)
            } else {
                "--:--".to_string()
            }
        } else if self.total_eligible > event.completed_count && event.completed_count > 0 {
            let items_per_sec = event.completed_count as f64 / elapsed_secs;
            let remaining_items = self.total_eligible - event.completed_count;
            let eta_secs = (remaining_items as f64 / items_per_sec) as u64;
            format_duration(eta_secs)
        } else {
            "--:--".to_string()
        };

        let downloaded_str = format_bytes(event.downloaded_bytes);
        let line = if let Some(total_bytes) = self.total_bytes_known {
            let total_bytes_str = format_bytes(total_bytes);
            format!(
                "[{:>4}s] {}/{} ({:>5.1}%) | {}/{} ({}) | retry: {}, reauth: {}, fail: {} | ETA: {}",
                elapsed_secs as u64,
                event.completed_count,
                self.total_eligible,
                percent,
                downloaded_str,
                total_bytes_str,
                throughput_str,
                event.retry_wait_count,
                event.needs_reauth_count,
                event.permanently_failed_count,
                eta_str,
            )
        } else {
            format!(
                "[{:>4}s] {}/{} ({:>5.1}%) | {} downloaded ({}) | retry: {}, reauth: {}, fail: {} | ETA: {}",
                elapsed_secs as u64,
                event.completed_count,
                self.total_eligible,
                percent,
                downloaded_str,
                throughput_str,
                event.retry_wait_count,
                event.needs_reauth_count,
                event.permanently_failed_count,
                eta_str,
            )
        };

        render_progress_line(&line, self.is_tty);
    }

    pub fn finish(&self) {
        if self.quiet || self.json {
            return;
        }
        if self.is_tty {
            eprintln!();
            let _ = std::io::stderr().flush();
        }
    }
}

pub struct MediaVerifyProgressTracker {
    quiet: bool,
    json: bool,
    is_tty: bool,
    total_checked: usize,
    start_time: Instant,
    throttle: Throttle,
    last_reported_count: usize,
}

impl MediaVerifyProgressTracker {
    pub fn new(total_checked: usize, quiet: bool, json: bool) -> Self {
        let is_tty = std::io::stderr().is_terminal();
        let interval = if is_tty {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(5)
        };

        Self {
            quiet,
            json,
            is_tty,
            total_checked,
            start_time: Instant::now(),
            throttle: Throttle::new(interval),
            last_reported_count: 0,
        }
    }

    pub fn on_progress(
        &mut self,
        current: usize,
        total: usize,
        report: &vendetta_media::VerificationReport,
    ) {
        if self.quiet || self.json {
            return;
        }

        let total = total.max(self.total_checked);
        let now = Instant::now();
        let is_done = current >= total;
        let count_delta = current.saturating_sub(self.last_reported_count);

        if !is_done && self.is_tty && !self.throttle.should_render(now) {
            return;
        }

        if !is_done && !self.is_tty && count_delta < 100 && !self.throttle.should_render(now) {
            return;
        }

        self.last_reported_count = current;

        let elapsed_secs = self.start_time.elapsed().as_secs_f64().max(0.001);
        let items_per_sec = (current as f64) / elapsed_secs;
        let percent = if total > 0 {
            (current as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        let eta_secs = if total > current && items_per_sec > 0.0 {
            ((total - current) as f64 / items_per_sec) as u64
        } else {
            0
        };
        let eta_str = if current < total {
            format_duration(eta_secs)
        } else {
            "00:00".to_string()
        };

        let line = format!(
            "[{:>4}s] {}/{} ({:>5.1}%) | verified: {}, missing: {}, corrupt_size: {}, corrupt_hash: {} | {:>4.1} items/s | ETA: {}",
            elapsed_secs as u64,
            current,
            total,
            percent,
            report.verified_count,
            report.missing_count,
            report.corrupted_size_count,
            report.corrupted_hash_count,
            items_per_sec,
            eta_str,
        );

        render_progress_line(&line, self.is_tty);
    }

    pub fn finish(&self) {
        if self.quiet || self.json {
            return;
        }
        clear_progress_line(self.is_tty);
    }
}

pub struct SyncProgressTracker {
    quiet: bool,
    json: bool,
    is_tty: bool,
    start_time: Instant,
    throttle: Throttle,
    last_reported_messages: usize,
    last_reported_step: Option<vendetta_sync::SyncStep>,
    last_reported_peer: Option<vendetta_model::PeerId>,
}

impl SyncProgressTracker {
    pub fn new(quiet: bool, json: bool) -> Self {
        let is_tty = std::env::var("VENDETTA_FORCE_TTY")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or_else(|_| std::io::stderr().is_terminal());
        Self::with_tty(quiet, json, is_tty)
    }

    pub fn with_tty(quiet: bool, json: bool, is_tty: bool) -> Self {
        let interval = if is_tty {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(5)
        };

        Self {
            quiet,
            json,
            is_tty,
            start_time: Instant::now(),
            throttle: Throttle::new(interval),
            last_reported_messages: 0,
            last_reported_step: None,
            last_reported_peer: None,
        }
    }

    pub fn format_line(&self, event: &vendetta_sync::SyncProgressEvent) -> String {
        let elapsed = self.start_time.elapsed();
        let elapsed_str = format_duration(elapsed.as_secs());
        let elapsed_secs = elapsed.as_secs_f64().max(0.001);
        let rate = (event.total_messages_processed as f64) / elapsed_secs;
        let rate_str = format!("{:.0} msg/s", rate);

        match event.step {
            vendetta_sync::SyncStep::CapturingBaseline => {
                format!("[{}] Sync | Step: Capturing baseline state", elapsed_str)
            }
            vendetta_sync::SyncStep::IngestingHistory => {
                let msgs_formatted = format_number(event.total_messages_processed);
                let batches_str = event.total_batches_completed.to_string();

                let current_str = if let Some(ref name) = event.current_peer_name {
                    name.clone()
                } else if let Some(peer_id) = event.current_peer_id {
                    format!("p_{}", peer_id.raw())
                } else {
                    "-".to_string()
                };

                if let Some(flood_secs) = event.flood_wait_seconds {
                    format!(
                        "[{}] Sync | Step: Ingesting history | FloodWait: {}s | Current: {} | Rate: {}",
                        elapsed_str, flood_secs, current_str, rate_str
                    )
                } else if event.total_peers > 1 {
                    let eta_str = if event.peer_index > 0 && event.peer_index < event.total_peers {
                        let elapsed_per_peer = elapsed_secs / (event.peer_index as f64);
                        let remaining_peers = (event.total_peers - event.peer_index) as f64;
                        let eta_secs = (elapsed_per_peer * remaining_peers) as u64;
                        format!(" | ETA: {}", format_duration(eta_secs))
                    } else {
                        String::new()
                    };

                    format!(
                        "[{}] Sync | Step: Ingesting history | Peers: {}/{} | Msgs: {} | Batches: {} | Current: {} | Rate: {}{}",
                        elapsed_str,
                        event.peer_index,
                        event.total_peers,
                        msgs_formatted,
                        batches_str,
                        current_str,
                        rate_str,
                        eta_str
                    )
                } else {
                    format!(
                        "[{}] Sync | Step: Ingesting history | Current: {} | Msgs: {} | Batches: {} | Rate: {}",
                        elapsed_str, current_str, msgs_formatted, batches_str, rate_str
                    )
                }
            }
            vendetta_sync::SyncStep::ReconcilingUpdates => {
                let msgs_formatted = format_number(event.total_messages_processed);
                format!(
                    "[{}] Sync | Step: Reconciling updates | Msgs: {} | Rate: {}",
                    elapsed_str, msgs_formatted, rate_str
                )
            }
            vendetta_sync::SyncStep::ChannelDiscovery => {
                format!("[{}] Sync | Step: Discovering channels", elapsed_str)
            }
            vendetta_sync::SyncStep::ChannelQueue => {
                format!(
                    "[{}] Sync | Step: Synchronizing channel queues",
                    elapsed_str
                )
            }
            vendetta_sync::SyncStep::Finalizing => {
                format!("[{}] Sync | Step: Finalizing archive commit", elapsed_str)
            }
            vendetta_sync::SyncStep::DiscoveringDialogs => {
                format!("[{}] Sync | Step: Discovering dialogs", elapsed_str)
            }
        }
    }

    pub fn on_progress(&mut self, event: &vendetta_sync::SyncProgressEvent) {
        if self.quiet || self.json {
            return;
        }

        let now = Instant::now();
        let step_changed = self.last_reported_step != Some(event.step);
        let peer_changed =
            event.current_peer_id.is_some() && event.current_peer_id != self.last_reported_peer;
        let is_final = event.step == vendetta_sync::SyncStep::Finalizing;

        if !step_changed && !peer_changed && !is_final {
            if self.is_tty {
                if !self.throttle.should_render(now) {
                    return;
                }
            } else {
                let msgs_delta = event
                    .total_messages_processed
                    .saturating_sub(self.last_reported_messages);
                if msgs_delta < 500 && !self.throttle.should_render(now) {
                    return;
                }
            }
        }

        self.last_reported_messages = event.total_messages_processed;
        self.last_reported_step = Some(event.step);
        if event.current_peer_id.is_some() {
            self.last_reported_peer = event.current_peer_id;
        }

        let line = self.format_line(event);
        render_progress_line(&line, self.is_tty);
    }

    pub fn finish(&self) {
        if self.quiet || self.json {
            return;
        }
        clear_progress_line(self.is_tty);
    }
}

pub fn format_number(n: usize) -> String {
    let s = n.to_string();
    let rem = s.len() % 3;
    let mut result = String::with_capacity(s.len() + s.len() / 3);

    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (i == rem || (i > rem && (i - rem).is_multiple_of(3))) {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.2} MB", b / MB)
    } else if b >= KB {
        format!("{:.2} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

pub fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let mins = (seconds % 3600) / 60;
    let secs = seconds % 60;
    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, mins, secs)
    } else {
        format!("{:02}:{:02}", mins, secs)
    }
}
