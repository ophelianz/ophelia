use super::host::should_flush_before_immediate;
use super::lock::session_dir;
use super::read_model::{SessionEventCoalescer, SessionReadModel};
use super::wire::{SessionWireCommand, SessionWireFrame};
use super::*;
use crate::engine::{DownloadStatus, TransferChunkMapState, TransferControlSupport};
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

fn test_paths(dir: &TempDir) -> CorePaths {
    CorePaths::new(
        dir.path().join("downloads.db"),
        None,
        dir.path().join("downloads"),
    )
}

async fn spawn_single_response_server(body: &'static [u8]) -> String {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok(Ok((mut stream, _))) =
                tokio::time::timeout(Duration::from_secs(5), listener.accept()).await
            else {
                break;
            };
            tokio::spawn(async move {
                let mut request = [0_u8; 1024];
                let read = stream.read(&mut request).await.unwrap_or(0);
                let is_head = request[..read].starts_with(b"HEAD ");
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).await.unwrap();
                if !is_head {
                    stream.write_all(body).await.unwrap();
                }
            });
        }
    });
    format!("http://{addr}/file.bin")
}

#[cfg(unix)]
fn file_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
fn socket_client(host: &SessionHost) -> SessionClient {
    SessionClient::connect_local(host.descriptor().unwrap()).unwrap()
}

fn test_snapshot(id: DownloadId, status: DownloadStatus) -> TransferSnapshot {
    TransferSnapshot {
        id,
        provider_kind: "http".into(),
        source_label: "https://example.com/file.bin".into(),
        destination: PathBuf::from("file.bin"),
        status,
        downloaded_bytes: 0,
        total_bytes: Some(100),
        speed_bytes_per_sec: 0,
        control_support: TransferControlSupport::all(),
        chunk_map_state: TransferChunkMapState::Unsupported,
    }
}

#[test]
fn session_read_model_coalesces_hot_transfer_updates() {
    let id = DownloadId(1);
    let mut read_model = SessionReadModel::default();
    let mut coalescer = SessionEventCoalescer::default();

    let added = read_model
        .apply_engine_event(
            EngineEvent::TransferAdded {
                snapshot: test_snapshot(id, DownloadStatus::Pending),
            },
            &mut coalescer,
        )
        .unwrap();
    assert!(matches!(added, SessionEvent::TransferChanged { .. }));

    assert!(
        read_model
            .apply_engine_event(
                EngineEvent::Progress(ProgressUpdate {
                    id,
                    status: DownloadStatus::Downloading,
                    downloaded_bytes: 40,
                    total_bytes: Some(100),
                    speed_bytes_per_sec: 10,
                }),
                &mut coalescer,
            )
            .is_none()
    );
    assert!(
        read_model
            .apply_engine_event(
                EngineEvent::ChunkMapChanged {
                    id,
                    state: TransferChunkMapState::Loading,
                },
                &mut coalescer,
            )
            .is_none()
    );
    assert!(
        read_model
            .apply_engine_event(
                EngineEvent::DownloadBytesWritten { id, bytes: 10 },
                &mut coalescer,
            )
            .is_none()
    );
    assert!(
        read_model
            .apply_engine_event(
                EngineEvent::DownloadBytesWritten { id, bytes: 15 },
                &mut coalescer,
            )
            .is_none()
    );

    let events = coalescer.drain_events();
    assert_eq!(events.len(), 2);
    match &events[0] {
        SessionEvent::TransferChanged { snapshot } => {
            assert_eq!(snapshot.downloaded_bytes, 40);
            assert_eq!(snapshot.chunk_map_state, TransferChunkMapState::Loading);
        }
        event => panic!("expected transfer update, got {event:?}"),
    }
    match &events[1] {
        SessionEvent::DownloadBytesWritten {
            id: event_id,
            bytes,
        } => {
            assert_eq!(*event_id, id);
            assert_eq!(*bytes, 25);
        }
        event => panic!("expected write update, got {event:?}"),
    }
}

