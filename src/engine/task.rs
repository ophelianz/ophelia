use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use tokio::sync::mpsc;

use crate::engine::chunk;
use crate::engine::types::{DownloadId, DownloadStatus, ProgressUpdate};

const DEFAULT_CHUNKS: usize = 8;

struct ProbeResult {
    content_length: Option<u64>,
    accepts_ranges: bool,
}

async fn probe(url: &str) -> Result<ProbeResult, reqwest::Error> {
    let response = reqwest::Client::new().head(url).send().await?;
    let content_length = response.content_length();
    let accepts_ranges = response
        .headers()
        .get("accept-ranges")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "bytes")
        .unwrap_or(false);
    Ok(ProbeResult {
        content_length,
        accepts_ranges,
    })
}

pub async fn download_task(
    id: DownloadId,
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

    // 1. Probe: HEAD request for size and range support
    let probe_result = match probe(&url).await {
        Ok(p) => p,
        Err(_) => {
            send(DownloadStatus::Error, 0, None, 0);
            return;
        }
    };

    let total_bytes = match probe_result.content_length {
        Some(len) => len,
        None => {
            // Unknown size, single stream fallback
            single_download(id, url, destination, progress_tx).await;
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
        DEFAULT_CHUNKS
    } else {
        1
    };
    let chunks = chunk::split(total_bytes, num_chunks);

    // 4. Shared progress: one atomic counter per chunk
    let chunk_downloaded: Arc<Vec<AtomicU64>> =
        Arc::new((0..chunks.len()).map(|_| AtomicU64::new(0)).collect());

    // 5. Spawn chunk download tasks
    send(DownloadStatus::Downloading, 0, Some(total_bytes), 0);
    let started = Instant::now();

    let mut handles = Vec::with_capacity(chunks.len());
    for i in 0..chunks.len() {
        let url = url.clone();
        let file = Arc::clone(&file);
        let counters = Arc::clone(&chunk_downloaded);
        let start = chunks.starts[i];
        let end = chunks.ends[i];

        let handle = tokio::spawn(async move {
            download_chunk(i, &url, start, end, &file, &counters).await
        });
        handles.push(handle);
    }

    // 6. Progress reporting loop: poll atomics every 100ms
    let progress_handle = {
        let counters = Arc::clone(&chunk_downloaded);
        let progress_tx = progress_tx.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
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
    index: usize,
    url: &str,
    start: u64,
    end: u64,
    file: &std::fs::File,
    counters: &[AtomicU64],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let range = format!("bytes={}-{}", start, end - 1);
    let response = client.get(url).header("Range", range).send().await?;

    let mut stream = response.bytes_stream();
    let mut offset = start;

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        write_at(file, &bytes, offset)?;
        offset += bytes.len() as u64;
        counters[index].fetch_add(bytes.len() as u64, Ordering::Relaxed);
    }

    Ok(())
}

/// Fallback for servers that don't report Content-Length
async fn single_download(
    id: DownloadId,
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

    let response = match reqwest::get(&url).await {
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
