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

use std::time::{Duration, Instant};

use ophelia::engine::destination::DestinationPolicy;
use ophelia::engine::http::HttpDownloadConfig;
use ophelia::engine::{
    ArtifactState, DownloadEngine, DownloadSpec, DownloadStatus, EngineNotification,
    LiveTransferRemovalAction, TransferChunkMapState, TransferControlSupport,
};
use ophelia::settings::Settings;

fn wait_for_matching_notification(
    engine: &mut DownloadEngine,
    mut predicate: impl FnMut(&EngineNotification) -> bool,
) -> EngineNotification {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(notification) = engine.poll_notification() {
            if predicate(&notification) {
                return notification;
            }
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for matching engine notification"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn spawn_no_content_length_server(body: Vec<u8>) -> std::net::SocketAddr {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else {
                break;
            };
            let body = body.clone();
            std::thread::spawn(move || {
                let mut request = [0u8; 4096];
                let _ = socket.read(&mut request);
                let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
                let _ = socket.write_all(&body);
            });
        }
    });
    addr
}

fn spawn_slow_range_server(
    body: Vec<u8>,
    chunk_size: usize,
    delay: Duration,
) -> std::net::SocketAddr {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else {
                break;
            };
            let body = body.clone();
            std::thread::spawn(move || {
                let mut request = [0u8; 4096];
                let bytes_read = socket.read(&mut request).unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..bytes_read]);
                let range_header = request
                    .lines()
                    .find_map(|line| line.strip_prefix("Range: bytes="));

                let (start, end) = if let Some(range) = range_header {
                    let mut parts = range.trim().split('-');
                    let start = parts.next().unwrap_or("0").parse::<usize>().unwrap_or(0);
                    let end = parts
                        .next()
                        .and_then(|end| end.parse::<usize>().ok())
                        .unwrap_or_else(|| body.len().saturating_sub(1))
                        .min(body.len().saturating_sub(1));
                    (start.min(end), end)
                } else {
                    (0, body.len().saturating_sub(1))
                };

                let payload = &body[start..=end];
                let headers = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    start,
                    end,
                    body.len(),
                    payload.len(),
                );
                let _ = socket.write_all(headers.as_bytes());

                for chunk in payload.chunks(chunk_size.max(1)) {
                    let _ = socket.write_all(chunk);
                    let _ = socket.flush();
                    std::thread::sleep(delay);
                }
            });
        }
    });
    addr
}

#[test]
fn queued_pause_resume_cancel_and_delete_emit_distinct_notifications() {
    let (db_tx, _db_rx) = std::sync::mpsc::channel();
    let settings = Settings {
        max_concurrent_downloads: 0,
        ..Settings::default()
    };
    let mut engine = DownloadEngine::new(settings, db_tx, 1);

    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let id = engine.add(DownloadSpec::http(
        "https://example.com/file.bin".to_string(),
        destination.clone(),
        DestinationPolicy::for_resolved_destination(&Settings::default(), &destination),
        HttpDownloadConfig::default(),
    ));

    engine.pause(id);
    match wait_for_matching_notification(&mut engine, |notification| {
        matches!(
            notification,
            EngineNotification::Update(update)
                if update.id == id && update.status == DownloadStatus::Paused
        )
    }) {
        EngineNotification::Update(update) => {
            assert_eq!(update.id, id);
            assert_eq!(update.status, DownloadStatus::Paused);
            assert_eq!(update.downloaded_bytes, 0);
            assert_eq!(update.total_bytes, None);
        }
        other => panic!("expected pause update, got {other:?}"),
    }

    engine.resume(id);
    match wait_for_matching_notification(&mut engine, |notification| {
        matches!(
            notification,
            EngineNotification::Update(update)
                if update.id == id && update.status == DownloadStatus::Pending
        )
    }) {
        EngineNotification::Update(update) => {
            assert_eq!(update.id, id);
            assert_eq!(update.status, DownloadStatus::Pending);
            assert_eq!(update.downloaded_bytes, 0);
            assert_eq!(update.total_bytes, None);
        }
        other => panic!("expected pending update, got {other:?}"),
    }

    engine.cancel(id);
    match wait_for_matching_notification(&mut engine, |notification| {
        matches!(
            notification,
            EngineNotification::LiveTransferRemoved { id: removed, .. } if *removed == id
        )
    }) {
        EngineNotification::LiveTransferRemoved {
            id: removed,
            action,
            artifact_state,
        } => {
            assert_eq!(removed, id);
            assert_eq!(action, LiveTransferRemovalAction::Cancelled);
            assert_eq!(artifact_state, ArtifactState::Missing);
        }
        other => panic!("expected cancelled removal notification, got {other:?}"),
    }

    let id = engine.add(DownloadSpec::http(
        "https://example.com/file.bin".to_string(),
        destination.clone(),
        DestinationPolicy::for_resolved_destination(&Settings::default(), &destination),
        HttpDownloadConfig::default(),
    ));
    std::fs::write(&destination, b"partial").unwrap();
    engine.delete_artifact(id, destination.clone());
    match wait_for_matching_notification(&mut engine, |notification| {
        matches!(
            notification,
            EngineNotification::LiveTransferRemoved { id: removed, .. } if *removed == id
        )
    }) {
        EngineNotification::LiveTransferRemoved {
            id: removed,
            action,
            artifact_state,
        } => {
            assert_eq!(removed, id);
            assert_eq!(action, LiveTransferRemovalAction::DeleteArtifact);
            assert_eq!(artifact_state, ArtifactState::Deleted);
            assert!(!destination.exists());
        }
        other => panic!("expected removed notification, got {other:?}"),
    }
}