#[test]
fn coalescer_stats_count_raw_and_emitted_hot_events() {
    let id = DownloadId(2);
    let mut read_model = SessionReadModel::default();
    let mut coalescer = SessionEventCoalescer::default();
    let _ = read_model.apply_engine_event(
        EngineEvent::TransferAdded {
            snapshot: test_snapshot(id, DownloadStatus::Pending),
        },
        &mut coalescer,
    );

    for downloaded_bytes in [10, 20, 30] {
        let _ = read_model.apply_engine_event(
            EngineEvent::Progress(ProgressUpdate {
                id,
                status: DownloadStatus::Downloading,
                downloaded_bytes,
                total_bytes: Some(100),
                speed_bytes_per_sec: downloaded_bytes,
            }),
            &mut coalescer,
        );
    }
    let _ = read_model.apply_engine_event(
        EngineEvent::DownloadBytesWritten { id, bytes: 10 },
        &mut coalescer,
    );
    let _ = read_model.apply_engine_event(
        EngineEvent::DownloadBytesWritten { id, bytes: 20 },
        &mut coalescer,
    );

    let events = coalescer.drain_events();
    let stats = coalescer.stats();

    assert_eq!(events.len(), 2);
    assert_eq!(stats.raw_transfer_updates, 3);
    assert_eq!(stats.raw_write_updates, 2);
    assert_eq!(stats.emitted_transfer_updates, 1);
    assert_eq!(stats.emitted_write_updates, 1);
    assert_eq!(stats.coalesced_transfer_updates(), 2);
    assert_eq!(stats.coalesced_write_updates(), 1);
}

#[test]
fn terminal_progress_clears_stale_coalesced_updates() {
    let id = DownloadId(3);
    let mut read_model = SessionReadModel::default();
    let mut coalescer = SessionEventCoalescer::default();
    let _ = read_model.apply_engine_event(
        EngineEvent::TransferAdded {
            snapshot: test_snapshot(id, DownloadStatus::Pending),
        },
        &mut coalescer,
    );
    let _ = read_model.apply_engine_event(
        EngineEvent::Progress(ProgressUpdate {
            id,
            status: DownloadStatus::Downloading,
            downloaded_bytes: 50,
            total_bytes: Some(100),
            speed_bytes_per_sec: 10,
        }),
        &mut coalescer,
    );

    let finished = read_model
        .apply_engine_event(
            EngineEvent::Progress(ProgressUpdate {
                id,
                status: DownloadStatus::Finished,
                downloaded_bytes: 100,
                total_bytes: Some(100),
                speed_bytes_per_sec: 0,
            }),
            &mut coalescer,
        )
        .unwrap();

    match finished {
        SessionEvent::TransferChanged { snapshot } => {
            assert_eq!(snapshot.status, DownloadStatus::Finished);
            assert_eq!(snapshot.downloaded_bytes, 100);
        }
        event => panic!("expected finished transfer update, got {event:?}"),
    }
    assert!(coalescer.drain_events().is_empty());
}

#[test]
fn snapshot_reflects_pending_hot_updates_before_flush() {
    let id = DownloadId(4);
    let mut read_model = SessionReadModel::default();
    let mut coalescer = SessionEventCoalescer::default();
    let _ = read_model.apply_engine_event(
        EngineEvent::TransferAdded {
            snapshot: test_snapshot(id, DownloadStatus::Pending),
        },
        &mut coalescer,
    );
    let _ = read_model.apply_engine_event(
        EngineEvent::Progress(ProgressUpdate {
            id,
            status: DownloadStatus::Downloading,
            downloaded_bytes: 80,
            total_bytes: Some(100),
            speed_bytes_per_sec: 12,
        }),
        &mut coalescer,
    );
    let _ = read_model.apply_engine_event(
        EngineEvent::ChunkMapChanged {
            id,
            state: TransferChunkMapState::Loading,
        },
        &mut coalescer,
    );

    let snapshot = read_model.snapshot();

    assert_eq!(snapshot.transfers.len(), 1);
    assert_eq!(snapshot.transfers[0].downloaded_bytes, 80);
    assert_eq!(
        snapshot.transfers[0].chunk_map_state,
        TransferChunkMapState::Loading
    );
}

