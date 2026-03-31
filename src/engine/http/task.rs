//! HTTP/HTTPS download orchestrator.
//!
//! `download_task` drives the full lifecycle of one download:
//!   1. Probe server for range support and file size
//!   2. Allocate the .ophelia_part file (platform-specific preallocation)
//!   3. Split into chunks; restore from snapshots on resume
//!   4. Spawn chunk workers via make_chunk_fut (retry loop per slot)
//!   5. Drain completions; ramp concurrency; trigger work stealing
//!   6. Background: progress reporter + health monitor
//!   7. On finish: atomic rename; on pause: write snapshots to pause_sink

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::engine::alloc::preallocate;
use crate::engine::chunk;
use crate::engine::http::HttpDownloadConfig;
use crate::engine::types::{ChunkSnapshot, DownloadId, DownloadStatus, ProgressUpdate};

use super::error::{ChunkError, ChunkOutcome};
use super::health::{activation_now, spawn_health_monitor};
use super::probe::probe;
use super::progress::spawn_progress_reporter;
use super::single::single_download;
use super::steal::{try_hedge, try_steal};
use super::worker::download_chunk;

/// Retry loop for a single chunk slot. Runs until the chunk finishes, the
/// download is paused, or retries are exhausted. Each attempt stamps a fresh
/// kill token so the health monitor always cancels the current connection.
async fn chunk_retry_loop(
    i: usize,
    url: String,
    client: Arc<reqwest::Client>,
    file: Arc<std::fs::File>,
    counters: Arc<Vec<AtomicU64>>,
    starts: Arc<Vec<AtomicU64>>,
    ends: Arc<Vec<AtomicU64>>,
    kills: Arc<Vec<Mutex<CancellationToken>>>,
    activation: Arc<Vec<AtomicU64>>,
    pause_token: CancellationToken,
    write_buffer_size: usize,
    stall_timeout: Duration,
    max_retries: u32,
) -> (usize, ChunkOutcome) {
    let mut attempt = 0u32;
    loop {
        let kill_token = {
            let new = CancellationToken::new();
            *kills[i].lock().unwrap() = new.clone();
            new
        };
        activation[i].store(activation_now(), Ordering::Relaxed);

        let start = starts[i].load(Ordering::Acquire);
        let end = ends[i].load(Ordering::Acquire);
        let resume_from = counters[i].load(Ordering::Relaxed);

        match download_chunk(
            &client, &url, start, end, resume_from, &file, &counters,
            i, write_buffer_size, stall_timeout, &pause_token, &kill_token,
        ).await {
            Ok(()) => return (i, ChunkOutcome::Finished),
            Err(ChunkError::Paused) => return (i, ChunkOutcome::Paused),
            Err(ChunkError::Killed) => {
                tracing::debug!(chunk = i, "slow worker killed, retrying");
                continue;
            }
            Err(ChunkError::Fatal(msg)) => {
                tracing::error!(chunk = i, msg, "fatal chunk error");
                return (i, ChunkOutcome::Failed);
            }
            Err(ChunkError::NonRetryable) => {
                tracing::error!(chunk = i, "non-retryable server error");
                return (i, ChunkOutcome::Failed);
            }
            Err(ChunkError::Retryable { retry_after }) => {
                if counters[i].load(Ordering::Relaxed) > resume_from {
                    attempt = 0;
                } else {
                    attempt += 1;
                }
                if attempt >= max_retries {
                    tracing::error!(chunk = i, attempt, "max retries exceeded");
                    return (i, ChunkOutcome::Failed);
                }
                let delay = retry_after.unwrap_or_else(|| {
                    Duration::from_secs(2u64.pow(attempt.min(5)).min(30))
                });
                tracing::warn!(chunk = i, attempt, delay_secs = delay.as_secs(), "retrying chunk");
                tokio::select! {
                    biased;
                    _ = pause_token.cancelled() => return (i, ChunkOutcome::Paused),
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        }
    }
}

/// Entry point. `pause_sink` is written on soft pause so the engine actor can
/// read chunk offsets for resume. `resume_from` is `Some` when continuing a
/// previously paused download.
#[tracing::instrument(
    name = "download",
    skip(config, progress_tx, pause_token, pause_sink, resume_from),
    fields(id = id.0, %url)
)]
pub async fn download_task(
    id: DownloadId,
    url: String,
    destination: PathBuf,
    config: HttpDownloadConfig,
    progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
    pause_token: CancellationToken,
    pause_sink: Arc<Mutex<Option<Vec<ChunkSnapshot>>>>,
    resume_from: Option<Vec<ChunkSnapshot>>,
) {
    let send = |status: DownloadStatus, downloaded: u64, total: Option<u64>, speed: u64| {
        let _ = progress_tx.send(ProgressUpdate {
            id,
            status,
            downloaded_bytes: downloaded,
            total_bytes: total,
            speed_bytes_per_sec: speed,
        });
    };

    // Probe uses the default client (HTTP/2 fine for a single request)
    // Chunk downloads use an HTTP/1.1-only client so each range request gets its
    // own TCP connection, HTTP/2 would multiplex all chunks onto one connection,
    // defeating the whole point of parallel chunking.
    let probe_client = reqwest::Client::new();
    let chunk_client = Arc::new(
        reqwest::Client::builder()
            .http1_only()
            .build()
            .expect("failed to build HTTP/1.1 client"),
    );

    // .ophelia_part suffix prevents other apps from opening partial files,
    // on completion we rename to the final destination atomically.
    let part_path = {
        let mut p = destination.clone();
        let name = p
            .file_name()
            .map(|n| format!("{}.ophelia_part", n.to_string_lossy()))
            .unwrap_or_else(|| "download.ophelia_part".into());
        p.set_file_name(name);
        p
    };

    // --- 1. Resolve total size, chunk boundaries, and file handle ---
    //
    // Resume: restore from snapshot, open existing .ophelia_part (no truncation).
    // Fresh:  probe server, allocate file with platform preallocation, split chunks.
    let (total_bytes, chunks, file) = match resume_from {
        Some(snapshots) => {
            let total = snapshots.last().map(|s| s.end).unwrap_or(0);
            let cl = chunk::ChunkList {
                starts: snapshots.iter().map(|s| s.start).collect(),
                ends: snapshots.iter().map(|s| s.end).collect(),
                downloaded: snapshots.iter().map(|s| s.downloaded).collect(),
                statuses: snapshots
                    .iter()
                    .map(|s| {
                        if s.downloaded >= s.end - s.start {
                            chunk::ChunkStatus::Finished
                        } else {
                            chunk::ChunkStatus::Pending
                        }
                    })
                    .collect(),
            };
            let file = match std::fs::OpenOptions::new().write(true).open(&part_path) {
                Ok(f) => f,
                Err(_) => { send(DownloadStatus::Error, 0, Some(total), 0); return; }
            };
            tracing::info!(total_bytes = total, chunks = cl.len(), "resuming chunked download");
            (total, cl, file)
        }
        None => {
            let probe_result = match probe(&probe_client, &url).await {
                Ok(p) => p,
                Err(_) => { send(DownloadStatus::Error, 0, None, 0); return; }
            };
            tracing::debug!(
                accepts_ranges = probe_result.accepts_ranges,
                content_length = probe_result.content_length,
                "probe complete"
            );

            let total_bytes = match probe_result.content_length {
                Some(len) => len,
                None => {
                    tracing::info!("no content-length, falling back to single stream");
                    single_download(
                        id, Arc::clone(&chunk_client), url,
                        part_path, destination, config.stall_timeout_secs, progress_tx,
                    ).await;
                    return;
                }
            };

            // O_CREAT | O_EXCL: fail if the part file already exists.
            // Two concurrent downloads targeting the same destination would otherwise
            // silently corrupt each other. The engine actor should prevent this at a
            // higher level, but defense-in-depth here.
            let file = match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&part_path)
            {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    tracing::warn!("part file already exists, another download may be active");
                    send(DownloadStatus::Error, 0, Some(total_bytes), 0);
                    return;
                }
                Err(_) => { send(DownloadStatus::Error, 0, Some(total_bytes), 0); return; }
            };

            if preallocate(&file, total_bytes).is_err() {
                send(DownloadStatus::Error, 0, Some(total_bytes), 0);
                return;
            }

            // Square-root heuristic for connection count (Surge pattern):
            //   workers = round(sqrt(file_size_mb)) clamped to [min, max]
            // Avoids over-connecting on small files and under-connecting on large ones.
            let num_chunks = if probe_result.accepts_ranges {
                let mb = total_bytes as f64 / (1024.0 * 1024.0);
                let sqrt_conns = mb.sqrt().round() as usize;
                sqrt_conns.clamp(config.min_connections, config.max_connections)
            } else {
                1
            };

            tracing::info!(total_bytes, num_chunks, "starting chunked download");
            let chunks = chunk::split(total_bytes, num_chunks);
            (total_bytes, chunks, file)
        }
    };

    let file = Arc::new(file);

    // --- 2. Extract config values ---
    let write_buffer_size = config.write_buffer_size;
    let progress_interval_ms = config.progress_interval_ms;
    let stall_timeout = Duration::from_secs(config.stall_timeout_secs);
    let max_retries = config.max_retries_per_chunk;
    let min_steal_bytes = config.min_steal_bytes;
    let num_initial_chunks = chunks.len();

    // --- 3. Per-slot atomic arrays, pre-allocated with headroom for stolen chunks ---
    //
    // Steal budget = num_initial_chunks: at most one steal per initial chunk, so
    // pre-allocating num_initial_chunks extra slots is always sufficient.
    let steal_budget = num_initial_chunks;
    let total_slots = num_initial_chunks + steal_budget;

    let chunk_starts_atomic: Arc<Vec<AtomicU64>> = Arc::new(
        chunks.starts.iter().map(|&s| AtomicU64::new(s))
            .chain((0..steal_budget).map(|_| AtomicU64::new(0)))
            .collect(),
    );
    let chunk_ends_atomic: Arc<Vec<AtomicU64>> = Arc::new(
        chunks.ends.iter().map(|&e| AtomicU64::new(e))
            .chain((0..steal_budget).map(|_| AtomicU64::new(0)))
            .collect(),
    );
    let chunk_downloaded: Arc<Vec<AtomicU64>> = Arc::new(
        chunks.downloaded.iter().map(|&d| AtomicU64::new(d))
            .chain((0..steal_budget).map(|_| AtomicU64::new(0)))
            .collect(),
    );

    // Kill tokens: Mutex so each attempt can swap in a fresh CancellationToken.
    // CancellationToken is one-shot so once cancelled it can't be reset.
    let kill_tokens: Arc<Vec<Mutex<CancellationToken>>> = Arc::new(
        (0..total_slots).map(|_| Mutex::new(CancellationToken::new())).collect(),
    );

    // Slot activation timestamps (ms since UNIX epoch) Initialised to u64::MAX
    // so slots that haven't started yet are never eligible for health-monitor killing;
    // make_chunk_fut writes activation_now() at the top of every attempt.
    let slot_activation: Arc<Vec<AtomicU64>> = Arc::new(
        (0..total_slots).map(|_| AtomicU64::new(u64::MAX)).collect(),
    );

    let next_slot = Arc::new(AtomicUsize::new(num_initial_chunks));

    // --- 4. Initial state ---
    let already_done: u64 = chunk_downloaded[..num_initial_chunks]
        .iter()
        .map(|a| a.load(Ordering::Relaxed))
        .sum();
    send(DownloadStatus::Downloading, already_done, Some(total_bytes), 0);

    let mut pending: VecDeque<usize> = (0..num_initial_chunks)
        .filter(|&i| chunks.statuses[i] != chunk::ChunkStatus::Finished)
        .collect();

    let mut active: HashSet<usize> = HashSet::new();
    let active_shared: Arc<Mutex<HashSet<usize>>> = Arc::new(Mutex::new(HashSet::new()));
    let mut join_set: JoinSet<(usize, ChunkOutcome)> = JoinSet::new();
    let mut current_limit: usize = 1;
    let mut all_ok = true;

    // --- 5. Chunk spawner ---
    //
    // Thin closure that clones all shared state and hands it to chunk_retry_loop.
    // The logic lives in that top-level async fn; this just wires up the Arcs.
    let make_chunk_fut = |i: usize| {
        chunk_retry_loop(
            i, url.clone(), Arc::clone(&chunk_client), Arc::clone(&file),
            Arc::clone(&chunk_downloaded), Arc::clone(&chunk_starts_atomic),
            Arc::clone(&chunk_ends_atomic), Arc::clone(&kill_tokens),
            Arc::clone(&slot_activation), pause_token.clone(),
            write_buffer_size, stall_timeout, max_retries,
        )
    };

    if let Some(i) = pending.pop_front() {
        active.insert(i);
        active_shared.lock().unwrap().insert(i);
        join_set.spawn(make_chunk_fut(i));
    }

    // --- 6. Background tasks ---
    let progress_handle = spawn_progress_reporter(
        id,
        Arc::clone(&chunk_downloaded),
        Arc::clone(&next_slot),
        total_bytes,
        already_done,
        progress_interval_ms,
        progress_tx.clone(),
    );

    let health_handle = spawn_health_monitor(
        Arc::clone(&chunk_downloaded),
        Arc::clone(&kill_tokens),
        Arc::clone(&active_shared),
        Arc::clone(&slot_activation),
        pause_token.clone(),
    );

    // --- 7. Drain loop ---
    //
    // A 200ms interval drives proactive steal/hedge when workers are imbalanced —
    // stealing only on completion would miss cases where one chunk is much larger
    // than the others mid-download (Surge's balancer goroutine pattern).
    //
    // HedgeWork: when try_steal finds nothing to split, spawn a duplicate connection
    // on the same remaining range. write_at is idempotent so both workers writing
    // the same bytes is safe. The first to finish snaps the original's counter to
    // the full chunk size, causing the original to exit on its next attempt when it
    // sees byte_start >= chunk_end.
    let mut paused = false;
    let mut hedge_for: HashMap<usize, usize> = HashMap::new(); // hedge_slot → original_slot
    let mut balancer = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_millis(200),
        Duration::from_millis(200),
    );

    loop {
        if join_set.is_empty() { break; }

        let mut chunk_done: Option<(usize, ChunkOutcome)> = None;
        tokio::select! {
            biased;
            result = join_set.join_next() => {
                match result {
                    Some(Ok(pair)) => chunk_done = Some(pair),
                    Some(Err(_panic)) => all_ok = false,
                    None => break,
                }
            }
            _ = balancer.tick(), if !paused => {}
        }

        if let Some((finished_i, outcome)) = chunk_done {
            active.remove(&finished_i);
            active_shared.lock().unwrap().remove(&finished_i);

            if let Some(&original) = hedge_for.get(&finished_i) {
                // Hedge finished first: snap original's counter to the full chunk
                // range so its next attempt sees byte_start >= chunk_end and exits.
                let range = chunk_ends_atomic[original].load(Ordering::Relaxed)
                    .saturating_sub(chunk_starts_atomic[original].load(Ordering::Relaxed));
                chunk_downloaded[original].store(range, Ordering::Relaxed);
                kill_tokens[original].lock().unwrap().cancel();
                hedge_for.remove(&finished_i);
            } else {
                // Original finished: cancel its hedge if one is running.
                let h = hedge_for.iter().find(|&(_, &o)| o == finished_i).map(|(&h, _)| h);
                if let Some(h) = h {
                    kill_tokens[h].lock().unwrap().cancel();
                    hedge_for.remove(&h);
                }

                if !paused {
                    match outcome {
                        ChunkOutcome::Finished => {
                            current_limit = (current_limit * 2).min(total_slots);
                        }
                        ChunkOutcome::Paused => paused = true,
                        ChunkOutcome::Failed => all_ok = false,
                    }
                } else if matches!(outcome, ChunkOutcome::Paused) {
                    paused = true;
                }
            }
        }

        if !paused {
            // Steal first; if nothing to steal and there is spare capacity, hedge.
            if pending.is_empty() && join_set.len() < current_limit {
                try_steal(
                    &chunk_starts_atomic, &chunk_ends_atomic, &chunk_downloaded,
                    &active, &next_slot, &mut pending,
                    write_buffer_size as u64, min_steal_bytes,
                );
                if pending.is_empty() && !active.is_empty() {
                    if let Some((h, orig)) = try_hedge(
                        &chunk_starts_atomic, &chunk_ends_atomic, &chunk_downloaded,
                        &active, &next_slot, &mut pending, min_steal_bytes,
                    ) {
                        hedge_for.insert(h, orig);
                    }
                }
            }
            while join_set.len() < current_limit {
                let Some(i) = pending.pop_front() else { break };
                active.insert(i);
                active_shared.lock().unwrap().insert(i);
                join_set.spawn(make_chunk_fut(i));
            }
        }
    }

    progress_handle.abort();
    health_handle.abort();
    drop(file); // close before rename on Windows

    // --- 8. Completion ---
    if paused {
        let populated = next_slot.load(Ordering::Relaxed);
        let snapshots: Vec<ChunkSnapshot> = (0..populated)
            .map(|i| ChunkSnapshot {
                start: chunk_starts_atomic[i].load(Ordering::Relaxed),
                end: chunk_ends_atomic[i].load(Ordering::Relaxed),
                downloaded: chunk_downloaded[i].load(Ordering::Relaxed),
            })
            .collect();
        *pause_sink.lock().unwrap() = Some(snapshots);
        let total_downloaded: u64 = chunk_downloaded[..populated]
            .iter().map(|a| a.load(Ordering::Relaxed)).sum();
        tracing::info!(total_downloaded, total_bytes, "download paused");
        send(DownloadStatus::Paused, total_downloaded, Some(total_bytes), 0);
    } else if all_ok {
        match std::fs::rename(&part_path, &destination) {
            Ok(()) => {
                tracing::info!(total_bytes, "download finished");
                send(DownloadStatus::Finished, total_bytes, Some(total_bytes), 0);
            }
            Err(e) => {
                tracing::error!(err = %e, "rename failed after download");
                send(DownloadStatus::Error, total_bytes, Some(total_bytes), 0);
            }
        }
    } else {
        let populated = next_slot.load(Ordering::Relaxed);
        let total_downloaded: u64 = chunk_downloaded[..populated]
            .iter().map(|a| a.load(Ordering::Relaxed)).sum();
        tracing::error!(total_downloaded, total_bytes, "download failed");
        send(DownloadStatus::Error, total_downloaded, Some(total_bytes), 0);
    }
}
