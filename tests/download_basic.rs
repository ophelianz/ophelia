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

use std::sync::{Arc, Mutex};

use ophelia::engine::destination::DestinationPolicy;
use ophelia::settings::{CollisionStrategy, DestinationRule, Settings};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Respond, ResponseTemplate};

use ophelia::engine::http::{HttpDownloadConfig, download_task};
use ophelia::engine::types::{
    DownloadId, DownloadStatus, TaskRuntimeUpdate, TransferControlSupport,
};

struct RangeDispositionResponder {
    data: Vec<u8>,
    filename: &'static str,
}

impl Respond for RangeDispositionResponder {
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
            .insert_header(
                "content-disposition",
                format!("attachment; filename=\"{}\"", self.filename),
            )
    }
}

async fn spawn_no_content_length_server(body: Vec<u8>) -> std::net::SocketAddr {
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
            let body = body.clone();
            tokio::spawn(async move {
                let mut request = vec![0u8; 4096];
                let _ = socket.read(&mut request).await;
                let _ = socket
                    .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                    .await;
                let _ = socket.write_all(&body).await;
            });
        }
    });
    addr
}

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
    let (runtime_tx, _runtime_rx) = runtime_updates_channel();
    download_task(
        DownloadId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        HttpDownloadConfig::default(),
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
    let (runtime_tx, _runtime_rx) = runtime_updates_channel();
    download_task(
        DownloadId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        HttpDownloadConfig::default(),
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
    assert_eq!(sha256(&downloaded), expected_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn fallback_when_no_content_length() {
    let data = test_data(3_000);
    let expected_hash = sha256(&data);

    let server = spawn_no_content_length_server(data.clone()).await;
    let url = format!("http://{server}/file.bin");
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_tx, _runtime_rx) = runtime_updates_channel();
    download_task(
        DownloadId(0),
        url,
        dest.clone(),
        exact_destination_policy(&dest),
        HttpDownloadConfig::default(),
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
    assert_eq!(sha256(&downloaded), expected_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_content_length_fallback_emits_narrowed_runtime_control_support() {
    let data = test_data(2_000);
    let server = spawn_no_content_length_server(data).await;
    let url = format!("http://{server}/file.bin");
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_tx, mut runtime_rx) = runtime_updates_channel();
    download_task(
        DownloadId(0),
        url,
        dest,
        exact_destination_policy(&dir.path().join("file.bin")),
        HttpDownloadConfig::default(),
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

    let mut saw_support_change = false;
    while let Ok(update) = runtime_rx.try_recv() {
        if let TaskRuntimeUpdate::ControlSupportChanged { support, .. } = update {
            assert_eq!(
                support,
                TransferControlSupport {
                    can_pause: false,
                    can_resume: false,
                    can_cancel: true,
                    can_restore: false,
                }
            );
            saw_support_change = true;
        }
    }
    assert!(
        saw_support_change,
        "expected runtime control-support narrowing"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn error_on_server_down() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_tx, _runtime_rx) = runtime_updates_channel();
    download_task(
        DownloadId(0),
        "http://127.0.0.1:1".to_string(),
        dest,
        exact_destination_policy(&dir.path().join("file.bin")),
        HttpDownloadConfig::default(),
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
    let (runtime_tx, _runtime_rx) = runtime_updates_channel();
    download_task(
        DownloadId(0),
        url,
        dest,
        exact_destination_policy(&dir.path().join("file.bin")),
        HttpDownloadConfig::default(),
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

    let downloading: Vec<_> = updates
        .iter()
        .filter(|u| u.status == DownloadStatus::Downloading)
        .collect();

    for window in downloading.windows(2) {
        assert!(window[1].downloaded_bytes >= window[0].downloaded_bytes);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn content_disposition_reruns_destination_rules_before_writing() {
    let data = test_data(12_000);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/download"))
        .respond_with(RangeDispositionResponder {
            data: data.clone(),
            filename: "movie.mp4",
        })
        .mount(&server)
        .await;

    let root = tempfile::tempdir().unwrap();
    let settings = Settings {
        default_download_dir: Some(root.path().join("Downloads")),
        destination_rules_enabled: true,
        destination_rules: vec![DestinationRule {
            id: "movies".into(),
            label: "Movies".into(),
            enabled: true,
            target_dir: root.path().join("Movies"),
            extensions: vec![".mp4".into()],
            icon_name: None,
        }],
        ..Settings::default()
    };
    let url = format!("{}/download", server.uri());
    let initial_destination = settings.download_dir().join("download");
    let destination_sink = Arc::new(Mutex::new(None));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_tx, _runtime_rx) = runtime_updates_channel();

    download_task(
        DownloadId(0),
        url,
        initial_destination,
        DestinationPolicy::automatic(&settings),
        HttpDownloadConfig::default(),
        tx,
        CancellationToken::new(),
        Arc::new(Mutex::new(None)),
        Arc::clone(&destination_sink),
        None,
        unlimited_semaphore(),
        unlimited_throttle(),
        runtime_tx,
    )
    .await;

    let updates = drain_progress(&mut rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Finished));

    let final_destination = root.path().join("Movies").join("movie.mp4");
    assert!(final_destination.exists());
    assert_eq!(std::fs::read(&final_destination).unwrap(), data);
    assert_eq!(
        destination_sink.lock().unwrap().clone(),
        Some(final_destination)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn chunked_replace_strategy_replaces_existing_file_on_commit() {
    let data = test_data(8_000);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(RangeResponder { data: data.clone() })
        .mount(&server)
        .await;

    let root = tempfile::tempdir().unwrap();
    let settings = Settings {
        default_download_dir: Some(root.path().join("Downloads")),
        collision_strategy: CollisionStrategy::Replace,
        ..Settings::default()
    };
    let destination = settings.download_dir().join("file.bin");
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::write(&destination, b"old").unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_tx, _runtime_rx) = runtime_updates_channel();
    download_task(
        DownloadId(0),
        format!("{}/file.bin", server.uri()),
        destination.clone(),
        DestinationPolicy::automatic(&settings),
        HttpDownloadConfig::default(),
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
    assert_eq!(std::fs::read(&destination).unwrap(), data);
    assert!(!destination.with_file_name("file.bin.ophelia_part").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn single_stream_replace_strategy_replaces_existing_file_on_commit() {
    let data = test_data(7_000);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
        .mount(&server)
        .await;

    let root = tempfile::tempdir().unwrap();
    let settings = Settings {
        default_download_dir: Some(root.path().join("Downloads")),
        collision_strategy: CollisionStrategy::Replace,
        ..Settings::default()
    };
    let destination = settings.download_dir().join("file.bin");
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::write(&destination, b"old").unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_tx, _runtime_rx) = runtime_updates_channel();
    download_task(
        DownloadId(0),
        format!("{}/file.bin", server.uri()),
        destination.clone(),
        DestinationPolicy::automatic(&settings),
        HttpDownloadConfig::default(),
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
    assert_eq!(std::fs::read(&destination).unwrap(), data);
    assert!(!destination.with_file_name("file.bin.ophelia_part").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn active_part_file_duplicate_returns_error() {
    let data = test_data(6_000);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(RangeResponder { data: data.clone() })
        .mount(&server)
        .await;

    let root = tempfile::tempdir().unwrap();
    let settings = Settings {
        default_download_dir: Some(root.path().join("Downloads")),
        ..Settings::default()
    };
    let destination = settings.download_dir().join("file.bin");
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::write(
        destination.with_file_name("file.bin.ophelia_part"),
        b"partial",
    )
    .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (runtime_tx, _runtime_rx) = runtime_updates_channel();
    download_task(
        DownloadId(0),
        format!("{}/file.bin", server.uri()),
        destination,
        DestinationPolicy::automatic(&settings),
        HttpDownloadConfig::default(),
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
    assert_eq!(last_status(&updates), Some(DownloadStatus::Error));
}
