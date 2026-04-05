/***************************************************
** This file is part of Ophelia, distributed under the
** terms of the GPL License, version 3 or later.
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs, do no evil and behave plz )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

//! Single-stream fallback for servers that don't support range requests or
//! don't send Content-Length.
//! Just stream to disk

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio::sync::mpsc;

use crate::engine::destination::{ResolvedDestination, finalize_part_file};
use crate::engine::http::throttle::Throttle;
use crate::engine::types::{DownloadId, DownloadStatus, ProgressUpdate};

use super::task::TaskFinalState;

const EMA_ALPHA: f64 = 0.3;
const WINDOW_SECS: f64 = 2.0;

fn task_state(
    status: DownloadStatus,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) -> TaskFinalState {
    TaskFinalState {
        status,
        downloaded_bytes,
        total_bytes,
    }
}

pub async fn single_download(
    id: DownloadId,
    client: Arc<reqwest::Client>,
    url: String,
    resolved_destination: ResolvedDestination,
    stall_timeout_secs: u64,
    progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
    throttle: Arc<Throttle>,
) -> TaskFinalState {
    let ResolvedDestination {
        part_path,
        destination,
        finalize_strategy,
    } = resolved_destination;
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
        Err(_) => {
            send(DownloadStatus::Error, 0, None, 0);
            return task_state(DownloadStatus::Error, 0, None);
        }
    };

    let mut file = match tokio::fs::File::create(&part_path).await {
        Ok(f) => f,
        Err(_) => {
            send(DownloadStatus::Error, 0, None, 0);
            return task_state(DownloadStatus::Error, 0, None);
        }
    };

    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();
    let mut ema_speed: f64 = 0.0;
    let mut window_start = Instant::now();
    let mut window_bytes: u64 = 0;

    send(DownloadStatus::Downloading, 0, None, 0);

    loop {
        let result = tokio::time::timeout(stall_timeout, stream.next()).await;
        let Ok(maybe) = result else {
            send(DownloadStatus::Error, downloaded, None, 0);
            return task_state(DownloadStatus::Error, downloaded, None);
        };
        let Some(item) = maybe else { break };
        let Ok(chunk) = item else {
            send(DownloadStatus::Error, downloaded, None, 0);
            return task_state(DownloadStatus::Error, downloaded, None);
        };

        if tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .is_err()
        {
            send(DownloadStatus::Error, downloaded, None, 0);
            return task_state(DownloadStatus::Error, downloaded, None);
        }
        let wait = throttle.consume(chunk.len() as u64);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        downloaded += chunk.len() as u64;
        window_bytes += chunk.len() as u64;
        let elapsed = window_start.elapsed().as_secs_f64();
        if elapsed >= WINDOW_SECS {
            let recent = window_bytes as f64 / elapsed;
            ema_speed = (1.0 - EMA_ALPHA) * ema_speed + EMA_ALPHA * recent;
            window_bytes = 0;
            window_start = Instant::now();
        }
        send(
            DownloadStatus::Downloading,
            downloaded,
            None,
            ema_speed as u64,
        );
    }

    drop(file);
    match finalize_part_file(&part_path, &destination, finalize_strategy) {
        Ok(()) => {
            send(DownloadStatus::Finished, downloaded, None, 0);
            task_state(DownloadStatus::Finished, downloaded, None)
        }
        Err(e) => {
            tracing::error!(err = %e, "rename failed after single download");
            send(DownloadStatus::Error, downloaded, None, 0);
            task_state(DownloadStatus::Error, downloaded, None)
        }
    }
}