#[test]
fn single_stream_http_emits_runtime_control_support_narrowing() {
    let server = spawn_no_content_length_server(vec![7u8; 2048]);

    let (db_tx, _db_rx) = std::sync::mpsc::channel();
    let mut engine = DownloadEngine::new(Settings::default(), db_tx, 1);
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let id = engine.add(DownloadSpec::http(
        format!("http://{server}/file.bin"),
        destination.clone(),
        DestinationPolicy::for_resolved_destination(&Settings::default(), &destination),
        HttpDownloadConfig::default(),
    ));

    match wait_for_matching_notification(&mut engine, |notification| {
        matches!(
            notification,
            EngineNotification::ControlSupportChanged { id: changed, .. } if *changed == id
        )
    }) {
        EngineNotification::ControlSupportChanged {
            id: changed,
            support,
        } => {
            assert_eq!(changed, id);
            assert_eq!(
                support,
                TransferControlSupport {
                    can_pause: false,
                    can_resume: false,
                    can_cancel: true,
                    can_restore: false,
                }
            );
        }
        other => panic!("expected control-support change, got {other:?}"),
    }
}

#[test]
fn chunked_http_emits_loading_snapshot_and_terminal_unsupported() {
    let server = spawn_slow_range_server(vec![5u8; 32 * 1024], 512, Duration::from_millis(25));

    let (db_tx, _db_rx) = std::sync::mpsc::channel();
    let mut engine = DownloadEngine::new(Settings::default(), db_tx, 1);
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let id = engine.add(DownloadSpec::http(
        format!("http://{server}/file.bin"),
        destination.clone(),
        DestinationPolicy::for_resolved_destination(&Settings::default(), &destination),
        HttpDownloadConfig {
            speed_limit_bps: 20_000,
            write_buffer_size: 1024,
            ..HttpDownloadConfig::default()
        },
    ));

    match wait_for_matching_notification(&mut engine, |notification| {
        matches!(
            notification,
            EngineNotification::ChunkMapStateChanged {
                id: changed,
                state: TransferChunkMapState::Loading,
            } if *changed == id
        )
    }) {
        EngineNotification::ChunkMapStateChanged {
            id: changed,
            state: TransferChunkMapState::Loading,
        } => assert_eq!(changed, id),
        other => panic!("expected loading chunk-map state, got {other:?}"),
    }

    match wait_for_matching_notification(&mut engine, |notification| {
        matches!(
            notification,
            EngineNotification::ChunkMapStateChanged {
                id: changed,
                state: TransferChunkMapState::Http(_),
            } if *changed == id
        )
    }) {
        EngineNotification::ChunkMapStateChanged {
            id: changed,
            state: TransferChunkMapState::Http(snapshot),
        } => {
            assert_eq!(changed, id);
            assert_eq!(snapshot.cells.len(), 128);
        }
        other => panic!("expected http chunk-map snapshot, got {other:?}"),
    }

    match wait_for_matching_notification(&mut engine, |notification| {
        matches!(
            notification,
            EngineNotification::ChunkMapStateChanged {
                id: changed,
                state: TransferChunkMapState::Unsupported,
            } if *changed == id
        )
    }) {
        EngineNotification::ChunkMapStateChanged {
            id: changed,
            state: TransferChunkMapState::Unsupported,
        } => assert_eq!(changed, id),
        other => panic!("expected terminal unsupported chunk-map state, got {other:?}"),
    }
}

#[test]
fn pausing_active_http_clears_chunk_map_to_unsupported() {
    let server = spawn_slow_range_server(vec![9u8; 32 * 1024], 512, Duration::from_millis(25));

    let (db_tx, _db_rx) = std::sync::mpsc::channel();
    let mut engine = DownloadEngine::new(Settings::default(), db_tx, 1);
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let id = engine.add(DownloadSpec::http(
        format!("http://{server}/file.bin"),
        destination.clone(),
        DestinationPolicy::for_resolved_destination(&Settings::default(), &destination),
        HttpDownloadConfig {
            speed_limit_bps: 20_000,
            write_buffer_size: 1024,
            ..HttpDownloadConfig::default()
        },
    ));

    let _ = wait_for_matching_notification(&mut engine, |notification| {
        matches!(
            notification,
            EngineNotification::ChunkMapStateChanged {
                id: changed,
                state: TransferChunkMapState::Http(_),
            } if *changed == id
        )
    });

    engine.pause(id);
    match wait_for_matching_notification(&mut engine, |notification| {
        matches!(
            notification,
            EngineNotification::ChunkMapStateChanged {
                id: changed,
                state: TransferChunkMapState::Unsupported,
            } if *changed == id
        )
    }) {
        EngineNotification::ChunkMapStateChanged {
            id: changed,
            state: TransferChunkMapState::Unsupported,
        } => assert_eq!(changed, id),
        other => panic!("expected paused unsupported chunk-map state, got {other:?}"),
    }
}
