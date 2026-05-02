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
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::disk::{
    DiskLease, DiskSessionLease, DiskWriteFailure, DiskWriteJob, DiskWriteResult, DiskWriteSender,
    DiskWriter,
};
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
    pub(super) disk: DiskLease,
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
        disk,
        stall_timeout_secs,
        runtime_update_tx,
        pause_token,
        throttle,
    } = request;
    let stall_timeout = Duration::from_secs(stall_timeout_secs);
    let disk_lease = disk;
    let session = disk_lease.session();

    let response = tokio::select! {
        biased;
        _ = pause_token.cancelled() => {
            disk_lease.mark_failed(None);
            send_progress(&runtime_update_tx, id, TransferStatus::Error, 0, None, 0).await;
            return task_state(TransferStatus::Error, 0, None);
        }
        result = client.get(&url).send() => match result {
            Ok(r) => r,
            Err(_) => {
                disk_lease.mark_failed(None);
                send_progress(&runtime_update_tx, id, TransferStatus::Error, 0, None, 0).await;
                return task_state(TransferStatus::Error, 0, None);
            }
        }
    };
    if !response.status().is_success() {
        tracing::warn!(status = %response.status(), "single-stream request failed");
        disk_lease.mark_failed(None);
        send_progress(&runtime_update_tx, id, TransferStatus::Error, 0, None, 0).await;
        return task_state(TransferStatus::Error, 0, None);
    }

    let (disk, writer) = disk_lease.split_for_writes::<TransferId>();
    let mut writer = Some(writer);
    let write_jobs = writer.as_ref().unwrap().sender();

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
                return fail_after_writer(
                    &runtime_update_tx,
                    id,
                    downloaded,
                    disk,
                    writer.take().unwrap(),
                    write_jobs,
                    None,
                ).await;
            }
            result = tokio::time::timeout(stall_timeout, stream.next()) => result,
        };
        let Ok(maybe) = result else {
            return fail_after_writer(
                &runtime_update_tx,
                id,
                downloaded,
                disk,
                writer.take().unwrap(),
                write_jobs,
                None,
            )
            .await;
        };
        let Some(item) = maybe else { break };
        let Ok(chunk) = item else {
            return fail_after_writer(
                &runtime_update_tx,
                id,
                downloaded,
                disk,
                writer.take().unwrap(),
                write_jobs,
                None,
            )
            .await;
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        let job = DiskWriteJob::new(session, id, downloaded, chunk.to_vec(), reply_tx);
        if write_jobs.send(job).await.is_err() {
            return fail_after_writer(
                &runtime_update_tx,
                id,
                downloaded,
                disk,
                writer.take().unwrap(),
                write_jobs,
                Some("disk writer stopped".into()),
            )
            .await;
        }
        match reply_rx.await {
            Ok(DiskWriteResult::Written { range, .. }) => {
                disk.confirm_logical(range.len());
                let _ = runtime_update_tx
                    .send(TaskRuntimeUpdate::TransferBytesWritten {
                        id,
                        bytes: range.len(),
                    })
                    .await;
                downloaded = range.end();
                window_bytes += range.len();
            }
            Ok(DiskWriteResult::Failed { failure, .. }) => {
                tracing::warn!(failure = %disk_failure_label(&failure), "single-stream disk write failed");
                return fail_after_writer(
                    &runtime_update_tx,
                    id,
                    downloaded,
                    disk,
                    writer.take().unwrap(),
                    write_jobs,
                    Some(disk_failure_message(failure)),
                )
                .await;
            }
            Err(_closed) => {
                return fail_after_writer(
                    &runtime_update_tx,
                    id,
                    downloaded,
                    disk,
                    writer.take().unwrap(),
                    write_jobs,
                    Some("disk writer dropped write result".into()),
                )
                .await;
            }
        }
        let wait = throttle.consume(chunk.len() as u64);
        if !wait.is_zero() {
            tokio::select! {
                biased;
                _ = pause_token.cancelled() => {
                    return fail_after_writer(
                        &runtime_update_tx,
                        id,
                        downloaded,
                        disk,
                        writer.take().unwrap(),
                        write_jobs,
                        None,
                    ).await;
                }
                _ = tokio::time::sleep(wait) => {}
            }
        }
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

    drop(write_jobs);
    for result in writer.take().unwrap().shutdown().await {
        if let DiskWriteResult::Failed { failure, .. } = result {
            disk.mark_failed(Some(disk_failure_message(failure)));
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
    }

    match disk.commit() {
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

async fn fail_after_writer(
    runtime_update_tx: &mpsc::Sender<TaskRuntimeUpdate>,
    id: TransferId,
    downloaded: u64,
    disk: DiskSessionLease,
    writer: DiskWriter<TransferId>,
    write_jobs: DiskWriteSender<TransferId>,
    message: Option<String>,
) -> TaskFinalState {
    drop(write_jobs);
    let _ = writer.shutdown().await;
    disk.mark_failed(message);
    send_progress(
        runtime_update_tx,
        id,
        TransferStatus::Error,
        downloaded,
        None,
        0,
    )
    .await;
    task_state(TransferStatus::Error, downloaded, None)
}

fn disk_failure_message(failure: DiskWriteFailure) -> String {
    match failure {
        DiskWriteFailure::FatalIo { message } | DiskWriteFailure::RetryableIo { message } => {
            message
        }
    }
}

fn disk_failure_label(failure: &DiskWriteFailure) -> &'static str {
    match failure {
        DiskWriteFailure::FatalIo { .. } => "fatal_io",
        DiskWriteFailure::RetryableIo { .. } => "retryable_io",
    }
}
