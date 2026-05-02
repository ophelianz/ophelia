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
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bitvec::prelude::{BitVec, Lsb0};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;
use tokio::time::{Instant as TokioInstant, Interval};
use tokio_util::sync::CancellationToken;

use crate::disk::{DiskLease, DiskSessionLease, DiskWriteFailure, DiskWriteResult, DiskWriter};
use crate::engine::chunk;
use crate::engine::http::config::HttpRangeStrategyConfig;
use crate::engine::http::throttle::Throttle;
use crate::engine::types::{
    ChunkSnapshot, ProgressUpdate, TaskRuntimeUpdate, TransferChunkMapState, TransferId,
    TransferStatus,
};

use super::chunk_map::snapshot_from_covered_ranges;
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
const NO_ATTEMPT_INDEX: usize = usize::MAX;

pub(super) struct RangeDownloadConfig {
    pub(super) id: TransferId,
    pub(super) url: String,
    pub(super) client: Arc<reqwest::Client>,
    pub(super) disk: DiskLease,
    pub(super) chunks: chunk::ChunkList,
    pub(super) total_bytes: u64,
    pub(super) chunk_map_support: ChunkMapSupport,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChunkMapSupport {
    Supported,
    Unsupported,
}

impl ChunkMapSupport {
    pub(super) fn from_supported(supported: bool) -> Self {
        if supported {
            Self::Supported
        } else {
            Self::Unsupported
        }
    }
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

#[derive(Default)]
struct AttemptControlTable {
    rows: AttemptControlRows,
    health: AttemptHealthTable,
    controls: AttemptCancelTable,
}

#[derive(Default)]
struct AttemptControlRows {
    generations: Vec<u32>,
    active: BitVec<usize, Lsb0>,
    active_rows: Vec<AttemptId>,
    active_positions: Vec<usize>,
}

#[derive(Default)]
struct AttemptHealthTable {
    started_at_ms: Vec<u64>,
    socket_bytes: Vec<u64>,
    previous_socket_bytes: Vec<u64>,
    ema_socket_bytes_per_tick: Vec<f64>,
}

#[derive(Default)]
struct AttemptCancelTable {
    live_stop_ats: Vec<Option<Arc<AtomicU64>>>,
    health_retry_tokens: Vec<Option<CancellationToken>>,
    hedge_lost_tokens: Vec<Option<CancellationToken>>,
}

struct RangeDownload {
    id: TransferId,
    url: String,
    client: Arc<reqwest::Client>,
    disk: Option<DiskSessionLease>,
    writer: Option<DiskWriter<AttemptId>>,
    scheduler: RangeScheduler,
    total_bytes: u64,
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
    attempts: AttemptControlTable,
    retry_counts: HashMap<(u64, u64), u32>,
    strategy_tick: Interval,
    health_tick: Interval,
    progress_tick: Interval,
    retry_cooldown_until: Option<TokioInstant>,
    speed: ProgressSpeed,
    pending_written_bytes: u64,
    flags: RangeDownloadFlags,
    stop_reason: RangeStopReason,
}

impl AttemptControlTable {
    fn len(&self) -> usize {
        self.rows.active_rows.len()
    }

    fn contains(&self, attempt: AttemptId) -> bool {
        self.index_for(attempt).is_some()
    }

    fn insert(&mut self, attempt: AttemptId, control: AttemptControl) {
        let slot = attempt.slot();
        self.grow_to_slot(slot);
        self.remove(attempt);

        self.rows.generations[slot] = attempt.generation();
        self.controls.live_stop_ats[slot] = Some(control.live_stop_at);
        self.controls.health_retry_tokens[slot] = Some(control.health_retry_token);
        self.controls.hedge_lost_tokens[slot] = Some(control.hedge_lost_token);
        self.health.started_at_ms[slot] = control.started_at_ms;
        self.health.socket_bytes[slot] = control.socket_bytes;
        self.health.previous_socket_bytes[slot] = control.previous_socket_bytes;
        self.health.ema_socket_bytes_per_tick[slot] = control.ema_socket_bytes_per_tick;
        self.rows.active.set(slot, true);
        self.rows.active_positions[slot] = self.rows.active_rows.len();
        self.rows.active_rows.push(attempt);
    }

