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
use wiremock::{Mock, MockServer};

use ophelia::engine::http::HttpDownloadConfig;
use ophelia::engine::types::{TransferId, TransferStatus};

#[tokio::test(flavor = "multi_thread")]
async fn work_stealing_produces_correct_output() {
    // min_steal_bytes is lowered so this small file can still hit stealing
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
    let (runtime_tx, mut runtime_rx) = runtime_updates_channel();
    let config = HttpDownloadConfig {
        min_connections: 4,
        min_steal_bytes: 4 * 1024,
        ..HttpDownloadConfig::default()
    };
    download_task(
        TransferId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        config,
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
    assert_eq!(last_status(&updates), Some(TransferStatus::Finished));

    let downloaded = std::fs::read(&dest).unwrap();
    assert_eq!(downloaded.len(), data.len());
    assert_eq!(sha256(&downloaded), expected_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn balanced_default_downloads_with_live_strategy_defaults() {
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
    let (runtime_tx, mut runtime_rx) = runtime_updates_channel();
    let config = HttpDownloadConfig {
        min_connections: 4,
        max_connections: 4,
        ..HttpDownloadConfig::default()
    };
    download_task(
        TransferId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        config,
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
    assert_eq!(last_status(&updates), Some(TransferStatus::Finished));

    let downloaded = std::fs::read(&dest).unwrap();
    assert_eq!(downloaded.len(), data.len());
    assert_eq!(sha256(&downloaded), expected_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn hedge_races_duplicate_connection_and_produces_correct_output() {
    let data = test_data(32 * 1024);
    let expected_hash = sha256(&data);
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
    let config = HttpDownloadConfig {
        min_connections: 2,
        max_connections: 2,
        min_steal_bytes: 10 * 1024,
        ..HttpDownloadConfig::default()
    };
    download_task(
        TransferId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        config,
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
    assert_eq!(last_status(&updates), Some(TransferStatus::Finished));
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

    let server = spawn_hedge_range_server(
        data.clone(),
        Arc::clone(&second_half_requests),
        true,
        Duration::from_millis(800),
    )
    .await;

    let url = format!("http://{server}/file.bin");
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");
    let (runtime_tx, mut runtime_rx) = runtime_updates_channel();
    let config = HttpDownloadConfig {
        min_connections: 2,
        max_connections: 2,
        min_steal_bytes: 10 * 1024,
        ..HttpDownloadConfig::default()
    };
    download_task(
        TransferId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        config,
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
    assert_eq!(last_status(&updates), Some(TransferStatus::Finished));
    assert!(
        second_half_requests.load(Ordering::Relaxed) >= 2,
        "test did not exercise the hedge path"
    );

    let downloaded = std::fs::read(&dest).unwrap();
    assert_eq!(downloaded.len(), data.len());
    assert_eq!(sha256(&downloaded), expected_hash);
}
