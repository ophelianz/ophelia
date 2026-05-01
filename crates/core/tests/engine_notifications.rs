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

use std::time::Duration;

use ophelia::engine::destination::{DestinationPolicy, part_path_for};
use ophelia::engine::http::HttpDownloadConfig;
use ophelia::engine::{
    ArtifactState, DownloadEngine, DownloadId, DownloadSpec, DownloadStatus, EngineEvent,
    LiveTransferRemovalAction, RestoredDownload, TransferChunkMapState, TransferControlSupport,
};
use ophelia::engine::{CoreConfig, DestinationPolicyConfig};
use tokio::runtime::Handle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn exact_destination_policy(destination: &std::path::Path) -> DestinationPolicy {
    DestinationPolicy::for_resolved_destination(&DestinationPolicyConfig::default(), destination)
}

fn engine_config(max_concurrent_downloads: usize) -> CoreConfig {
    CoreConfig {
        max_concurrent_downloads,
        ..CoreConfig::default()
    }
}

fn start_engine(
    config: CoreConfig,
    db_tx: std::sync::mpsc::Sender<ophelia::engine::DbEvent>,
    initial_next_id: u64,
) -> DownloadEngine {
    DownloadEngine::spawn_on(&Handle::current(), config, db_tx, initial_next_id)
}

async fn wait_for_matching_event(
    engine: &mut DownloadEngine,
    mut predicate: impl FnMut(&EngineEvent) -> bool,
) -> EngineEvent {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let notification = engine
                .next_event()
                .await
                .expect("engine notification channel closed");
            if predicate(&notification) {
                return notification;
            }
        }
    })
    .await
    .expect("timed out waiting for matching engine notification")
}

async fn wait_for_matching_progress(
    engine: &mut DownloadEngine,
    mut predicate: impl FnMut(&ophelia::engine::ProgressUpdate) -> bool,
) -> ophelia::engine::ProgressUpdate {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let event = engine
                .next_event()
                .await
                .expect("engine progress channel closed");
            let EngineEvent::Progress(update) = event else {
                continue;
            };
            if predicate(&update) {
                return update;
            }
        }
    })
    .await
    .expect("timed out waiting for matching progress update")
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

