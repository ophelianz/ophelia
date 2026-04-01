mod common;
use common::*;

use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use ophelia::engine::http::{download_task, HttpDownloadConfig};
use ophelia::engine::types::{DownloadId, DownloadStatus};

#[tokio::test(flavor = "multi_thread")]
async fn parallel_download_with_range_support() {
    let data = test_data(10_000);
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
    download_task(
        DownloadId(0), url, dest.clone(), HttpDownloadConfig::default(), tx,
        CancellationToken::new(), Arc::new(Mutex::new(None)), None,
        unlimited_semaphore(), unlimited_throttle(),
    ).await;

    let updates = drain_progress(&mut rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Finished));

    let downloaded = std::fs::read(&dest).unwrap();
    assert_eq!(downloaded.len(), data.len());
    assert_eq!(sha256(&downloaded), expected_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn single_stream_fallback_no_range_support() {
    let data = test_data(5_000);
    let expected_hash = sha256(&data);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
        .mount(&server)
        .await;

    let url = format!("{}/file.bin", server.uri());
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    download_task(
        DownloadId(0), url, dest.clone(), HttpDownloadConfig::default(), tx,
        CancellationToken::new(), Arc::new(Mutex::new(None)), None,
        unlimited_semaphore(), unlimited_throttle(),
    ).await;

    let updates = drain_progress(&mut rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Finished));

    let downloaded = std::fs::read(&dest).unwrap();
    assert_eq!(sha256(&downloaded), expected_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn fallback_when_no_content_length() {
    let data = test_data(3_000);
    let expected_hash = sha256(&data);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
        .mount(&server)
        .await;

    let url = format!("{}/file.bin", server.uri());
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    download_task(
        DownloadId(0), url, dest.clone(), HttpDownloadConfig::default(), tx,
        CancellationToken::new(), Arc::new(Mutex::new(None)), None,
        unlimited_semaphore(), unlimited_throttle(),
    ).await;

    let updates = drain_progress(&mut rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Finished));

    let downloaded = std::fs::read(&dest).unwrap();
    assert_eq!(sha256(&downloaded), expected_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn error_on_server_down() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    download_task(
        DownloadId(0),
        "http://127.0.0.1:1".to_string(),
        dest,
        HttpDownloadConfig::default(),
        tx,
        CancellationToken::new(),
        Arc::new(Mutex::new(None)),
        None,
        unlimited_semaphore(),
        unlimited_throttle(),
    ).await;

    let updates = drain_progress(&mut rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Error));
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_reports_increasing_bytes() {
    let data = test_data(50_000);

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
    download_task(
        DownloadId(0), url, dest, HttpDownloadConfig::default(), tx,
        CancellationToken::new(), Arc::new(Mutex::new(None)), None,
        unlimited_semaphore(), unlimited_throttle(),
    ).await;

    let updates = drain_progress(&mut rx).await;

    let downloading: Vec<_> = updates
        .iter()
        .filter(|u| u.status == DownloadStatus::Downloading)
        .collect();

    for window in downloading.windows(2) {
        assert!(window[1].downloaded_bytes >= window[0].downloaded_bytes);
    }
}
