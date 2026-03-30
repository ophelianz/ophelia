//! HTTP/HTTPS download pipeline.
//!
//! Probes server capabilities via GET+Range, then either drives parallel
//! chunked range requests (206) or falls back to a single stream (200).
//! Progress is tracked via per-chunk atomics; the timer starts after chunks
//! are spawned to exclude probe and allocation time from speed calculations.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use reqwest::StatusCode;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::engine::chunk;
use crate::engine::http::HttpDownloadConfig;
use crate::engine::types::{DownloadId, DownloadStatus, ProgressUpdate};

/// Classification of chunk-level errors, used to decide whether to retry.
enum ChunkError {
    /// Transient failure. `retry_after` is populated from the Retry-After header on 429.
    Retryable { retry_after: Option<Duration> },
    /// Server refused definitively (403, 404, 410). Retrying won't help.
    NonRetryable,
    /// Local failure (disk full, permission denied). Stops the entire download.
    Fatal(String),
}

fn classify_status(status: StatusCode, headers: &reqwest::header::HeaderMap) -> ChunkError {
    match status.as_u16() {
        403 | 404 | 410 => ChunkError::NonRetryable,
        429 => {
            let retry_after = headers
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            ChunkError::Retryable { retry_after }
        }
        500..=599 => ChunkError::Retryable { retry_after: None },
        _ => ChunkError::NonRetryable,
    }
}

fn classify_io_error(e: std::io::Error) -> ChunkError {
    match e.kind() {
        std::io::ErrorKind::StorageFull | std::io::ErrorKind::PermissionDenied => {
            ChunkError::Fatal(e.to_string())
        }
        _ => ChunkError::Retryable { retry_after: None },
    }
}

struct ProbeResult {
    content_length: Option<u64>,
    accepts_ranges: bool,
}

async fn probe(
    client: &reqwest::Client,
    url: &str,
) -> Result<ProbeResult, reqwest::Error> {
    let response = client
        .get(url)
        .header("Range", "bytes=0-0")
        .send()
        .await?;

    if response.status() == StatusCode::PARTIAL_CONTENT {
        let total = response
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split('/').last())
            .and_then(|v| v.parse::<u64>().ok());
        Ok(ProbeResult {
            content_length: total,
            accepts_ranges: true,
        })
    } else {
        let content_length = response.content_length();
        Ok(ProbeResult {
            content_length,
            accepts_ranges: false,
        })
    }
}

