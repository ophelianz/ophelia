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

mod common;
use common::*;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Respond, ResponseTemplate};

use ophelia::engine::http::HttpDownloadConfig;
use ophelia::engine::types::{ChunkSnapshot, DownloadId, DownloadStatus, HttpResumeData};

#[tokio::test(flavor = "multi_thread")]
async fn ranged_worker_rejects_200_ok_when_server_ignores_partial_range() {
    let data = test_data(64 * 1024);
    let requests = Arc::new(AtomicUsize::new(0));

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(ProbeThenIgnoreRangeResponder {
            data: data.clone(),
            requests: Arc::clone(&requests),
        })
        .mount(&server)
        .await;

    let url = format!("{}/file.bin", server.uri());
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");
    let (runtime_tx, mut runtime_rx) = runtime_updates_channel();
    download_task(
        DownloadId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        HttpDownloadConfig {
            min_connections: 2,
            max_connections: 2,
            ..HttpDownloadConfig::default()
        },
        CancellationToken::new(),
        Arc::new(Mutex::new(None)),
        Arc::new(Mutex::new(None)),
        None,
        unlimited_semaphore(),
        unlimited_throttle(),
        runtime_tx,
    )
    .await;

    let updates = drain_progress(&mut runtime_rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Error));
    assert!(
        !dest.exists(),
        "ignored range response should not produce a committed file"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_never_exceeds_total_during_hedged_ranges() {
    let data = test_data(32 * 1024);
    let second_half_requests = Arc::new(AtomicUsize::new(0));

    let server = spawn_hedge_range_server(
        data.clone(),
        Arc::clone(&second_half_requests),
        false,
        Duration::from_millis(800),
    )
    .await;

    let url = format!("http://{server}/file.bin");
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (runtime_tx, mut runtime_rx) = runtime_updates_channel();
    download_task(
        DownloadId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        HttpDownloadConfig {
            min_connections: 2,
            max_connections: 2,
            min_steal_bytes: 10 * 1024,
            progress_interval_ms: 10,
            ..HttpDownloadConfig::default()
        },
        CancellationToken::new(),
        Arc::new(Mutex::new(None)),
        Arc::new(Mutex::new(None)),
        None,
        unlimited_semaphore(),
        unlimited_throttle(),
        runtime_tx,
    )
    .await;

    let updates = drain_progress(&mut runtime_rx).await;
    assert!(
        second_half_requests.load(Ordering::Relaxed) >= 2,
        "test did not exercise the hedge path"
    );
    assert!(
        updates
            .iter()
            .all(|update| update.downloaded_bytes <= data.len() as u64),
        "progress updates must never report more bytes than the file size"
    );
}

#[test]
fn old_http_resume_total_size_is_not_the_last_extra_slot() {
    let resume = HttpResumeData::new(vec![
        ChunkSnapshot {
            start: 0,
            end: 100,
            downloaded: 100,
        },
        ChunkSnapshot {
            start: 100,
            end: 200,
            downloaded: 40,
        },
        ChunkSnapshot {
            start: 120,
            end: 160,
            downloaded: 0,
        },
    ]);

    assert_eq!(resume.total_bytes(), Some(200));
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_after_delays_range_retry() {
    let data = test_data(8 * 1024);
    let (server, request_times) = spawn_retry_after_once_server(data.clone()).await;
    let url = format!("http://{server}/file.bin");
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");
    let (runtime_tx, mut runtime_rx) = runtime_updates_channel();
    download_task(
        DownloadId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        HttpDownloadConfig {
            max_retries_per_chunk: 2,
            ..HttpDownloadConfig::default()
        },
        CancellationToken::new(),
        Arc::new(Mutex::new(None)),
        Arc::new(Mutex::new(None)),
        None,
        unlimited_semaphore(),
        unlimited_throttle(),
        runtime_tx,
    )
    .await;

    let updates = drain_progress(&mut runtime_rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Finished));
    assert_eq!(std::fs::read(&dest).unwrap(), data);

    let request_times = request_times.lock().unwrap();
    assert!(
        request_times.len() >= 2,
        "test did not observe a retry after the 429 response"
    );
    assert!(
        request_times[1].duration_since(request_times[0]) >= Duration::from_millis(900),
        "Retry-After should delay the second range attempt"
    );
}

struct ProbeThenIgnoreRangeResponder {
    data: Vec<u8>,
    requests: Arc<AtomicUsize>,
}

impl Respond for ProbeThenIgnoreRangeResponder {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let request_index = self.requests.fetch_add(1, Ordering::Relaxed);
        let range_header = request
            .headers
            .get("range")
            .expect("missing range header")
            .to_str()
            .unwrap();

        if request_index == 0 {
            let range = range_header.strip_prefix("bytes=").unwrap();
            let parts: Vec<&str> = range.split('-').collect();
            let start: usize = parts[0].parse().unwrap();
            let end: usize = parts[1].parse::<usize>().unwrap() + 1;
            let content_range = format!("bytes {}-{}/{}", start, end - 1, self.data.len());
            return ResponseTemplate::new(206)
                .set_body_bytes(self.data[start..end].to_vec())
                .insert_header("content-range", content_range.as_str());
        }

        ResponseTemplate::new(200).set_body_bytes(self.data.clone())
    }
}

async fn spawn_retry_after_once_server(
    data: Vec<u8>,
) -> (std::net::SocketAddr, Arc<Mutex<Vec<Instant>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let request_times = Arc::new(Mutex::new(Vec::new()));
    let worker_requests = Arc::new(AtomicUsize::new(0));
    let times_for_task = Arc::clone(&request_times);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let data = data.clone();
            let times = Arc::clone(&times_for_task);
            let worker_requests = Arc::clone(&worker_requests);
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

                if start == 0 && end == 1 {
                    write_range_response(&mut socket, &data, start, end).await;
                    return;
                }

                times.lock().unwrap().push(Instant::now());
                if worker_requests.fetch_add(1, Ordering::SeqCst) == 0 {
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                    return;
                }

                write_range_response(&mut socket, &data, start, end).await;
            });
        }
    });

    (addr, request_times)
}

async fn write_range_response(
    socket: &mut tokio::net::TcpStream,
    data: &[u8],
    start: usize,
    end: usize,
) {
    use tokio::io::AsyncWriteExt;

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