    fn remove(&mut self, attempt: AttemptId) -> Option<AttemptControl> {
        let slot = self.index_for(attempt)?;
        self.rows.active.set(slot, false);
        self.remove_active_row(slot);
        Some(AttemptControl {
            live_stop_at: self.controls.live_stop_ats[slot].take()?,
            health_retry_token: self.controls.health_retry_tokens[slot].take()?,
            hedge_lost_token: self.controls.hedge_lost_tokens[slot].take()?,
            started_at_ms: self.health.started_at_ms[slot],
            socket_bytes: self.health.socket_bytes[slot],
            previous_socket_bytes: self.health.previous_socket_bytes[slot],
            ema_socket_bytes_per_tick: self.health.ema_socket_bytes_per_tick[slot],
        })
    }

    fn add_socket_bytes(&mut self, attempt: AttemptId, bytes: u64) -> bool {
        let Some(slot) = self.index_for(attempt) else {
            return false;
        };
        self.health.socket_bytes[slot] = self.health.socket_bytes[slot].saturating_add(bytes);
        true
    }

    fn store_live_stop(&self, attempt: AttemptId, stop_at: u64) -> bool {
        let Some(slot) = self.index_for(attempt) else {
            return false;
        };
        let Some(live_stop_at) = self.controls.live_stop_ats[slot].as_ref() else {
            return false;
        };
        live_stop_at.store(stop_at, Ordering::Release);
        true
    }

    fn cancel_hedge_lost(&self, attempt: AttemptId) {
        if let Some(slot) = self.index_for(attempt)
            && let Some(token) = self.controls.hedge_lost_tokens[slot].as_ref()
        {
            token.cancel();
        }
    }

    fn cancel_health_retry(&self, attempt: AttemptId) {
        if let Some(slot) = self.index_for(attempt)
            && let Some(token) = self.controls.health_retry_tokens[slot].as_ref()
        {
            token.cancel();
        }
    }

    fn health_samples(&mut self, now: u64) -> Vec<(AttemptId, f64)> {
        let mut samples = Vec::new();
        for attempt in self.rows.active_rows.iter().copied() {
            let Some(slot) = self.index_for(attempt) else {
                continue;
            };
            if self.controls.health_retry_tokens[slot]
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
            {
                continue;
            }
            if now.saturating_sub(self.health.started_at_ms[slot]) < HEALTH_GRACE_MS {
                continue;
            }

            let delta = self.health.socket_bytes[slot]
                .saturating_sub(self.health.previous_socket_bytes[slot]);
            self.health.previous_socket_bytes[slot] = self.health.socket_bytes[slot];
            self.health.ema_socket_bytes_per_tick[slot] = (1.0 - HEALTH_EMA_ALPHA)
                * self.health.ema_socket_bytes_per_tick[slot]
                + HEALTH_EMA_ALPHA * delta as f64;
            samples.push((attempt, self.health.ema_socket_bytes_per_tick[slot]));
        }
        samples
    }

    fn index_for(&self, attempt: AttemptId) -> Option<usize> {
        let slot = attempt.slot();
        if self.rows.generations.get(slot).copied()? != attempt.generation() {
            return None;
        }
        self.rows
            .active
            .get(slot)
            .is_some_and(|bit| *bit)
            .then_some(slot)
    }

