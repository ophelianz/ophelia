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

/***************************************************
** This file is part of Ophelia, distributed under the
** terms of the GPL License, version 3 or later.
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( tests, plz pass )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

mod common;
use common::*;

use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

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
    // Exercises HedgeWork: 1 connection on a 32KB file, min_steal_bytes = 32KB.
    // try_steal needs stealable >= 2 * 32KB = 64KB, which never holds on a 32KB
    // file, so steal always fails. try_hedge fires when current_limit > join_set.len()
    // after a completion and limit doubling. Both connections write the same byte
    // range via write_at (idempotent). The first to finish snaps the original's
    // counter; download completes correctly.
    let data = test_data(32 * 1024);
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
        min_connections: 1,
        max_connections: 1,
        min_steal_bytes: 32 * 1024,
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
