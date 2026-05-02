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

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

use ophelia::engine::DestinationPolicyConfig;
use ophelia::engine::destination::DestinationPolicy;
use ophelia::engine::http::{DownloadTaskRequest, HttpDownloadConfig, TokenBucket};
use ophelia::engine::types::{
    ChunkSnapshot, ProgressUpdate, RunnerEvent, TransferId, TransferStatus,
};

pub fn unlimited_semaphore() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(Semaphore::MAX_PERMITS))
}

pub fn unlimited_throttle() -> Arc<TokenBucket> {
    Arc::new(TokenBucket::new(0))
}

pub fn exact_destination_policy(destination: &std::path::Path) -> DestinationPolicy {
    DestinationPolicy::for_resolved_destination(&DestinationPolicyConfig::default(), destination)
}

#[allow(clippy::too_many_arguments)]
pub async fn download_task(
    id: TransferId,
    url: String,
    destination: std::path::PathBuf,
    destination_policy: DestinationPolicy,
    config: HttpDownloadConfig,
    pause_token: CancellationToken,
    pause_sink: Arc<Mutex<Option<Vec<ChunkSnapshot>>>>,
    destination_sink: Arc<Mutex<Option<std::path::PathBuf>>>,
    resume_from: Option<Vec<ChunkSnapshot>>,
    server_semaphore: Arc<Semaphore>,
    global_throttle: Arc<TokenBucket>,
    runtime_update_tx: mpsc::Sender<RunnerEvent>,
) -> ophelia::engine::http::TaskFinalState {
    ophelia::engine::http::download_task(DownloadTaskRequest::new(
        id,
        url,
        destination,
        destination_policy,
        config,
        pause_token,
        pause_sink,
        destination_sink,
        resume_from,
        server_semaphore,
        global_throttle,
        runtime_update_tx,
    ))
    .await
}

pub fn runtime_updates_channel() -> (mpsc::Sender<RunnerEvent>, mpsc::Receiver<RunnerEvent>) {
    mpsc::channel(256)
}

pub fn test_data(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 256) as u8).collect()
}

pub fn sha256(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

pub async fn drain_progress(rx: &mut mpsc::Receiver<RunnerEvent>) -> Vec<ProgressUpdate> {
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut updates = vec![];
    while let Ok(update) = rx.try_recv() {
        if let RunnerEvent::Progress(update) = update {
            updates.push(update);
        }
    }
    updates
}

pub async fn drain_runtime_updates(rx: &mut mpsc::Receiver<RunnerEvent>) -> Vec<RunnerEvent> {
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut updates = vec![];
    while let Ok(update) = rx.try_recv() {
        updates.push(update);
    }
    updates
}

pub fn progress_updates(updates: &[RunnerEvent]) -> Vec<ProgressUpdate> {
    updates
        .iter()
        .filter_map(|update| match update {
            RunnerEvent::Progress(progress) => Some(progress.clone()),
            _ => None,
        })
        .collect()
}

pub fn download_write_bytes_from(updates: &[RunnerEvent]) -> u64 {
    updates.iter().fold(0_u64, |total, update| match update {
        RunnerEvent::TransferBytesWritten { bytes, .. } => total.saturating_add(*bytes),
        _ => total,
    })
}

pub fn last_status(updates: &[ProgressUpdate]) -> Option<TransferStatus> {
    updates.last().map(|u| u.status)
}

pub async fn wait_for_runtime_update(
    rx: &mut mpsc::Receiver<RunnerEvent>,
    mut predicate: impl FnMut(&RunnerEvent) -> bool,
) -> RunnerEvent {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(update) = rx.try_recv()
            && predicate(&update)
        {
            return update;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for matching runtime update"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub fn drain_download_write_bytes(rx: &mut mpsc::Receiver<RunnerEvent>) -> u64 {
    let mut total = 0_u64;
    while let Ok(update) = rx.try_recv() {
        if let RunnerEvent::TransferBytesWritten { bytes, .. } = update {
            total = total.saturating_add(bytes);
        }
    }
    total
}

pub async fn spawn_hedge_range_server(
    data: Vec<u8>,
    second_half_requests: Arc<AtomicUsize>,
    fail_second_half_hedge: bool,
    delay_first_second_half: Duration,
) -> std::net::SocketAddr {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let data = data.clone();
            let second_half_requests = Arc::clone(&second_half_requests);
            tokio::spawn(async move {
                let mut request = vec![0u8; 4096];
                let Ok(read) = socket.read(&mut request).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&request[..read]);
                let Some((start, end)) = parse_range_header(&request) else {
                    let _ = socket
                        .write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
                        .await;
                    return;
                };

                let half = data.len() / 2;
                let second_half_request =
                    (start >= half).then(|| second_half_requests.fetch_add(1, Ordering::Relaxed));
                if second_half_request == Some(1) && fail_second_half_hedge {
                    let _ = socket
                        .write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n")
                        .await;
                    return;
                }
                if second_half_request == Some(0) {
                    tokio::time::sleep(delay_first_second_half).await;
                }

                let end = end.min(data.len());
                let body = &data[start..end];
                let header = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                    body.len(),
                    start,
                    end.saturating_sub(1),
                    data.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(body).await;
            });
        }
    });
    addr
}

fn parse_range_header(request: &str) -> Option<(usize, usize)> {
    let line = request
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("range:"))?;
    let range = line.split_once("bytes=")?.1.trim();
    let (start, end) = range.split_once('-')?;
    let start = start.parse::<usize>().ok()?;
    let end = end.parse::<usize>().ok()?.checked_add(1)?;
    Some((start, end))
}

// ---------------------------------------------------------------------------
// Wiremock responders
// ---------------------------------------------------------------------------

use wiremock::{Respond, ResponseTemplate};

#[allow(dead_code)] // shared test helper used selectively by integration test targets
pub struct RangeResponder {
    pub data: Vec<u8>,
}

impl Respond for RangeResponder {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let range_header = request
            .headers
            .get("range")
            .expect("missing range header")
            .to_str()
            .unwrap();
        let range = range_header.strip_prefix("bytes=").unwrap();
        let parts: Vec<&str> = range.split('-').collect();
        let start: usize = parts[0].parse().unwrap();
        let end: usize = parts[1].parse::<usize>().unwrap() + 1;
        let content_range = format!("bytes {}-{}/{}", start, end - 1, self.data.len());
        ResponseTemplate::new(206)
            .set_body_bytes(self.data[start..end].to_vec())
            .insert_header("content-range", content_range.as_str())
    }
}

#[allow(dead_code)] // shared test helper used selectively by integration test targets
pub struct SlowRangeResponder {
    pub data: Vec<u8>,
    pub delay: Duration,
}

impl Respond for SlowRangeResponder {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let range_header = request
            .headers
            .get("range")
            .expect("missing range header")
            .to_str()
            .unwrap();
        let range = range_header.strip_prefix("bytes=").unwrap();
        let parts: Vec<&str> = range.split('-').collect();
        let start: usize = parts[0].parse().unwrap();
        let end: usize = parts[1].parse::<usize>().unwrap() + 1;
        let content_range = format!("bytes {}-{}/{}", start, end - 1, self.data.len());
        ResponseTemplate::new(206)
            .set_delay(self.delay)
            .set_body_bytes(self.data[start..end].to_vec())
            .insert_header("content-range", content_range.as_str())
    }
}
