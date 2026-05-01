/***************************************************
** This file is part of Ophelia.
** Copyright © 2026 Viktor Luna <viktor@hystericca.dev>
** Released under the GPL License, version 3 or later.
**
** If you found a weird little bug in here, tell the cat:
** viktor@hystericca.dev
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs behave plz, we're all trying our best )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

//! Runs one chunked HTTP download
//!
//! Starts range workers, receives their events, reports progress, saves pause data,
//! and moves the part file into place on success

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;
use tokio::time::{Instant as TokioInstant, Interval};
use tokio_util::sync::CancellationToken;

use crate::engine::chunk;
use crate::engine::destination::{FinalizeStrategy, finalize_part_file};
use crate::engine::http::config::HttpRangeStrategyConfig;
use crate::engine::http::throttle::Throttle;
use crate::engine::types::{
    ChunkSnapshot, DownloadId, DownloadStatus, ProgressUpdate, TaskRuntimeUpdate,
    TransferChunkMapState,
};

use super::chunk_map::snapshot_from_covered_ranges;
use super::disk_writer::{RangeDiskWriter, RangeWriteResult};
use super::events::{SchedulerAction, WorkerEvent};
use super::range_worker::{RangeWorkerConfig, run_range_worker};
use super::ranges::{ByteRange, RangeSet};
use super::scheduler::{ActiveAttempt, AttemptId, RangeScheduler};

const STRATEGY_TICK_MS: u64 = 200;
const HEALTH_TICK_MS: u64 = 1_000;
const HEALTH_GRACE_MS: u64 = 5_000;
const HEALTH_SLOW_FACTOR: f64 = 0.5;
const HEALTH_EMA_ALPHA: f64 = 0.3;
const PROGRESS_EMA_ALPHA: f64 = 0.3;
const PROGRESS_WINDOW_SECS: f64 = 2.0;
const WORKER_EVENT_CAPACITY: usize = 256;

pub(super) struct RangeDownloadConfig {
    pub(super) id: DownloadId,
    pub(super) url: String,
    pub(super) client: Arc<reqwest::Client>,
    pub(super) file: std::fs::File,
    pub(super) chunks: chunk::ChunkList,
    pub(super) total_bytes: u64,
    pub(super) part_path: PathBuf,
    pub(super) destination: PathBuf,
    pub(super) finalize_strategy: FinalizeStrategy,
    pub(super) chunk_map_supported: bool,
    pub(super) connection_limit: usize,
    pub(super) write_buffer_size: usize,
    pub(super) stall_timeout: Duration,
    pub(super) progress_interval: Duration,
    pub(super) max_retries: u32,
    pub(super) strategies: HttpRangeStrategyConfig,
    pub(super) safe_zone: u64,
    pub(super) min_steal_bytes: u64,
    pub(super) steal_align: u64,
    pub(super) pause_token: CancellationToken,
    pub(super) pause_sink: Arc<std::sync::Mutex<Option<Vec<ChunkSnapshot>>>>,
    pub(super) server_semaphore: Arc<Semaphore>,
    pub(super) throttle: Arc<Throttle>,
    pub(super) runtime_update_tx: mpsc::Sender<TaskRuntimeUpdate>,
}

pub(super) async fn run_range_download(config: RangeDownloadConfig) -> super::task::TaskFinalState {
    RangeDownload::new(config).run().await
}

struct AttemptControl {
    live_stop_at: Arc<AtomicU64>,
    health_retry_token: CancellationToken,
    hedge_lost_token: CancellationToken,
    started_at_ms: u64,
    socket_bytes: u64,
    previous_socket_bytes: u64,
    ema_socket_bytes_per_tick: f64,
}

struct RangeDownload {
    id: DownloadId,
    url: String,
    client: Arc<reqwest::Client>,
    writer: Option<RangeDiskWriter>,
    scheduler: RangeScheduler,
    total_bytes: u64,
    part_path: PathBuf,
    destination: PathBuf,
    finalize_strategy: FinalizeStrategy,
    chunk_map_supported: bool,
    connection_limit: usize,
    write_buffer_size: usize,
    stall_timeout: Duration,
    max_retries: u32,
    strategies: HttpRangeStrategyConfig,
    safe_zone: u64,
    min_steal_bytes: u64,
    steal_align: u64,
    pause_token: CancellationToken,
    pause_sink: Arc<std::sync::Mutex<Option<Vec<ChunkSnapshot>>>>,
    server_semaphore: Arc<Semaphore>,
    throttle: Arc<Throttle>,
    runtime_update_tx: mpsc::Sender<TaskRuntimeUpdate>,
    events_tx: mpsc::Sender<WorkerEvent>,
    events_rx: mpsc::Receiver<WorkerEvent>,
    workers: JoinSet<()>,
    attempts: HashMap<AttemptId, AttemptControl>,
    retry_counts: HashMap<(u64, u64), u32>,
    strategy_tick: Interval,
    health_tick: Interval,
    progress_tick: Interval,
    retry_cooldown_until: Option<TokioInstant>,
    speed: ProgressSpeed,
    pending_written_bytes: u64,
    chunk_map_dirty: bool,
    failed: bool,
    paused: bool,
}

struct ProgressSpeed {
    ema_speed: f64,
    window_start: Instant,
    window_bytes: u64,
    last_downloaded: u64,
}

impl ProgressSpeed {
    fn new(initial_downloaded: u64) -> Self {
        Self {
            ema_speed: 0.0,
            window_start: Instant::now(),
            window_bytes: 0,
            last_downloaded: initial_downloaded,
        }
    }

    fn sample(&mut self, status: DownloadStatus, downloaded: u64) -> u64 {
        if status != DownloadStatus::Downloading {
            self.last_downloaded = downloaded;
            self.window_bytes = 0;
            return 0;
        }

        let new_bytes = downloaded.saturating_sub(self.last_downloaded);
        self.last_downloaded = downloaded;
        self.window_bytes = self.window_bytes.saturating_add(new_bytes);

        let elapsed = self.window_start.elapsed().as_secs_f64();
        if elapsed >= PROGRESS_WINDOW_SECS {
            let recent = self.window_bytes as f64 / elapsed;
            self.ema_speed =
                (1.0 - PROGRESS_EMA_ALPHA) * self.ema_speed + PROGRESS_EMA_ALPHA * recent;
            if self.window_bytes == 0 {
                self.ema_speed *= PROGRESS_WINDOW_SECS / elapsed.max(PROGRESS_WINDOW_SECS);
            }
            self.window_bytes = 0;
            self.window_start = Instant::now();
        }

        self.ema_speed as u64
    }
}

impl RangeDownload {
    fn new(config: RangeDownloadConfig) -> Self {
        let (events_tx, events_rx) = mpsc::channel(WORKER_EVENT_CAPACITY);
        let scheduler = scheduler_from_chunks(config.total_bytes, &config.chunks);
        let initial_downloaded = scheduler.downloaded_bytes();
        let writer = RangeDiskWriter::spawn(config.file);

        Self {
            id: config.id,
            url: config.url,
            client: config.client,
            writer: Some(writer),
            scheduler,
            total_bytes: config.total_bytes,
            part_path: config.part_path,
            destination: config.destination,
            finalize_strategy: config.finalize_strategy,
            chunk_map_supported: config.chunk_map_supported,
            connection_limit: config.connection_limit.max(1),
            write_buffer_size: config.write_buffer_size,
            stall_timeout: config.stall_timeout,
            max_retries: config.max_retries,
            strategies: config.strategies,
            safe_zone: config.safe_zone,
            min_steal_bytes: config.min_steal_bytes,
            steal_align: config.steal_align,
            pause_token: config.pause_token,
            pause_sink: config.pause_sink,
            server_semaphore: config.server_semaphore,
            throttle: config.throttle,
            runtime_update_tx: config.runtime_update_tx,
            events_tx,
            events_rx,
            workers: JoinSet::new(),
            attempts: HashMap::new(),
            retry_counts: HashMap::new(),
            strategy_tick: interval_from_now(STRATEGY_TICK_MS),
            health_tick: interval_from_now(HEALTH_TICK_MS),
            progress_tick: interval_from_now_duration(
                config.progress_interval.max(Duration::from_millis(1)),
            ),
            retry_cooldown_until: None,
            speed: ProgressSpeed::new(initial_downloaded),
            pending_written_bytes: 0,
            chunk_map_dirty: false,
            failed: false,
            paused: false,
        }
    }

    async fn run(mut self) -> super::task::TaskFinalState {
        self.send_progress(DownloadStatus::Downloading).await;
        self.send_chunk_map().await;

        while !self.should_stop() {
            self.start_ready_attempts();

            if self.should_stop() {
                break;
            }

            if self.scheduler.active_len() == 0 && !self.retry_cooldown_active() {
                self.failed = true;
                break;
            }

            self.wait_for_event().await;
        }

        self.stop_workers().await;
        self.finish().await
    }

    fn should_stop(&self) -> bool {
        self.failed
            || self.paused
            || (self.scheduler.is_complete() && self.scheduler.active_len() == 0)
    }

    fn start_ready_attempts(&mut self) {
        if self.retry_cooldown_active() {
            return;
        }

        while self.scheduler.active_len() < self.connection_limit {
            if self.pause_token.is_cancelled() {
                self.paused = true;
                return;
            }

            if let Some(attempt) = self.scheduler.start_next_attempt() {
                self.spawn_attempt(attempt);
                continue;
            }

            if !self.try_strategy_once() {
                return;
            }
        }
    }

    fn try_strategy_once(&mut self) -> bool {
        self.try_steal_once() || self.try_hedge_once()
    }

    fn try_steal_once(&mut self) -> bool {
        if !self.strategies.stealing {
            return false;
        }

        if let Some(steal) =
            self.scheduler
                .steal_largest(self.safe_zone, self.min_steal_bytes, self.steal_align)
        {
            if let Some(control) = self.attempts.get(&steal.victim) {
                control
                    .live_stop_at
                    .store(steal.victim_stop_at, Ordering::Release);
                return true;
            }

            self.failed = true;
            return false;
        }

        false
    }

    fn try_hedge_once(&mut self) -> bool {
        if !self.strategies.hedging {
            return false;
        }

        if let Some(attempt) = self.scheduler.start_largest_hedge(self.min_steal_bytes) {
            self.spawn_attempt(attempt);
            return true;
        }

        false
    }

    fn spawn_attempt(&mut self, attempt: ActiveAttempt) {
        let Some(write_jobs) = self.writer.as_ref().map(RangeDiskWriter::sender) else {
            self.failed = true;
            return;
        };

        let live_stop_at = Arc::new(AtomicU64::new(attempt.stop_at()));
        let health_retry_token = CancellationToken::new();
        let hedge_lost_token = CancellationToken::new();
        let worker_config = RangeWorkerConfig {
            client: Arc::clone(&self.client),
            url: self.url.clone(),
            attempt,
            live_stop_at: Arc::clone(&live_stop_at),
            write_jobs,
            write_buffer_size: self.write_buffer_size,
            stall_timeout: self.stall_timeout,
            pause_token: self.pause_token.clone(),
            health_retry_token: health_retry_token.clone(),
            hedge_lost_token: hedge_lost_token.clone(),
            throttle: Arc::clone(&self.throttle),
            events: self.events_tx.clone(),
        };

        self.attempts.insert(
            attempt.id(),
            AttemptControl {
                live_stop_at,
                health_retry_token,
                hedge_lost_token,
                started_at_ms: activation_now(),
                socket_bytes: 0,
                previous_socket_bytes: 0,
                ema_socket_bytes_per_tick: 0.0,
            },
        );

        let pause_token = self.pause_token.clone();
        let semaphore = Arc::clone(&self.server_semaphore);
        let events = self.events_tx.clone();

        self.workers.spawn(async move {
            let permit = tokio::select! {
                biased;
                _ = pause_token.cancelled() => {
                    let event = WorkerEvent::Paused { attempt: attempt.id() };
                    let _ = events.send(event).await;
                    return;
                }
                result = semaphore.acquire_owned() => {
                    result.expect("server semaphore should stay open")
                }
            };

            run_range_worker(worker_config).await;
            drop(permit);
        });
    }

    async fn wait_for_event(&mut self) {
        tokio::select! {
            event = self.events_rx.recv() => {
                match event {
                    Some(event) => self.apply_event(event),
                    None => self.failed = true,
                }
            }
            result = self.workers.join_next(), if !self.workers.is_empty() => {
                if !matches!(result, Some(Ok(()))) {
                    self.failed = true;
                }
            }
            _ = self.strategy_tick.tick(), if self.strategies.can_create_extra_work() => {
                self.start_ready_attempts();
            }
            _ = self.health_tick.tick(), if self.strategies.health_retry => {
                self.check_health();
            }
            _ = self.progress_tick.tick() => {
                self.send_progress(DownloadStatus::Downloading).await;
                self.flush_runtime_updates().await;
            }
            _ = retry_cooldown_sleep(self.retry_cooldown_until), if self.retry_cooldown_until.is_some() => {
                self.retry_cooldown_until = None;
            }
        }
    }

    fn apply_event(&mut self, event: WorkerEvent) {
        if self.track_data_received(&event) {
            return;
        }
        self.track_bytes_written(&event);

        let terminal_attempt = terminal_attempt(&event);
        let action = self.scheduler.apply_worker_event(event);
        if let Some(attempt) = terminal_attempt
            && !matches!(action, SchedulerAction::UnknownAttempt { .. })
        {
            self.attempts.remove(&attempt);
        }

        match action {
            SchedulerAction::Nothing => {}
            SchedulerAction::CountedProgress { .. } => {
                self.chunk_map_dirty = true;
            }
            SchedulerAction::Requeued { range, retry_after } => {
                self.record_retry(range);
                self.apply_retry_after(retry_after);
            }
            SchedulerAction::PauseDownload => {
                self.paused = true;
            }
            SchedulerAction::FailDownload { .. } | SchedulerAction::UnknownAttempt { .. } => {
                self.failed = true;
            }
            SchedulerAction::CancelAttempt { attempt } => {
                self.cancel_attempt_as_hedge_loser(attempt);
            }
        }
    }

    fn track_data_received(&mut self, event: &WorkerEvent) -> bool {
        let WorkerEvent::DataReceived { attempt, bytes } = event else {
            return false;
        };

        let Some(control) = self.attempts.get_mut(attempt) else {
            return false;
        };

        control.socket_bytes = control.socket_bytes.saturating_add(*bytes);
        true
    }

    fn track_bytes_written(&mut self, event: &WorkerEvent) {
        let WorkerEvent::BytesWritten { attempt, written } = event else {
            return;
        };
        if !self.attempts.contains_key(attempt) {
            return;
        }
        self.pending_written_bytes = self.pending_written_bytes.saturating_add(written.len());
    }

    fn cancel_attempt_as_hedge_loser(&self, attempt: AttemptId) {
        if let Some(control) = self.attempts.get(&attempt) {
            control.hedge_lost_token.cancel();
        }
    }

    fn check_health(&mut self) {
        if self.attempts.len() < 2 {
            return;
        }

        let now = activation_now();
        let mut eligible = Vec::new();

        for (&attempt, control) in &mut self.attempts {
            if control.health_retry_token.is_cancelled() {
                continue;
            }

            if now.saturating_sub(control.started_at_ms) < HEALTH_GRACE_MS {
                continue;
            }

            let delta = control
                .socket_bytes
                .saturating_sub(control.previous_socket_bytes) as f64;
            control.previous_socket_bytes = control.socket_bytes;
            control.ema_socket_bytes_per_tick = (1.0 - HEALTH_EMA_ALPHA)
                * control.ema_socket_bytes_per_tick
                + HEALTH_EMA_ALPHA * delta;
            eligible.push((attempt, control.ema_socket_bytes_per_tick));
        }

        let positive_speeds = eligible
            .iter()
            .filter_map(|(_, speed)| (*speed > 0.0).then_some(*speed))
            .collect::<Vec<_>>();
        if positive_speeds.is_empty() {
            return;
        }

        let mean = positive_speeds.iter().sum::<f64>() / positive_speeds.len() as f64;
        if mean < 1.0 {
            return;
        }

        for (attempt, speed) in eligible {
            if speed < mean * HEALTH_SLOW_FACTOR
                && let Some(control) = self.attempts.get(&attempt)
            {
                control.health_retry_token.cancel();
            }
        }
    }

    fn record_retry(&mut self, range: ByteRange) {
        let key = (range.start(), range.end());
        let count = self.retry_counts.entry(key).or_default();
        *count += 1;

        if *count >= self.max_retries {
            self.failed = true;
        }
    }

    fn apply_retry_after(&mut self, retry_after: Option<Duration>) {
        let Some(retry_after) = retry_after else {
            return;
        };
        if retry_after.is_zero() {
            return;
        }

        let until = TokioInstant::now() + retry_after;
        self.retry_cooldown_until = Some(
            self.retry_cooldown_until
                .map_or(until, |current| current.max(until)),
        );
    }

    fn retry_cooldown_active(&mut self) -> bool {
        let Some(until) = self.retry_cooldown_until else {
            return false;
        };
        if TokioInstant::now() >= until {
            self.retry_cooldown_until = None;
            false
        } else {
            true
        }
    }

    async fn stop_workers(&mut self) {
        if self.paused && !self.failed {
            self.wait_for_workers_to_stop().await;
        } else {
            self.abort_workers().await;
        }

        if let Some(writer) = self.writer.take() {
            for result in writer.shutdown().await {
                self.apply_write_result(result);
            }
        }
        self.drain_worker_events();
    }

    async fn abort_workers(&mut self) {
        self.workers.abort_all();
        while self.workers.join_next().await.is_some() {}
    }

    async fn wait_for_workers_to_stop(&mut self) {
        // On pause, do not abort first
        // A worker may have a confirmed write but not yet have sent `BytesWritten`
        while !self.workers.is_empty() {
            tokio::select! {
                event = self.events_rx.recv() => {
                    match event {
                        Some(event) => self.apply_event(event),
                        None => {
                            self.failed = true;
                            break;
                        }
                    }
                }
                result = self.workers.join_next() => {
                    match result {
                        Some(Ok(())) | None => {}
                        Some(Err(_error)) => self.failed = true,
                    }
                }
            }
        }
    }

    fn apply_write_result(&mut self, result: RangeWriteResult) {
        self.apply_event(result.into_worker_event());
    }

    fn drain_worker_events(&mut self) {
        while let Ok(event) = self.events_rx.try_recv() {
            self.apply_event(event);
        }
    }

    async fn finish(mut self) -> super::task::TaskFinalState {
        if self.failed {
            self.flush_runtime_updates().await;
            self.send_progress(DownloadStatus::Error).await;
            return self.task_state(DownloadStatus::Error);
        }

        if self.paused {
            self.save_pause_snapshot();
            self.flush_runtime_updates().await;
            self.send_progress(DownloadStatus::Paused).await;
            return self.task_state(DownloadStatus::Paused);
        }

        if !self.scheduler.is_complete() {
            self.flush_runtime_updates().await;
            self.send_progress(DownloadStatus::Error).await;
            return self.task_state(DownloadStatus::Error);
        }

        match finalize_part_file(&self.part_path, &self.destination, self.finalize_strategy) {
            Ok(()) => {
                self.flush_runtime_updates().await;
                self.send_progress(DownloadStatus::Finished).await;
                self.task_state(DownloadStatus::Finished)
            }
            Err(error) => {
                tracing::error!(err = %error, "rename failed after range download");
                self.flush_runtime_updates().await;
                self.send_progress(DownloadStatus::Error).await;
                self.task_state(DownloadStatus::Error)
            }
        }
    }

    fn save_pause_snapshot(&mut self) {
        let mut snapshots = Vec::new();
        snapshots.extend(
            self.scheduler
                .completed_ranges()
                .iter()
                .map(|range| ChunkSnapshot {
                    start: range.start(),
                    end: range.end(),
                    downloaded: range.len(),
                }),
        );
        snapshots.extend(
            self.scheduler
                .pause_remaining()
                .ranges()
                .iter()
                .map(|range| ChunkSnapshot {
                    start: range.start(),
                    end: range.end(),
                    downloaded: 0,
                }),
        );
        snapshots.sort_by_key(|snapshot| (snapshot.start, snapshot.end));
        *self.pause_sink.lock().unwrap() = Some(snapshots);
    }

    async fn send_progress(&mut self, status: DownloadStatus) {
        let downloaded = self.downloaded_for_status(status);
        let speed = self.speed.sample(status, downloaded);
        let _ = self
            .runtime_update_tx
            .send(TaskRuntimeUpdate::Progress(ProgressUpdate {
                id: self.id,
                status,
                downloaded_bytes: downloaded,
                total_bytes: Some(self.total_bytes),
                speed_bytes_per_sec: speed,
            }))
            .await;
    }

    fn downloaded_for_status(&self, status: DownloadStatus) -> u64 {
        if status == DownloadStatus::Finished {
            self.total_bytes
        } else {
            self.scheduler.downloaded_bytes()
        }
    }

    async fn flush_runtime_updates(&mut self) {
        self.flush_written_bytes().await;
        self.send_dirty_chunk_map().await;
    }

    async fn flush_written_bytes(&mut self) {
        if self.pending_written_bytes == 0 {
            return;
        }

        let bytes = self.pending_written_bytes;
        self.pending_written_bytes = 0;
        let _ = self
            .runtime_update_tx
            .send(TaskRuntimeUpdate::DownloadBytesWritten { id: self.id, bytes })
            .await;
    }

    async fn send_dirty_chunk_map(&mut self) {
        if !self.chunk_map_dirty {
            return;
        }

        self.chunk_map_dirty = false;
        self.send_chunk_map().await;
    }

    async fn send_chunk_map(&self) {
        if !self.chunk_map_supported {
            return;
        }

        let snapshot = snapshot_from_covered_ranges(
            self.total_bytes,
            self.scheduler
                .completed_ranges()
                .iter()
                .map(|range| (range.start(), range.end())),
        );
        let _ = self
            .runtime_update_tx
            .send(TaskRuntimeUpdate::ChunkMapChanged {
                id: self.id,
                state: TransferChunkMapState::Http(snapshot),
            })
            .await;
    }

    fn task_state(&self, status: DownloadStatus) -> super::task::TaskFinalState {
        super::task::TaskFinalState {
            status,
            downloaded_bytes: self.downloaded_for_status(status),
            total_bytes: Some(self.total_bytes),
        }
    }
}

fn scheduler_from_chunks(total_bytes: u64, chunks: &chunk::ChunkList) -> RangeScheduler {
    let completed = completed_ranges_from_chunks(chunks);
    let pending = pending_ranges_from_chunks(chunks);
    RangeScheduler::from_completed_and_pending(total_bytes, completed, pending)
}

fn completed_ranges_from_chunks(chunks: &chunk::ChunkList) -> RangeSet {
    let ranges = chunks
        .starts
        .iter()
        .zip(&chunks.ends)
        .zip(&chunks.downloaded)
        .filter_map(|((&start, &end), &downloaded)| {
            let completed_end = start.saturating_add(downloaded).min(end);
            ByteRange::new(start, completed_end)
        });

    RangeSet::from_ranges(ranges)
}

fn pending_ranges_from_chunks(chunks: &chunk::ChunkList) -> Vec<ByteRange> {
    chunks
        .starts
        .iter()
        .zip(&chunks.ends)
        .zip(&chunks.downloaded)
        .zip(&chunks.statuses)
        .filter_map(|(((&start, &end), &downloaded), &status)| {
            if status == chunk::ChunkStatus::Finished {
                return None;
            }
            let pending_start = start.saturating_add(downloaded).min(end);
            ByteRange::new(pending_start, end)
        })
        .collect()
}

fn terminal_attempt(event: &WorkerEvent) -> Option<AttemptId> {
    match event {
        WorkerEvent::Finished { attempt }
        | WorkerEvent::Paused { attempt }
        | WorkerEvent::Failed { attempt, .. } => Some(*attempt),
        WorkerEvent::DataReceived { .. } | WorkerEvent::BytesWritten { .. } => None,
    }
}

fn interval_from_now(period_ms: u64) -> Interval {
    interval_from_now_duration(Duration::from_millis(period_ms))
}

fn interval_from_now_duration(period: Duration) -> Interval {
    tokio::time::interval_at(tokio::time::Instant::now() + period, period)
}

async fn retry_cooldown_sleep(until: Option<TokioInstant>) {
    if let Some(until) = until {
        tokio::time::sleep_until(until).await;
    }
}

fn activation_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{PROGRESS_WINDOW_SECS, ProgressSpeed, scheduler_from_chunks};
    use crate::engine::chunk::{ChunkList, ChunkStatus};
    use crate::engine::types::DownloadStatus;

    #[test]
    fn scheduler_from_chunks_uses_completed_prefixes() {
        let chunks = ChunkList {
            starts: vec![0, 50],
            ends: vec![50, 100],
            downloaded: vec![50, 10],
            statuses: vec![ChunkStatus::Finished, ChunkStatus::Pending],
        };

        let scheduler = scheduler_from_chunks(100, &chunks);

        assert_eq!(scheduler.downloaded_bytes(), 60);
        assert_eq!(scheduler.pending_len(), 1);
    }

    #[test]
    fn scheduler_from_chunks_keeps_fresh_chunk_boundaries() {
        let chunks = ChunkList {
            starts: vec![0, 50],
            ends: vec![50, 100],
            downloaded: vec![0, 0],
            statuses: vec![ChunkStatus::Pending, ChunkStatus::Pending],
        };

        let scheduler = scheduler_from_chunks(100, &chunks);

        assert_eq!(
            scheduler.pending_ranges().collect::<Vec<_>>(),
            vec![
                crate::engine::http::ranges::ByteRange::new(0, 50).unwrap(),
                crate::engine::http::ranges::ByteRange::new(50, 100).unwrap()
            ]
        );
    }

    #[test]
    fn progress_speed_reports_new_bytes_not_resumed_total() {
        let mut speed = ProgressSpeed::new(1_000);
        speed.window_start = Instant::now() - Duration::from_secs_f64(PROGRESS_WINDOW_SECS + 0.1);

        let sample = speed.sample(DownloadStatus::Downloading, 3_000);

        assert!(sample > 0);
        assert!(sample < 1_000);
    }

    #[test]
    fn progress_speed_reports_zero_for_terminal_status() {
        let mut speed = ProgressSpeed::new(0);
        speed.window_start = Instant::now() - Duration::from_secs_f64(PROGRESS_WINDOW_SECS + 0.1);
        let _ = speed.sample(DownloadStatus::Downloading, 3_000);

        assert_eq!(speed.sample(DownloadStatus::Finished, 3_000), 0);
    }
}
