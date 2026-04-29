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
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Respond, ResponseTemplate};

use ophelia::engine::http::{HttpDownloadConfig, download_task};
use ophelia::engine::types::{DownloadId, DownloadStatus};

#[tokio::test(flavor = "multi_thread")]
async fn work_stealing_produces_correct_output() {
    // min_steal_bytes = 4KB so stealing triggers on this small test file.
    // With 8 connections on a 128KB file each initial chunk is 16KB; a chunk
    // with ~14KB remaining easily clears the 2×4KB = 8KB threshold.
    let data = test_data(128 * 1024);
    let expected_hash = sha256(&data);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(RangeResponder { data: data.clone() })
        .mount(&server)
        .await;

    let url = format!("{}/file.bin", server.uri());
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_tx, _runtime_rx) = runtime_updates_channel();
    let config = HttpDownloadConfig {
        min_connections: 4,
        min_steal_bytes: 4 * 1024,
        ..HttpDownloadConfig::default()
    };
    download_task(
        DownloadId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        config,
        tx,
        CancellationToken::new(),
        Arc::new(Mutex::new(None)),
        Arc::new(Mutex::new(None)),
        None,
        unlimited_semaphore(),
        unlimited_throttle(),
        runtime_tx,
    )
    .await;

    let updates = drain_progress(&mut rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Finished));

    let downloaded = std::fs::read(&dest).unwrap();
    assert_eq!(downloaded.len(), data.len());
    assert_eq!(sha256(&downloaded), expected_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn hedge_races_duplicate_connection_and_produces_correct_output() {
    let data = test_data(32 * 1024);
    let expected_hash = sha256(&data);
    let second_half_requests = Arc::new(AtomicUsize::new(0));

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(HedgeResponder {
            data: data.clone(),
            second_half_requests: Arc::clone(&second_half_requests),
            fail_second_half_hedge: false,
        })
        .mount(&server)
        .await;

    let url = format!("{}/file.bin", server.uri());
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_tx, _runtime_rx) = runtime_updates_channel();
    let config = HttpDownloadConfig {
        min_connections: 2,
        max_connections: 2,
        min_steal_bytes: 10 * 1024,
        ..HttpDownloadConfig::default()
    };
    download_task(
        DownloadId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        config,
        tx,
        CancellationToken::new(),
        Arc::new(Mutex::new(None)),
        Arc::new(Mutex::new(None)),
        None,
        unlimited_semaphore(),
        unlimited_throttle(),
        runtime_tx,
    )
    .await;

    let updates = drain_progress(&mut rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Finished));
    assert!(
        second_half_requests.load(Ordering::Relaxed) >= 2,
        "test did not exercise the hedge path"
    );

    let downloaded = std::fs::read(&dest).unwrap();
    assert_eq!(downloaded.len(), data.len());
    assert_eq!(sha256(&downloaded), expected_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_hedge_does_not_mark_original_range_complete() {
    let data = test_data(32 * 1024);
    let expected_hash = sha256(&data);
    let second_half_requests = Arc::new(AtomicUsize::new(0));

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(HedgeResponder {
            data: data.clone(),
            second_half_requests: Arc::clone(&second_half_requests),
            fail_second_half_hedge: true,
        })
        .mount(&server)
        .await;

    let url = format!("{}/file.bin", server.uri());
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_tx, _runtime_rx) = runtime_updates_channel();
    let config = HttpDownloadConfig {
        min_connections: 2,
        max_connections: 2,
        min_steal_bytes: 10 * 1024,
        ..HttpDownloadConfig::default()
    };
    download_task(
        DownloadId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        config,
        tx,
        CancellationToken::new(),
        Arc::new(Mutex::new(None)),
        Arc::new(Mutex::new(None)),
        None,
        unlimited_semaphore(),
        unlimited_throttle(),
        runtime_tx,
    )
    .await;

    let updates = drain_progress(&mut rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Finished));
    assert!(
        second_half_requests.load(Ordering::Relaxed) >= 2,
        "test did not exercise the hedge path"
    );

    let downloaded = std::fs::read(&dest).unwrap();
    assert_eq!(downloaded.len(), data.len());
    assert_eq!(sha256(&downloaded), expected_hash);
}

struct HedgeResponder {
    data: Vec<u8>,
    second_half_requests: Arc<AtomicUsize>,
    fail_second_half_hedge: bool,
}

impl Respond for HedgeResponder {
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
        let half = self.data.len() / 2;

        let second_half_request =
            (start >= half).then(|| self.second_half_requests.fetch_add(1, Ordering::Relaxed));
        if second_half_request == Some(1) && self.fail_second_half_hedge {
            return ResponseTemplate::new(404);
        }

        let content_range = format!("bytes {}-{}/{}", start, end - 1, self.data.len());
        let response = ResponseTemplate::new(206)
            .set_body_bytes(self.data[start..end].to_vec())
            .insert_header("content-range", content_range.as_str());

        if second_half_request == Some(0) {
            response.set_delay(Duration::from_millis(700))
        } else {
            response
        }
    }
}
