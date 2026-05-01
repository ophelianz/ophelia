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

//! Single-stream fallback
//! Used when the server does not support ranges or does not send Content-Length

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::engine::destination::{ResolvedDestination, finalize_part_file};
use crate::engine::http::throttle::Throttle;
use crate::engine::types::{ProgressUpdate, TaskRuntimeUpdate, TransferId, TransferStatus};

use super::task::TaskFinalState;

const EMA_ALPHA: f64 = 0.3;
const WINDOW_SECS: f64 = 2.0;

fn task_state(
    status: TransferStatus,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) -> TaskFinalState {
    TaskFinalState {
        status,
        downloaded_bytes,
        total_bytes,
    }
}

pub(super) struct SingleTransferRequest {
    pub(super) id: TransferId,
    pub(super) client: Arc<reqwest::Client>,
    pub(super) url: String,
    pub(super) resolved_destination: ResolvedDestination,
    pub(super) stall_timeout_secs: u64,
    pub(super) runtime_update_tx: mpsc::Sender<TaskRuntimeUpdate>,
    pub(super) pause_token: CancellationToken,
    pub(super) throttle: Arc<Throttle>,
}

async fn send_progress(
    runtime_update_tx: &mpsc::Sender<TaskRuntimeUpdate>,
    id: TransferId,
    status: TransferStatus,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    speed_bytes_per_sec: u64,
) {
    let _ = runtime_update_tx
        .send(TaskRuntimeUpdate::Progress(ProgressUpdate {
            id,
            status,
            downloaded_bytes,
            total_bytes,
            speed_bytes_per_sec,
        }))
        .await;
}

pub async fn single_download(request: SingleTransferRequest) -> TaskFinalState {
    let SingleTransferRequest {
        id,
        client,
        url,
        resolved_destination,
        stall_timeout_secs,
        runtime_update_tx,
        pause_token,
        throttle,
    } = request;
    let ResolvedDestination {
        part_path,
        destination,
        finalize_strategy,
    } = resolved_destination;
    let stall_timeout = Duration::from_secs(stall_timeout_secs);

    let response = tokio::select! {
        biased;
        _ = pause_token.cancelled() => {
            send_progress(&runtime_update_tx, id, TransferStatus::Error, 0, None, 0).await;
            return task_state(TransferStatus::Error, 0, None);
        }
        result = client.get(&url).send() => match result {
            Ok(r) => r,
            Err(_) => {
                send_progress(&runtime_update_tx, id, TransferStatus::Error, 0, None, 0).await;
                return task_state(TransferStatus::Error, 0, None);
            }
        }
    };
    if !response.status().is_success() {
        tracing::warn!(status = %response.status(), "single-stream request failed");
        send_progress(&runtime_update_tx, id, TransferStatus::Error, 0, None, 0).await;
        return task_state(TransferStatus::Error, 0, None);
    }

    let mut file = match tokio::fs::File::create(&part_path).await {
        Ok(f) => f,
        Err(_) => {
            send_progress(&runtime_update_tx, id, TransferStatus::Error, 0, None, 0).await;
            return task_state(TransferStatus::Error, 0, None);
        }
    };

    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();
    let mut ema_speed: f64 = 0.0;
    let mut window_start = Instant::now();
    let mut window_bytes: u64 = 0;

    send_progress(
        &runtime_update_tx,
        id,
        TransferStatus::Downloading,
        0,
        None,
        0,
    )
    .await;

    loop {
        let result = tokio::select! {
            biased;
            _ = pause_token.cancelled() => {
                send_progress(&runtime_update_tx, id, TransferStatus::Error, downloaded, None, 0).await;
                return task_state(TransferStatus::Error, downloaded, None);
            }
            result = tokio::time::timeout(stall_timeout, stream.next()) => result,
        };
        let Ok(maybe) = result else {
            send_progress(
                &runtime_update_tx,
                id,
                TransferStatus::Error,
                downloaded,
                None,
                0,
            )
            .await;
            return task_state(TransferStatus::Error, downloaded, None);
        };
        let Some(item) = maybe else { break };
        let Ok(chunk) = item else {
            send_progress(
                &runtime_update_tx,
                id,
                TransferStatus::Error,
                downloaded,
                None,
                0,
            )
            .await;
            return task_state(TransferStatus::Error, downloaded, None);
        };

        if file.write_all(&chunk).await.is_err() {
            send_progress(
                &runtime_update_tx,
                id,
                TransferStatus::Error,
                downloaded,
                None,
                0,
            )
            .await;
            return task_state(TransferStatus::Error, downloaded, None);
        }
        let _ = runtime_update_tx
            .send(TaskRuntimeUpdate::TransferBytesWritten {
                id,
                bytes: chunk.len() as u64,
            })
            .await;
        let wait = throttle.consume(chunk.len() as u64);
        if !wait.is_zero() {
            tokio::select! {
                biased;
                _ = pause_token.cancelled() => {
                    send_progress(&runtime_update_tx, id, TransferStatus::Error, downloaded, None, 0).await;
                    return task_state(TransferStatus::Error, downloaded, None);
                }
                _ = tokio::time::sleep(wait) => {}
            }
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
        send_progress(
            &runtime_update_tx,
            id,
            TransferStatus::Downloading,
            downloaded,
            None,
            ema_speed as u64,
        )
        .await;
    }

    if file.flush().await.is_err() {
        send_progress(
            &runtime_update_tx,
            id,
            TransferStatus::Error,
            downloaded,
            None,
            0,
        )
        .await;
        return task_state(TransferStatus::Error, downloaded, None);
    }
    drop(file);
    match finalize_part_file(&part_path, &destination, finalize_strategy) {
        Ok(()) => {
            send_progress(
                &runtime_update_tx,
                id,
                TransferStatus::Finished,
                downloaded,
                None,
                0,
            )
            .await;
            task_state(TransferStatus::Finished, downloaded, None)
        }
        Err(e) => {
            tracing::error!(err = %e, "rename failed after single download");
            send_progress(
                &runtime_update_tx,
                id,
                TransferStatus::Error,
                downloaded,
                None,
                0,
            )
            .await;
            task_state(TransferStatus::Error, downloaded, None)
        }
    }
}