#[tracing::instrument(name = "download", skip(config, progress_tx), fields(id = id.0, %url))]
pub async fn download_task(
    id: DownloadId,
    url: String,
    destination: PathBuf,
    config: HttpDownloadConfig,
    progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
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

    // Single client shared across probe and all chunk tasks.
    // reqwest::Client pools connections internally; this enables HTTP/2 multiplexing
    // so all range requests share one TCP+TLS connection where supported.
    let client = Arc::new(reqwest::Client::new());

    // 1. Probe: GET with Range to test server capabilities
    let probe_result = match probe(&client, &url).await {
        Ok(p) => p,
        Err(_) => {
            send(DownloadStatus::Error, 0, None, 0);
            return;
        }
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
            single_download(id, client, url, destination, config.stall_timeout_secs, progress_tx).await;
            return;
        }
    };

    // 2. Create and pre-allocate file
    let file = match std::fs::File::create(&destination) {
        Ok(f) => f,
        Err(_) => {
            send(DownloadStatus::Error, 0, Some(total_bytes), 0);
            return;
        }
    };
    if file.set_len(total_bytes).is_err() {
        send(DownloadStatus::Error, 0, Some(total_bytes), 0);
        return;
    }
    let file = Arc::new(file);

    // 3. Split into chunks, extract config values before moving into spawns
    let num_chunks = if probe_result.accepts_ranges { config.max_connections } else { 1 };
    tracing::info!(total_bytes, num_chunks, "starting chunked download");
    let write_buffer_size = config.write_buffer_size;
    let progress_interval_ms = config.progress_interval_ms;
    let stall_timeout = Duration::from_secs(config.stall_timeout_secs);
    let max_retries = config.max_retries_per_chunk;
    let chunks = chunk::split(total_bytes, num_chunks);

    // 4. Shared progress: one atomic counter per chunk
    let chunk_downloaded: Arc<Vec<AtomicU64>> =
        Arc::new((0..chunks.len()).map(|_| AtomicU64::new(0)).collect());

    // 5. Slow-start: open 1 connection first, double the concurrency limit on each
    //    successful chunk completion up to num_chunks. Avoids bursting all connections
    //    at once, which often triggers 429s from CDNs that rate-limit by connection count.
    send(DownloadStatus::Downloading, 0, Some(total_bytes), 0);

    let mut pending: VecDeque<usize> = (0..chunks.len()).collect();
    let mut join_set: JoinSet<Result<(), String>> = JoinSet::new();
    let mut current_limit: usize = 1;
    let mut all_ok = true;

    // Builds the retry-loop future for chunk i. Clones Arc handles so the future is 'static.
    let make_chunk_fut = |i: usize| {
        let url = url.clone();
        let client = Arc::clone(&client);
        let file = Arc::clone(&file);
        let counters = Arc::clone(&chunk_downloaded);
        let start = chunks.starts[i];
        let end = chunks.ends[i];
        async move {
            let mut attempt = 0u32;
            loop {
                let resume_from = counters[i].load(Ordering::Relaxed);
                match download_chunk(
                    &client, &url,
                    start, end, resume_from,
                    &file, &counters, i,
                    write_buffer_size, stall_timeout,
                ).await {
                    Ok(()) => break Ok(()),
                    Err(ChunkError::Fatal(msg)) => break Err(msg),
                    Err(ChunkError::NonRetryable) => break Err("non-retryable server error".into()),
                    Err(ChunkError::Retryable { retry_after }) => {
                        if counters[i].load(Ordering::Relaxed) > resume_from {
                            attempt = 0;
                        } else {
                            attempt += 1;
                        }
                        if attempt >= max_retries {
                            tracing::error!(chunk = i, attempt, "max retries exceeded");
                            break Err("max retries exceeded".into());
                        }
                        let delay = retry_after.unwrap_or_else(|| {
                            Duration::from_secs(2u64.pow(attempt.min(5)).min(30))
                        });
                        tracing::warn!(chunk = i, attempt, delay_secs = delay.as_secs(), "retrying chunk");
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }
    };

    if let Some(i) = pending.pop_front() {
        join_set.spawn(make_chunk_fut(i));
    }

    // 6. Progress reporting loop
    let progress_handle = {
        let counters = Arc::clone(&chunk_downloaded);
        let progress_tx = progress_tx.clone();
        let started = Instant::now();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(progress_interval_ms)).await;
                let total_downloaded: u64 = counters
                    .iter()
                    .map(|a| a.load(Ordering::Relaxed))
                    .sum();
                let elapsed = started.elapsed().as_secs_f64();
                let speed = if elapsed > 0.0 {
                    (total_downloaded as f64 / elapsed) as u64
                } else {
                    0
                };
                let _ = progress_tx.send(ProgressUpdate {
                    id,
                    status: DownloadStatus::Downloading,
                    downloaded_bytes: total_downloaded,
                    total_bytes: Some(total_bytes),
                    speed_bytes_per_sec: speed,
                });
                if total_downloaded >= total_bytes {
                    break;
                }
            }
        })
    };

    // 7. Drain completed chunks; ramp up limit on success, fill to new limit from pending
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(())) => {
                current_limit = (current_limit * 2).min(num_chunks);
            }
            _ => {
                all_ok = false;
            }
        }
        while join_set.len() < current_limit {
            if let Some(i) = pending.pop_front() {
                join_set.spawn(make_chunk_fut(i));
            } else {
                break;
            }
        }
    }

    progress_handle.abort();
    drop(file);

    if all_ok {
        tracing::info!(total_bytes, "download finished");
        send(DownloadStatus::Finished, total_bytes, Some(total_bytes), 0);
    } else {
        let total_downloaded: u64 = chunk_downloaded
            .iter()
            .map(|a| a.load(Ordering::Relaxed))
            .sum();
        tracing::error!(total_downloaded, total_bytes, "download failed");
        send(DownloadStatus::Error, total_downloaded, Some(total_bytes), 0);
    }
}