#[test]
fn terminal_events_flush_hot_updates_first() {
    assert!(!should_flush_before_immediate(
        &EngineEvent::DownloadBytesWritten {
            id: DownloadId(5),
            bytes: 8,
        }
    ));
    assert!(!should_flush_before_immediate(&EngineEvent::Progress(
        ProgressUpdate {
            id: DownloadId(5),
            status: DownloadStatus::Downloading,
            downloaded_bytes: 8,
            total_bytes: Some(100),
            speed_bytes_per_sec: 8,
        }
    )));
    assert!(should_flush_before_immediate(&EngineEvent::Progress(
        ProgressUpdate {
            id: DownloadId(5),
            status: DownloadStatus::Finished,
            downloaded_bytes: 100,
            total_bytes: Some(100),
            speed_bytes_per_sec: 0,
        }
    )));
    assert!(should_flush_before_immediate(
        &EngineEvent::LiveTransferRemoved {
            id: DownloadId(5),
            action: LiveTransferRemovalAction::DeleteArtifact,
            artifact_state: ArtifactState::Deleted,
        }
    ));
}

#[test]
fn terminal_progress_keeps_pending_write_bytes() {
    let id = DownloadId(6);
    let mut read_model = SessionReadModel::default();
    let mut coalescer = SessionEventCoalescer::default();
    let _ = read_model.apply_engine_event(
        EngineEvent::TransferAdded {
            snapshot: test_snapshot(id, DownloadStatus::Pending),
        },
        &mut coalescer,
    );
    let _ = read_model.apply_engine_event(
        EngineEvent::DownloadBytesWritten { id, bytes: 32 },
        &mut coalescer,
    );

    let finished = read_model.apply_engine_event(
        EngineEvent::Progress(ProgressUpdate {
            id,
            status: DownloadStatus::Finished,
            downloaded_bytes: 100,
            total_bytes: Some(100),
            speed_bytes_per_sec: 0,
        }),
        &mut coalescer,
    );

    assert!(matches!(
        finished,
        Some(SessionEvent::TransferChanged { .. })
    ));
    assert!(matches!(
        coalescer.drain_events().as_slice(),
        [SessionEvent::DownloadBytesWritten { id: event_id, bytes }]
            if *event_id == id && *bytes == 32
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn session_host_rejects_second_owner_for_profile() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths.clone())
        .expect("first session should start");

    let second = SessionHost::start(&Handle::current(), CoreConfig::default(), paths.clone());

    assert!(matches!(second, Err(SessionError::LockHeld { .. })));
    host.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn stale_lock_with_dead_pid_is_reclaimed() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    fs::write(session_lock_path(&paths), "pid=999999\n").unwrap();

    let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths.clone())
        .expect("stale lock should not block startup");

    assert_eq!(file_mode(&session_lock_path(&paths)), 0o600);
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn new_subscriber_receives_snapshot_before_live_events() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths)
        .expect("session should start");

    let subscription = host.client().subscribe().await.unwrap();

    assert!(subscription.snapshot.transfers.is_empty());
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn history_loads_through_session_client() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths)
        .expect("session should start");

    let rows = host
        .client()
        .load_history(HistoryFilter::All, "")
        .await
        .unwrap();

    assert!(rows.is_empty());
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_artifact_after_finished_transfer_uses_session_state() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let config = CoreConfig::default_with_download_dir(dir.path().join("downloads"));
    let host = SessionHost::start(&Handle::current(), config, paths).expect("session should start");
    let client = host.client();
    let mut subscription = client.subscribe().await.unwrap();
    let destination = dir.path().join("finished.bin");
    let url = spawn_single_response_server(b"hello").await;

    let id = client
        .add(DownloadRequest {
            source: DownloadRequestSource::Http { url },
            destination: DownloadDestination::ExplicitPath(destination.clone()),
        })
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let SessionEvent::TransferChanged { snapshot } =
                subscription.next_event().await.unwrap()
                && snapshot.id == id
                && snapshot.status == DownloadStatus::Finished
            {
                break;
            }
        }
    })
    .await
    .unwrap();

    assert_eq!(fs::read(&destination).unwrap(), b"hello");
    client.delete_artifact(id).await.unwrap();

    let removed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let SessionEvent::TransferRemoved {
                id: removed_id,
                action,
                artifact_state,
            } = subscription.next_event().await.unwrap()
                && removed_id == id
            {
                return (action, artifact_state);
            }
        }
    })
    .await
    .unwrap();

    assert_eq!(removed.0, LiveTransferRemovalAction::DeleteArtifact);
    assert_eq!(removed.1, ArtifactState::Deleted);
    assert!(!destination.exists());

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let rows = client.load_history(HistoryFilter::All, "").await.unwrap();
            if rows
                .iter()
                .any(|row| row.id == id && row.artifact_state == ArtifactState::Deleted)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    host.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn socket_delete_artifact_after_finished_transfer_uses_session_state() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let config = CoreConfig::default_with_download_dir(dir.path().join("downloads"));
    let host = SessionHost::start(&Handle::current(), config, paths).expect("session should start");
    let client = socket_client(&host);
    let mut subscription = client.subscribe().await.unwrap();
    let destination = dir.path().join("socket-finished.bin");
    let url = spawn_single_response_server(b"socket").await;

    let id = client
        .add(DownloadRequest {
            source: DownloadRequestSource::Http { url },
            destination: DownloadDestination::ExplicitPath(destination.clone()),
        })
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let SessionEvent::TransferChanged { snapshot } =
                subscription.next_event().await.unwrap()
                && snapshot.id == id
                && snapshot.status == DownloadStatus::Finished
            {
                break;
            }
        }
    })
    .await
    .unwrap();

    assert_eq!(fs::read(&destination).unwrap(), b"socket");
    client.delete_artifact(id).await.unwrap();

    let removed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let SessionEvent::TransferRemoved {
                id: removed_id,
                action,
                artifact_state,
            } = subscription.next_event().await.unwrap()
                && removed_id == id
            {
                return (action, artifact_state);
            }
        }
    })
    .await
    .unwrap();

    assert_eq!(removed.0, LiveTransferRemovalAction::DeleteArtifact);
    assert_eq!(removed.1, ArtifactState::Deleted);
    assert!(!destination.exists());

    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn download_request_resolves_explicit_destination() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("picked.bin");
    let config = CoreConfig::default_with_download_dir(dir.path().join("downloads"));
    let request = DownloadRequest {
        source: DownloadRequestSource::Http {
            url: "https://example.com/file.bin".into(),
        },
        destination: DownloadDestination::ExplicitPath(destination.clone()),
    };

    let spec = request.into_spec(&config).unwrap();

    assert_eq!(spec.destination(), destination);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn unix_session_files_are_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths.clone())
        .expect("session should start");

    assert_eq!(file_mode(&session_dir(&paths)), 0o700);
    assert_eq!(file_mode(&session_lock_path(&paths)), 0o600);
    assert_eq!(file_mode(&session_descriptor_path(&paths)), 0o600);
    assert_eq!(file_mode(&session_socket_path(&paths)), 0o600);

    host.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn descriptor_is_not_written_when_socket_start_fails() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    fs::create_dir_all(session_socket_path(&paths)).unwrap();

    let result = SessionHost::start(&Handle::current(), CoreConfig::default(), paths.clone());

    assert!(matches!(result, Err(SessionError::Transport { .. })));
    assert!(!session_descriptor_path(&paths).exists());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn unix_socket_returns_snapshot_response() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths)
        .expect("session should start");
    let socket_path = host.descriptor().unwrap().socket_path.clone();

    let stream = tokio::net::UnixStream::connect(socket_path).await.unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let command = SessionWireCommand {
        id: 7,
        command: SessionCommand::Snapshot,
    };
    let mut body = serde_json::to_vec(&command).unwrap();
    body.push(b'\n');
    writer.write_all(&body).await.unwrap();

    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let frame: SessionWireFrame = serde_json::from_str(&line).unwrap();

    assert!(matches!(
        frame,
        SessionWireFrame::Response {
            id: 7,
            response: SessionResponse::Snapshot { .. }
        }
    ));
    host.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn socket_client_reads_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths)
        .expect("session should start");
    let client = socket_client(&host);

    let snapshot = client.snapshot().await.unwrap();

    assert!(snapshot.transfers.is_empty());
    host.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn socket_client_subscribe_gets_snapshot_then_live_events() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let config = CoreConfig::default_with_download_dir(dir.path().join("downloads"));
    let host = SessionHost::start(&Handle::current(), config, paths).expect("session should start");
    let client = socket_client(&host);
    let mut subscription = client.subscribe().await.unwrap();

    assert!(subscription.snapshot.transfers.is_empty());

    let destination = dir.path().join("socket-add.bin");
    let id = client
        .add(DownloadRequest {
            source: DownloadRequestSource::Http {
                url: "http://127.0.0.1:9/socket-add.bin".into(),
            },
            destination: DownloadDestination::ExplicitPath(destination),
        })
        .await
        .unwrap();

    let snapshot = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let SessionEvent::TransferChanged { snapshot } =
                subscription.next_event().await.unwrap()
                && snapshot.id == id
            {
                return snapshot;
            }
        }
    })
    .await
    .unwrap();

    assert_eq!(snapshot.id, id);
    host.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn socket_client_routes_control_commands() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths)
        .expect("session should start");
    let client = socket_client(&host);
    let missing = DownloadId(999_999);

    assert!(matches!(
        client.pause(missing).await,
        Err(SessionError::NotFound { id }) if id == missing
    ));
    assert!(matches!(
        client.resume(missing).await,
        Err(SessionError::NotFound { id }) if id == missing
    ));
    assert!(matches!(
        client.cancel(missing).await,
        Err(SessionError::NotFound { id }) if id == missing
    ));
    assert!(matches!(
        client.delete_artifact(missing).await,
        Err(SessionError::NotFound { id }) if id == missing
    ));
    client.update_config(CoreConfig::default()).await.unwrap();

    host.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn socket_client_reports_closed_after_server_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths)
        .expect("session should start");
    let client = socket_client(&host);

    host.shutdown().await.unwrap();

    assert!(matches!(client.snapshot().await, Err(SessionError::Closed)));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn raw_socket_shutdown_command_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths)
        .expect("session should start");
    let socket_path = host.descriptor().unwrap().socket_path.clone();

    let stream = tokio::net::UnixStream::connect(socket_path).await.unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    writer
        .write_all(br#"{"id":9,"command":"Shutdown"}"#)
        .await
        .unwrap();
    writer.write_all(b"\n").await.unwrap();

    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let frame: SessionWireFrame = serde_json::from_str(&line).unwrap();

    assert!(matches!(
        frame,
        SessionWireFrame::Error {
            id: 9,
            error: SessionError::BadRequest { .. }
        }
    ));
    assert!(host.client().snapshot().await.is_ok());

    host.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn unix_socket_reports_bad_json_as_bad_request() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths)
        .expect("session should start");
    let socket_path = host.descriptor().unwrap().socket_path.clone();

    let stream = tokio::net::UnixStream::connect(socket_path).await.unwrap();
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    writer.write_all(b"not json\n").await.unwrap();

    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let frame: SessionWireFrame = serde_json::from_str(&line).unwrap();

    assert!(matches!(
        frame,
        SessionWireFrame::Error {
            error: SessionError::BadRequest { .. },
            ..
        }
    ));
    host.shutdown().await.unwrap();
}
