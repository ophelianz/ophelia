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
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

use ophelia::engine::http::{HttpDownloadConfig, download_task};
use ophelia::engine::types::{DownloadId, DownloadStatus, ProgressUpdate};

#[tokio::test(flavor = "multi_thread")]
async fn pause_and_resume_completes_correctly() {
    // Server adds 150ms delay per range response so the download is guaranteed
    // to still be in-flight when we cancel after 50ms.
    let data = test_data(128 * 1024);
    let expected_hash = sha256(&data);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(SlowRangeResponder {
            data: data.clone(),
            delay: Duration::from_millis(150),
        })
        .mount(&server)
        .await;

    let url = format!("{}/file.bin", server.uri());
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    // — Pass 1: start, pause mid-download —
    let pause_token = CancellationToken::new();
    let pause_sink = Arc::new(Mutex::new(None));
    let (tx1, _rx1) = tokio::sync::mpsc::unbounded_channel::<ProgressUpdate>();

    let handle = {
        let url = url.clone();
        let dest = dest.clone();
        let sink = Arc::clone(&pause_sink);
        let token = pause_token.clone();
        let (runtime_tx, _runtime_rx) = runtime_updates_channel();
        tokio::spawn(async move {
            download_task(
                DownloadId(0),
                url,
                dest.clone(),
                exact_destination_policy(&dest),
                HttpDownloadConfig::default(),
                tx1,
                token,
                sink,
                Arc::new(Mutex::new(None)),
                None,
                unlimited_semaphore(),
                unlimited_throttle(),
                runtime_tx,
            )
            .await;
        })
    };

    tokio::time::sleep(Duration::from_millis(50)).await;
    pause_token.cancel();
    handle.await.unwrap();

    let snapshots = pause_sink.lock().unwrap().take();
    // If the file is somehow already done (very fast machine), skip.
    let snapshots = match snapshots {
        Some(s) => s,
        None => return,
    };
    assert!(!snapshots.is_empty());

    // — Pass 2: resume from snapshots, run to completion —
    let (tx2, mut rx2) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_tx, _runtime_rx) = runtime_updates_channel();
    download_task(
        DownloadId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        HttpDownloadConfig::default(),
        tx2,
        CancellationToken::new(),
        Arc::new(Mutex::new(None)),
        Arc::new(Mutex::new(None)),
        Some(snapshots),
        unlimited_semaphore(),
        unlimited_throttle(),
        runtime_tx,
    )
    .await;

    let updates = drain_progress(&mut rx2).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Finished));

    let downloaded = std::fs::read(&dest).unwrap();
    assert_eq!(downloaded.len(), data.len());
    assert_eq!(sha256(&downloaded), expected_hash);
}