fn spawn_slow_no_content_length_server(body: Vec<u8>, delay: Duration) -> std::net::SocketAddr {
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
                std::thread::sleep(delay);
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

#[tokio::test(flavor = "multi_thread")]
async fn queued_pause_resume_cancel_and_delete_emit_distinct_notifications() {
    let (db_tx, _db_rx) = std::sync::mpsc::channel();
    let mut engine = start_engine(engine_config(0), db_tx, 1);

    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let id = engine
        .add(DownloadSpec::http(
            "https://example.com/file.bin".to_string(),
            destination.clone(),
            exact_destination_policy(&destination),
            HttpDownloadConfig::default(),
        ))
        .await
        .unwrap();

    engine.pause(id).await.unwrap();
    match wait_for_matching_event(&mut engine, |notification| {
        matches!(
            notification,
            EngineEvent::Progress(update)
                if update.id == id && update.status == DownloadStatus::Paused
        )
    })
    .await
    {
        EngineEvent::Progress(update) => {
            assert_eq!(update.id, id);
            assert_eq!(update.status, DownloadStatus::Paused);
            assert_eq!(update.downloaded_bytes, 0);
            assert_eq!(update.total_bytes, None);
        }
        other => panic!("expected pause update, got {other:?}"),
    }

    engine.resume(id).await.unwrap();
    match wait_for_matching_event(&mut engine, |notification| {
        matches!(
            notification,
            EngineEvent::Progress(update)
                if update.id == id && update.status == DownloadStatus::Pending
        )
    })
    .await
    {
        EngineEvent::Progress(update) => {
            assert_eq!(update.id, id);
            assert_eq!(update.status, DownloadStatus::Pending);
            assert_eq!(update.downloaded_bytes, 0);
            assert_eq!(update.total_bytes, None);
        }
        other => panic!("expected pending update, got {other:?}"),
    }

    engine.cancel(id).await.unwrap();
    match wait_for_matching_event(&mut engine, |notification| {
        matches!(
            notification,
            EngineEvent::LiveTransferRemoved { id: removed, .. } if *removed == id
        )
    })
    .await
    {
        EngineEvent::LiveTransferRemoved {
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

    let id = engine
        .add(DownloadSpec::http(
            "https://example.com/file.bin".to_string(),
            destination.clone(),
            exact_destination_policy(&destination),
            HttpDownloadConfig::default(),
        ))
        .await
        .unwrap();
    std::fs::write(&destination, b"partial").unwrap();
    engine
        .delete_artifact(id, destination.clone())
        .await
        .unwrap();
    match wait_for_matching_event(&mut engine, |notification| {
        matches!(
            notification,
            EngineEvent::LiveTransferRemoved { id: removed, .. } if *removed == id
        )
    })
    .await
    {
        EngineEvent::LiveTransferRemoved {
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

#[tokio::test(flavor = "multi_thread")]
async fn single_stream_http_emits_runtime_control_support_narrowing() {
    let server = spawn_no_content_length_server(vec![7u8; 2048]);

    let (db_tx, _db_rx) = std::sync::mpsc::channel();
    let mut engine = start_engine(CoreConfig::default(), db_tx, 1);
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let id = engine
        .add(DownloadSpec::http(
            format!("http://{server}/file.bin"),
            destination.clone(),
            exact_destination_policy(&destination),
            HttpDownloadConfig::default(),
        ))
        .await
        .unwrap();

    match wait_for_matching_event(&mut engine, |notification| {
        matches!(
            notification,
            EngineEvent::ControlSupportChanged { id: changed, .. } if *changed == id
        )
    })
    .await
    {
        EngineEvent::ControlSupportChanged {
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

#[tokio::test(flavor = "multi_thread")]
async fn single_stream_http_emits_download_bytes_written_event() {
    let server = spawn_no_content_length_server(vec![3u8; 4096]);

    let (db_tx, _db_rx) = std::sync::mpsc::channel();
    let mut engine = start_engine(CoreConfig::default(), db_tx, 1);
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let id = engine
        .add(DownloadSpec::http(
            format!("http://{server}/file.bin"),
            destination.clone(),
            exact_destination_policy(&destination),
            HttpDownloadConfig::default(),
        ))
        .await
        .unwrap();

    match wait_for_matching_event(&mut engine, |event| {
        matches!(event, EngineEvent::DownloadBytesWritten { id: changed, bytes } if *changed == id && *bytes > 0)
    })
    .await
    {
        EngineEvent::DownloadBytesWritten { id: changed, bytes } => {
            assert_eq!(changed, id);
            assert!(bytes > 0);
        }
        other => panic!("expected write stats event, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn destination_change_event_arrives_before_finished_progress() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/file.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-disposition", "attachment; filename=\"server.bin\""),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-disposition", "attachment; filename=\"server.bin\"")
                .set_body_bytes(vec![4u8; 2048]),
        )
        .mount(&server)
        .await;

    let (db_tx, _db_rx) = std::sync::mpsc::channel();
    let mut engine = start_engine(CoreConfig::default(), db_tx, 1);
    let tempdir = tempfile::tempdir().unwrap();
    let requested_destination = tempdir.path().join("file.bin");
    let server_destination = tempdir.path().join("server.bin");
    let destination_config = DestinationPolicyConfig {
        default_download_dir: tempdir.path().to_path_buf(),
        ..DestinationPolicyConfig::default()
    };
    let id = engine
        .add(DownloadSpec::http(
            format!("{}/file.bin", server.uri()),
            requested_destination.clone(),
            DestinationPolicy::automatic(&destination_config),
            HttpDownloadConfig::default(),
        ))
        .await
        .unwrap();

    let mut saw_destination_change = false;
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            match engine
                .next_event()
                .await
                .expect("engine event channel closed")
            {
                EngineEvent::DestinationChanged {
                    id: changed,
                    destination,
                } => {
                    if changed == id {
                        assert_eq!(destination, server_destination);
                        saw_destination_change = true;
                    }
                }
                EngineEvent::Progress(update)
                    if update.id == id && update.status == DownloadStatus::Finished =>
                {
                    assert!(saw_destination_change);
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("timed out waiting for destination and finish events");
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_during_probe_before_single_stream_fallback_exits_cleanly() {
    let server = spawn_slow_no_content_length_server(vec![7u8; 2048], Duration::from_millis(100));

    let (db_tx, _db_rx) = std::sync::mpsc::channel();
    let mut engine = start_engine(CoreConfig::default(), db_tx, 1);
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let id = engine
        .add(DownloadSpec::http(
            format!("http://{server}/file.bin"),
            destination.clone(),
            exact_destination_policy(&destination),
            HttpDownloadConfig::default(),
        ))
        .await
        .unwrap();

    engine.pause(id).await.unwrap();

    let update = wait_for_matching_progress(&mut engine, |update| {
        update.id == id && update.status == DownloadStatus::Error
    })
    .await;
    assert_eq!(update.downloaded_bytes, 0);
    assert!(!destination.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn chunked_http_emits_loading_snapshot_and_terminal_unsupported() {
    let server = spawn_slow_range_server(vec![5u8; 32 * 1024], 512, Duration::from_millis(25));

    let (db_tx, _db_rx) = std::sync::mpsc::channel();
    let mut engine = start_engine(CoreConfig::default(), db_tx, 1);
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let id = engine
        .add(DownloadSpec::http(
            format!("http://{server}/file.bin"),
            destination.clone(),
            exact_destination_policy(&destination),
            HttpDownloadConfig {
                speed_limit_bps: 20_000,
                write_buffer_size: 1024,
                ..HttpDownloadConfig::default()
            },
        ))
        .await
        .unwrap();

    match wait_for_matching_event(&mut engine, |notification| {
        matches!(
            notification,
            EngineEvent::ChunkMapChanged {
                id: changed,
                state: TransferChunkMapState::Loading,
            } if *changed == id
        )
    })
    .await
    {
        EngineEvent::ChunkMapChanged {
            id: changed,
            state: TransferChunkMapState::Loading,
        } => assert_eq!(changed, id),
        other => panic!("expected loading chunk-map state, got {other:?}"),
    }

    match wait_for_matching_event(&mut engine, |notification| {
        matches!(
            notification,
            EngineEvent::ChunkMapChanged {
                id: changed,
                state: TransferChunkMapState::Http(_),
            } if *changed == id
        )
    })
    .await
    {
        EngineEvent::ChunkMapChanged {
            id: changed,
            state: TransferChunkMapState::Http(snapshot),
        } => {
            assert_eq!(changed, id);
            assert_eq!(snapshot.cells.len(), 128);
        }
        other => panic!("expected http chunk-map snapshot, got {other:?}"),
    }

    match wait_for_matching_event(&mut engine, |notification| {
        matches!(
            notification,
            EngineEvent::ChunkMapChanged {
                id: changed,
                state: TransferChunkMapState::Unsupported,
            } if *changed == id
        )
    })
    .await
    {
        EngineEvent::ChunkMapChanged {
            id: changed,
            state: TransferChunkMapState::Unsupported,
        } => assert_eq!(changed, id),
        other => panic!("expected terminal unsupported chunk-map state, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pausing_active_http_clears_chunk_map_to_unsupported() {
    let server = spawn_slow_range_server(vec![9u8; 32 * 1024], 512, Duration::from_millis(25));

    let (db_tx, _db_rx) = std::sync::mpsc::channel();
    let mut engine = start_engine(CoreConfig::default(), db_tx, 1);
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let id = engine
        .add(DownloadSpec::http(
            format!("http://{server}/file.bin"),
            destination.clone(),
            exact_destination_policy(&destination),
            HttpDownloadConfig {
                speed_limit_bps: 20_000,
                write_buffer_size: 1024,
                ..HttpDownloadConfig::default()
            },
        ))
        .await
        .unwrap();

    let _ = wait_for_matching_event(&mut engine, |notification| {
        matches!(
            notification,
            EngineEvent::ChunkMapChanged {
                id: changed,
                state: TransferChunkMapState::Http(_),
            } if *changed == id
        )
    })
    .await;

    engine.pause(id).await.unwrap();
    match wait_for_matching_event(&mut engine, |notification| {
        matches!(
            notification,
            EngineEvent::ChunkMapChanged {
                id: changed,
                state: TransferChunkMapState::Unsupported,
            } if *changed == id
        )
    })
    .await
    {
        EngineEvent::ChunkMapChanged {
            id: changed,
            state: TransferChunkMapState::Unsupported,
        } => assert_eq!(changed, id),
        other => panic!("expected paused unsupported chunk-map state, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pausing_active_http_starts_next_queued_download() {
    let first_server =
        spawn_slow_range_server(vec![1u8; 128 * 1024], 512, Duration::from_millis(25));
    let second_server =
        spawn_slow_range_server(vec![2u8; 32 * 1024], 512, Duration::from_millis(25));

    let (db_tx, _db_rx) = std::sync::mpsc::channel();
    let mut engine = start_engine(engine_config(1), db_tx, 1);
    let tempdir = tempfile::tempdir().unwrap();
    let first_destination = tempdir.path().join("first.bin");
    let second_destination = tempdir.path().join("second.bin");

    let first_id = engine
        .add(DownloadSpec::http(
            format!("http://{first_server}/first.bin"),
            first_destination.clone(),
            exact_destination_policy(&first_destination),
            HttpDownloadConfig {
                speed_limit_bps: 20_000,
                write_buffer_size: 1024,
                ..HttpDownloadConfig::default()
            },
        ))
        .await
        .unwrap();
    let second_id = engine
        .add(DownloadSpec::http(
            format!("http://{second_server}/second.bin"),
            second_destination.clone(),
            exact_destination_policy(&second_destination),
            HttpDownloadConfig {
                speed_limit_bps: 20_000,
                write_buffer_size: 1024,
                ..HttpDownloadConfig::default()
            },
        ))
        .await
        .unwrap();

    let _ = wait_for_matching_event(&mut engine, |notification| {
        matches!(
            notification,
            EngineEvent::ChunkMapChanged {
                id,
                state: TransferChunkMapState::Http(_),
            } if *id == first_id
        )
    })
    .await;

    engine.pause(first_id).await.unwrap();

    match wait_for_matching_event(&mut engine, |notification| {
        matches!(
            notification,
            EngineEvent::ChunkMapChanged {
                id,
                state: TransferChunkMapState::Loading,
            } if *id == second_id
        )
    })
    .await
    {
        EngineEvent::ChunkMapChanged {
            id,
            state: TransferChunkMapState::Loading,
        } => assert_eq!(id, second_id),
        other => panic!("expected queued download to start, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn restored_http_without_resume_data_discards_stale_part_file_before_restart() {
    let data = vec![8u8; 32 * 1024];
    let server = spawn_slow_range_server(data.clone(), 8192, Duration::from_millis(0));

    let (db_tx, _db_rx) = std::sync::mpsc::channel();
    let mut engine = start_engine(CoreConfig::default(), db_tx, 1);
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let part_path = part_path_for(&destination);
    std::fs::write(&part_path, b"stale partial bytes").unwrap();

    let id = DownloadId(77);
    engine
        .restore(RestoredDownload::http(
            id,
            format!("http://{server}/file.bin"),
            destination.clone(),
            &DestinationPolicyConfig::default(),
            HttpDownloadConfig::default(),
            None,
        ))
        .await
        .unwrap();

    engine.resume(id).await.unwrap();

    let update = wait_for_matching_progress(&mut engine, |update| {
        update.id == id && update.status == DownloadStatus::Finished
    })
    .await;
    assert_eq!(update.downloaded_bytes, data.len() as u64);
    assert_eq!(std::fs::read(&destination).unwrap(), data);
    assert!(!part_path.exists());
}