async fn download_chunk(
    client: &reqwest::Client,
    url: &str,
    chunk_start: u64,
    chunk_end: u64,
    resume_from: u64,
    file: &std::fs::File,
    counters: &[AtomicU64],
    index: usize,
    write_buffer_size: usize,
    stall_timeout: Duration,
) -> Result<(), ChunkError> {
    // Resume from where the last attempt left off
    let byte_start = chunk_start + resume_from;
    let range = format!("bytes={}-{}", byte_start, chunk_end - 1);

    let response = client
        .get(url)
        .header("Range", range)
        .send()
        .await
        .map_err(|_| ChunkError::Retryable { retry_after: None })?;

    if !response.status().is_success() {
        return Err(classify_status(response.status(), response.headers()));
    }

    let mut stream = response.bytes_stream();
    let mut offset = byte_start;
    let mut buffer = Vec::with_capacity(write_buffer_size);

    loop {
        match tokio::time::timeout(stall_timeout, stream.next()).await {
            Err(_elapsed) => return Err(ChunkError::Retryable { retry_after: None }),
            Ok(None) => break,
            Ok(Some(Err(_))) => return Err(ChunkError::Retryable { retry_after: None }),
            Ok(Some(Ok(bytes))) => {
                buffer.extend_from_slice(&bytes);
                if buffer.len() >= write_buffer_size {
                    write_at(file, &buffer, offset).map_err(classify_io_error)?;
                    offset += buffer.len() as u64;
                    counters[index].fetch_add(buffer.len() as u64, Ordering::Relaxed);
                    buffer.clear();
                }
            }
        }
    }

    if !buffer.is_empty() {
        write_at(file, &buffer, offset).map_err(classify_io_error)?;
        counters[index].fetch_add(buffer.len() as u64, Ordering::Relaxed);
    }

    Ok(())
}

/// Fallback for servers that don't report Content-Length.
/// Uses tokio::fs for async sequential writes since there's only one writer.
/// No retry — unknown-length streams can't be resumed to a byte offset.
async fn single_download(
    id: DownloadId,
    client: Arc<reqwest::Client>,
    url: String,
    destination: PathBuf,
    stall_timeout_secs: u64,
    progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
) {
    let stall_timeout = Duration::from_secs(stall_timeout_secs);

    let send = |status: DownloadStatus, downloaded: u64, total: Option<u64>, speed: u64| {
        let _ = progress_tx.send(ProgressUpdate {
            id,
            status,
            downloaded_bytes: downloaded,
            total_bytes: total,
            speed_bytes_per_sec: speed,
        });
    };

    let response = match client.get(&url).send().await {
        Ok(r) => r,
        Err(_) => { send(DownloadStatus::Error, 0, None, 0); return; }
    };

    let mut file = match tokio::fs::File::create(&destination).await {
        Ok(f) => f,
        Err(_) => { send(DownloadStatus::Error, 0, None, 0); return; }
    };

    let mut downloaded: u64 = 0;
    let started = Instant::now();
    let mut stream = response.bytes_stream();

    send(DownloadStatus::Downloading, 0, None, 0);

    loop {
        match tokio::time::timeout(stall_timeout, stream.next()).await {
            Err(_) | Ok(Some(Err(_))) => {
                send(DownloadStatus::Error, downloaded, None, 0);
                return;
            }
            Ok(None) => break,
            Ok(Some(Ok(chunk))) => {
                if tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await.is_err() {
                    send(DownloadStatus::Error, downloaded, None, 0);
                    return;
                }
                downloaded += chunk.len() as u64;
                let elapsed = started.elapsed().as_secs_f64();
                let speed = if elapsed > 0.0 { (downloaded as f64 / elapsed) as u64 } else { 0 };
                send(DownloadStatus::Downloading, downloaded, None, speed);
            }
        }
    }

    send(DownloadStatus::Finished, downloaded, None, 0);
}

#[cfg(unix)]
fn write_at(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
fn write_at(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    file.seek_write(buf, offset)?;
    Ok(())
}