    fn grow_to_slot(&mut self, slot: usize) {
        while self.rows.generations.len() <= slot {
            self.rows.generations.push(0);
            self.rows.active.push(false);
            self.rows.active_positions.push(NO_ATTEMPT_INDEX);
            self.controls.live_stop_ats.push(None);
            self.controls.health_retry_tokens.push(None);
            self.controls.hedge_lost_tokens.push(None);
            self.health.started_at_ms.push(0);
            self.health.socket_bytes.push(0);
            self.health.previous_socket_bytes.push(0);
            self.health.ema_socket_bytes_per_tick.push(0.0);
        }
    }

    fn remove_active_row(&mut self, slot: usize) {
        let pos = self.rows.active_positions[slot];
        if pos == NO_ATTEMPT_INDEX {
            return;
        }
        self.rows.active_rows.swap_remove(pos);
        if let Some(&moved) = self.rows.active_rows.get(pos) {
            self.rows.active_positions[moved.slot()] = pos;
        }
        self.rows.active_positions[slot] = NO_ATTEMPT_INDEX;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeStopReason {
    Running,
    Failed,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RangeDownloadFlags(u8);

impl RangeDownloadFlags {
    const CHUNK_MAP_SUPPORTED: u8 = 1 << 0;
    const CHUNK_MAP_DIRTY: u8 = 1 << 1;

    fn new(chunk_map_support: ChunkMapSupport) -> Self {
        let mut flags = Self(0);
        flags.set(
            Self::CHUNK_MAP_SUPPORTED,
            chunk_map_support == ChunkMapSupport::Supported,
        );
        flags
    }

    fn chunk_map_supported(self) -> bool {
        self.has(Self::CHUNK_MAP_SUPPORTED)
    }

    fn chunk_map_dirty(self) -> bool {
        self.has(Self::CHUNK_MAP_DIRTY)
    }

    fn mark_chunk_map_dirty(&mut self) {
        self.set(Self::CHUNK_MAP_DIRTY, true);
    }

    fn clear_chunk_map_dirty(&mut self) {
        self.set(Self::CHUNK_MAP_DIRTY, false);
    }

    fn has(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    fn set(&mut self, flag: u8, enabled: bool) {
        if enabled {
            self.0 |= flag;
        } else {
            self.0 &= !flag;
        }
    }
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

    fn sample(&mut self, status: TransferStatus, downloaded: u64) -> u64 {
        if status != TransferStatus::Downloading {
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
        let (disk, writer) = config.disk.split_for_writes();

        Self {
            id: config.id,
            url: config.url,
            client: config.client,
            disk: Some(disk),
            writer: Some(writer),
            scheduler,
            total_bytes: config.total_bytes,
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
            attempts: AttemptControlTable::default(),
            retry_counts: HashMap::new(),
            strategy_tick: interval_from_now(STRATEGY_TICK_MS),
            health_tick: interval_from_now(HEALTH_TICK_MS),
            progress_tick: interval_from_now_duration(
                config.progress_interval.max(Duration::from_millis(1)),
            ),
            retry_cooldown_until: None,
            speed: ProgressSpeed::new(initial_downloaded),
            pending_written_bytes: 0,
            flags: RangeDownloadFlags::new(config.chunk_map_support),
            stop_reason: RangeStopReason::Running,
        }
    }

    async fn run(mut self) -> super::task::TaskFinalState {
        self.send_progress(TransferStatus::Downloading).await;
        self.send_chunk_map().await;

        while !self.should_stop() {
            self.start_ready_attempts();

            if self.should_stop() {
                break;
            }

            if self.scheduler.active_len() == 0 && !self.retry_cooldown_active() {
                self.fail_download();
                break;
            }

            self.wait_for_event().await;
        }

        self.stop_workers().await;
        self.finish().await
    }

    fn should_stop(&self) -> bool {
        self.stop_reason != RangeStopReason::Running
            || (self.scheduler.is_complete() && self.scheduler.active_len() == 0)
    }

    fn fail_download(&mut self) {
        self.stop_reason = RangeStopReason::Failed;
    }

    fn pause_download(&mut self) {
        if self.stop_reason == RangeStopReason::Running {
            self.stop_reason = RangeStopReason::Paused;
        }
    }

    fn start_ready_attempts(&mut self) {
        if self.retry_cooldown_active() {
            return;
        }

        while self.scheduler.active_len() < self.connection_limit {
            if self.pause_token.is_cancelled() {
                self.pause_download();
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
            if self
                .attempts
                .store_live_stop(steal.victim, steal.victim_stop_at)
            {
                return true;
            }

            self.fail_download();
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
        let Some(write_jobs) = self.writer.as_ref().map(DiskWriter::sender) else {
            self.fail_download();
            return;
        };
        let Some(disk_session) = self.disk.as_ref().map(DiskSessionLease::session) else {
            self.fail_download();
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
            disk_session,
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
                    None => self.fail_download(),
                }
            }
            result = self.workers.join_next(), if !self.workers.is_empty() => {
                if !matches!(result, Some(Ok(()))) {
                    self.fail_download();
                }
            }
            _ = self.strategy_tick.tick(), if self.strategies.can_create_extra_work() => {
                self.start_ready_attempts();
            }
            _ = self.health_tick.tick(), if self.strategies.health_retry => {
                self.check_health();
            }
            _ = self.progress_tick.tick() => {
                self.send_progress(TransferStatus::Downloading).await;
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
            self.attempts.remove(attempt);
        }

        match action {
            SchedulerAction::Nothing => {}
            SchedulerAction::CountedProgress { new_bytes } => {
                if let Some(disk) = self.disk.as_ref() {
                    disk.confirm_logical(new_bytes);
                }
                self.flags.mark_chunk_map_dirty();
            }
            SchedulerAction::Requeued { range, retry_after } => {
                self.record_retry(range);
                self.apply_retry_after(retry_after);
            }
            SchedulerAction::PauseDownload => {
                self.pause_download();
            }
            SchedulerAction::FailDownload { .. } | SchedulerAction::UnknownAttempt { .. } => {
                self.fail_download();
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

        if !self.attempts.add_socket_bytes(*attempt, *bytes) {
            return false;
        }
        true
    }

    fn track_bytes_written(&mut self, event: &WorkerEvent) {
        let WorkerEvent::BytesWritten { attempt, written } = event else {
            return;
        };
        if !self.attempts.contains(*attempt) {
            return;
        }
        self.pending_written_bytes = self.pending_written_bytes.saturating_add(written.len());
    }

    fn cancel_attempt_as_hedge_loser(&self, attempt: AttemptId) {
        self.attempts.cancel_hedge_lost(attempt);
    }

    fn check_health(&mut self) {
        if self.attempts.len() < 2 {
            return;
        }

        let now = activation_now();
        let eligible = self.attempts.health_samples(now);

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
            if speed < mean * HEALTH_SLOW_FACTOR {
                self.attempts.cancel_health_retry(attempt);
            }
        }
    }

    fn record_retry(&mut self, range: ByteRange) {
        let key = (range.start(), range.end());
        let count = self.retry_counts.entry(key).or_default();
        *count += 1;

        if *count >= self.max_retries {
            self.fail_download();
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
        if self.stop_reason == RangeStopReason::Paused {
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
                            self.fail_download();
                            break;
                        }
                    }
                }
                result = self.workers.join_next() => {
                    match result {
                        Some(Ok(())) | None => {}
                        Some(Err(_error)) => self.fail_download(),
                    }
                }
            }
        }
    }

    fn apply_write_result(&mut self, result: DiskWriteResult<AttemptId>) {
        self.apply_event(worker_event_from_disk_result(result));
    }

    fn drain_worker_events(&mut self) {
        while let Ok(event) = self.events_rx.try_recv() {
            self.apply_event(event);
        }
    }

    async fn finish(mut self) -> super::task::TaskFinalState {
        match self.stop_reason {
            RangeStopReason::Running => {}
            RangeStopReason::Failed => {
                if let Some(disk) = self.disk.take() {
                    disk.mark_failed(None);
                }
                self.flush_runtime_updates().await;
                self.send_progress(TransferStatus::Error).await;
                return self.task_state(TransferStatus::Error);
            }
            RangeStopReason::Paused => {
                self.save_pause_snapshot();
                self.flush_runtime_updates().await;
                self.send_progress(TransferStatus::Paused).await;
                return self.task_state(TransferStatus::Paused);
            }
        }

        if !self.scheduler.is_complete() {
            self.flush_runtime_updates().await;
            self.send_progress(TransferStatus::Error).await;
            return self.task_state(TransferStatus::Error);
        }

        let Some(disk) = self.disk.take() else {
            self.flush_runtime_updates().await;
            self.send_progress(TransferStatus::Error).await;
            return self.task_state(TransferStatus::Error);
        };

        match disk.commit() {
            Ok(()) => {
                self.flush_runtime_updates().await;
                self.send_progress(TransferStatus::Finished).await;
                self.task_state(TransferStatus::Finished)
            }
            Err(error) => {
                tracing::error!(err = %error, "rename failed after range download");
                self.flush_runtime_updates().await;
                self.send_progress(TransferStatus::Error).await;
                self.task_state(TransferStatus::Error)
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

    async fn send_progress(&mut self, status: TransferStatus) {
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

    fn downloaded_for_status(&self, status: TransferStatus) -> u64 {
        if status == TransferStatus::Finished {
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
            .send(TaskRuntimeUpdate::TransferBytesWritten { id: self.id, bytes })
            .await;
    }

    async fn send_dirty_chunk_map(&mut self) {
        if !self.flags.chunk_map_dirty() {
            return;
        }

        self.flags.clear_chunk_map_dirty();
        self.send_chunk_map().await;
    }

    async fn send_chunk_map(&self) {
        if !self.flags.chunk_map_supported() {
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

    fn task_state(&self, status: TransferStatus) -> super::task::TaskFinalState {
        super::task::TaskFinalState {
            status,
            downloaded_bytes: self.downloaded_for_status(status),
            total_bytes: Some(self.total_bytes),
        }
    }
}

fn worker_event_from_disk_result(result: DiskWriteResult<AttemptId>) -> WorkerEvent {
    match result {
        DiskWriteResult::Written { owner, range, .. } => {
            let Some(written) = ByteRange::new(range.start(), range.end()) else {
                return WorkerEvent::Failed {
                    attempt: owner,
                    failure: super::events::WorkerFailure::FatalIo {
                        message: "disk writer returned invalid range".to_string(),
                    },
                };
            };
            WorkerEvent::BytesWritten {
                attempt: owner,
                written,
            }
        }
        DiskWriteResult::Failed { owner, failure, .. } => WorkerEvent::Failed {
            attempt: owner,
            failure: worker_failure_from_disk(failure),
        },
    }
}

fn worker_failure_from_disk(failure: DiskWriteFailure) -> super::events::WorkerFailure {
    match failure {
        DiskWriteFailure::FatalIo { message } => super::events::WorkerFailure::FatalIo { message },
        DiskWriteFailure::RetryableIo { message } => {
            super::events::WorkerFailure::RetryableIo { message }
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
    use crate::engine::types::TransferStatus;

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

        let sample = speed.sample(TransferStatus::Downloading, 3_000);

        assert!(sample > 0);
        assert!(sample < 1_000);
    }

    #[test]
    fn progress_speed_reports_zero_for_terminal_status() {
        let mut speed = ProgressSpeed::new(0);
        speed.window_start = Instant::now() - Duration::from_secs_f64(PROGRESS_WINDOW_SECS + 0.1);
        let _ = speed.sample(TransferStatus::Downloading, 3_000);

        assert_eq!(speed.sample(TransferStatus::Finished, 3_000), 0);
    }
}
