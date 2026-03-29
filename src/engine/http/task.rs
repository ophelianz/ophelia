//! HTTP/HTTPS download pipeline.
//!
//! Probes server capabilities via GET+Range, then either drives parallel
//! chunked range requests (206) or falls back to a single stream (200).
//! Progress is tracked via per-chunk atomics; the timer starts after chunks
//! are spawned to exclude probe and allocation time from speed calculations.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use tokio::sync::mpsc;

use crate::engine::chunk;
use crate::engine::http::HttpDownloadConfig;
use crate::engine::types::{DownloadId, DownloadStatus, ProgressUpdate};

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

    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
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

    let total_bytes = match probe_result.content_length {
        Some(len) => len,
        None => {
            single_download(id, client, url, destination, progress_tx).await;
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

    // 3. Split into chunks
    let num_chunks = if probe_result.accepts_ranges {
        config.max_connections
    } else {
        1
    };
    let write_buffer_size = config.write_buffer_size;
    let progress_interval_ms = config.progress_interval_ms;
    let chunks = chunk::split(total_bytes, num_chunks);

    // 4. Shared progress: one atomic counter per chunk
    let chunk_downloaded: Arc<Vec<AtomicU64>> =
        Arc::new((0..chunks.len()).map(|_| AtomicU64::new(0)).collect());

    // 5. Spawn chunk download tasks
    send(DownloadStatus::Downloading, 0, Some(total_bytes), 0);

    let mut handles = Vec::with_capacity(chunks.len());
    for i in 0..chunks.len() {
        let url = url.clone();
        let client = Arc::clone(&client);
        let file = Arc::clone(&file);
        let counters = Arc::clone(&chunk_downloaded);
        let start = chunks.starts[i];
        let end = chunks.ends[i];

        let handle = tokio::spawn(async move {
            download_chunk(&client, &url, start, end, &file, &counters, i, write_buffer_size).await
        });
        handles.push(handle);
    }

    // 6. Progress reporting loop
    let progress_handle = {
        let counters = Arc::clone(&chunk_downloaded);
        let progress_tx = progress_tx.clone();
        let started = Instant::now();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(progress_interval_ms)).await;
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

    // 7. Wait for all chunks
    let mut all_ok = true;
    for handle in handles {
        match handle.await {
            Ok(Ok(())) => {}
            _ => all_ok = false,
        }
    }

    progress_handle.abort();
    drop(file);

    if all_ok {
        send(DownloadStatus::Finished, total_bytes, Some(total_bytes), 0);
    } else {
        let total_downloaded: u64 = chunk_downloaded
            .iter()
            .map(|a| a.load(Ordering::Relaxed))
            .sum();
        send(
            DownloadStatus::Error,
            total_downloaded,
            Some(total_bytes),
            0,
        );
    }
}

async fn download_chunk(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
    file: &std::fs::File,
    counters: &[AtomicU64],
    index: usize,
    write_buffer_size: usize,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let range = format!("bytes={}-{}", start, end - 1);
    let response = client.get(url).header("Range", range).send().await?;

    let mut stream = response.bytes_stream();
    let mut offset = start;
    let mut buffer = Vec::with_capacity(write_buffer_size);

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        buffer.extend_from_slice(&bytes);

        if buffer.len() >= write_buffer_size {
            write_at(file, &buffer, offset)?;
            offset += buffer.len() as u64;
            counters[index].fetch_add(buffer.len() as u64, Ordering::Relaxed);
            buffer.clear();
        }
    }

    // Flush remaining bytes
    if !buffer.is_empty() {
        write_at(file, &buffer, offset)?;
        counters[index].fetch_add(buffer.len() as u64, Ordering::Relaxed);
    }

    Ok(())
}

/// Fallback for servers that don't report Content-Length.
/// Uses tokio::fs for async sequential writes since there's only one writer.
async fn single_download(
    id: DownloadId,
    client: Arc<reqwest::Client>,
    url: String,
    destination: PathBuf,
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

    let response = match client.get(&url).send().await {
        Ok(r) => r,
        Err(_) => {
            send(DownloadStatus::Error, 0, None, 0);
            return;
        }
    };

    let mut file = match tokio::fs::File::create(&destination).await {
        Ok(f) => f,
        Err(_) => {
            send(DownloadStatus::Error, 0, None, 0);
            return;
        }
    };

    let mut downloaded: u64 = 0;
    let started = Instant::now();
    let mut stream = response.bytes_stream();

    send(DownloadStatus::Downloading, 0, None, 0);

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(_) => {
                send(DownloadStatus::Error, downloaded, None, 0);
                return;
            }
        };

        if tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .is_err()
        {
            send(DownloadStatus::Error, downloaded, None, 0);
            return;
        }

        downloaded += chunk.len() as u64;
        let elapsed = started.elapsed().as_secs_f64();
        let speed = if elapsed > 0.0 {
            (downloaded as f64 / elapsed) as u64
        } else {
            0
        };

        send(DownloadStatus::Downloading, downloaded, None, speed);
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
