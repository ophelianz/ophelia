use std::path::PathBuf;
use std::time::Instant;

use futures::StreamExt;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use crate::engine::types::{DownloadId, DownloadStatus, ProgressUpdate};

pub async fn download_task(
    id: DownloadId,
    url: String,
    destination: PathBuf,
    progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
) {
    let send = |status, downloaded, total, speed| {
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

    let total_bytes = response.content_length();

    let mut file = match File::create(&destination).await {
        Ok(f) => f,
        Err(_) => {
            send(DownloadStatus::Error, 0, total_bytes, 0);
            return;
        }
    };

    let mut downloaded: u64 = 0;
    let started = Instant::now();
    let mut stream = response.bytes_stream();

    send(DownloadStatus::Downloading, 0, total_bytes, 0);

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(_) => {
                send(DownloadStatus::Error, downloaded, total_bytes, 0);
                return;
            }
        };

        if file.write_all(&chunk).await.is_err() {
            send(DownloadStatus::Error, downloaded, total_bytes, 0);
            return;
        }

        downloaded += chunk.len() as u64;
        let elapsed = started.elapsed().as_secs_f64();
        let speed = if elapsed > 0.0 {
            (downloaded as f64 / elapsed) as u64
        } else {
            0
        };

        send(DownloadStatus::Downloading, downloaded, total_bytes, speed);
    }

    send(DownloadStatus::Finished, downloaded, total_bytes, 0);
}
