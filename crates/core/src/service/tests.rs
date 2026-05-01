use super::host::should_flush_before_immediate;
use super::read_model::{OpheliaEventCoalescer, OpheliaReadModel};
use super::wire::{
    OpheliaWireCommand, OpheliaWireFrame, command_from_payload, command_to_payload,
    frame_from_payload, frame_to_payload,
};
use super::*;
use crate::engine::{TransferChunkMapState, TransferControlSupport, TransferStatus};
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn test_paths(dir: &TempDir) -> ProfilePaths {
    ProfilePaths::new(
        dir.path().join("downloads.db"),
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

fn test_snapshot(id: TransferId, status: TransferStatus) -> TransferSummary {
    TransferSummary {
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
    let id = TransferId(1);
    let mut read_model = OpheliaReadModel::default();
    let mut coalescer = OpheliaEventCoalescer::default();

    let added = read_model
        .apply_engine_event(
            EngineEvent::TransferAdded {
                snapshot: test_snapshot(id, TransferStatus::Pending),
            },
            &mut coalescer,
        )
        .unwrap();
    assert!(matches!(added, OpheliaEvent::TransferChanged { .. }));

    assert!(
        read_model
            .apply_engine_event(
                EngineEvent::Progress(ProgressUpdate {
                    id,
                    status: TransferStatus::Downloading,
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
                EngineEvent::TransferBytesWritten { id, bytes: 10 },
                &mut coalescer,
            )
            .is_none()
    );
    assert!(
        read_model
            .apply_engine_event(
                EngineEvent::TransferBytesWritten { id, bytes: 15 },
                &mut coalescer,
            )
            .is_none()
    );

    let events = coalescer.drain_events();
    assert_eq!(events.len(), 2);
    match &events[0] {
        OpheliaEvent::TransferChanged { snapshot } => {
            assert_eq!(snapshot.downloaded_bytes, 40);
            assert_eq!(snapshot.chunk_map_state, TransferChunkMapState::Loading);
        }
        event => panic!("expected transfer update, got {event:?}"),
    }
    match &events[1] {
        OpheliaEvent::TransferBytesWritten {
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
    let id = TransferId(2);
    let mut read_model = OpheliaReadModel::default();
    let mut coalescer = OpheliaEventCoalescer::default();
    let _ = read_model.apply_engine_event(
        EngineEvent::TransferAdded {
            snapshot: test_snapshot(id, TransferStatus::Pending),
        },
        &mut coalescer,
    );

    for downloaded_bytes in [10, 20, 30] {
        let _ = read_model.apply_engine_event(
            EngineEvent::Progress(ProgressUpdate {
                id,
                status: TransferStatus::Downloading,
                downloaded_bytes,
                total_bytes: Some(100),
                speed_bytes_per_sec: downloaded_bytes,
            }),
            &mut coalescer,
        );
    }
    let _ = read_model.apply_engine_event(
        EngineEvent::TransferBytesWritten { id, bytes: 10 },
        &mut coalescer,
    );
    let _ = read_model.apply_engine_event(
        EngineEvent::TransferBytesWritten { id, bytes: 20 },
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
    let id = TransferId(3);
    let mut read_model = OpheliaReadModel::default();
    let mut coalescer = OpheliaEventCoalescer::default();
    let _ = read_model.apply_engine_event(
        EngineEvent::TransferAdded {
            snapshot: test_snapshot(id, TransferStatus::Pending),
        },
        &mut coalescer,
    );
    let _ = read_model.apply_engine_event(
        EngineEvent::Progress(ProgressUpdate {
            id,
            status: TransferStatus::Downloading,
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
                status: TransferStatus::Finished,
                downloaded_bytes: 100,
                total_bytes: Some(100),
                speed_bytes_per_sec: 0,
            }),
            &mut coalescer,
        )
        .unwrap();

    match finished {
        OpheliaEvent::TransferChanged { snapshot } => {
            assert_eq!(snapshot.status, TransferStatus::Finished);
            assert_eq!(snapshot.downloaded_bytes, 100);
        }
        event => panic!("expected finished transfer update, got {event:?}"),
    }
    assert!(coalescer.drain_events().is_empty());
}

#[test]
fn snapshot_reflects_pending_hot_updates_before_flush() {
    let id = TransferId(4);
    let mut read_model = OpheliaReadModel::default();
    let mut coalescer = OpheliaEventCoalescer::default();
    let _ = read_model.apply_engine_event(
        EngineEvent::TransferAdded {
            snapshot: test_snapshot(id, TransferStatus::Pending),
        },
        &mut coalescer,
    );
    let _ = read_model.apply_engine_event(
        EngineEvent::Progress(ProgressUpdate {
            id,
            status: TransferStatus::Downloading,
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

    let snapshot = read_model.snapshot(&ServiceSettings::default());

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
        &EngineEvent::TransferBytesWritten {
            id: TransferId(5),
            bytes: 8,
        }
    ));
    assert!(!should_flush_before_immediate(&EngineEvent::Progress(
        ProgressUpdate {
            id: TransferId(5),
            status: TransferStatus::Downloading,
            downloaded_bytes: 8,
            total_bytes: Some(100),
            speed_bytes_per_sec: 8,
        }
    )));
    assert!(should_flush_before_immediate(&EngineEvent::Progress(
        ProgressUpdate {
            id: TransferId(5),
            status: TransferStatus::Finished,
            downloaded_bytes: 100,
            total_bytes: Some(100),
            speed_bytes_per_sec: 0,
        }
    )));
    assert!(should_flush_before_immediate(
        &EngineEvent::LiveTransferRemoved {
            id: TransferId(5),
            action: LiveTransferRemovalAction::DeleteArtifact,
            artifact_state: ArtifactState::Deleted,
        }
    ));
}

#[test]
fn terminal_progress_keeps_pending_write_bytes() {
    let id = TransferId(6);
    let mut read_model = OpheliaReadModel::default();
    let mut coalescer = OpheliaEventCoalescer::default();
    let _ = read_model.apply_engine_event(
        EngineEvent::TransferAdded {
            snapshot: test_snapshot(id, TransferStatus::Pending),
        },
        &mut coalescer,
    );
    let _ = read_model.apply_engine_event(
        EngineEvent::TransferBytesWritten { id, bytes: 32 },
        &mut coalescer,
    );

    let finished = read_model.apply_engine_event(
        EngineEvent::Progress(ProgressUpdate {
            id,
            status: TransferStatus::Finished,
            downloaded_bytes: 100,
            total_bytes: Some(100),
            speed_bytes_per_sec: 0,
        }),
        &mut coalescer,
    );

    assert!(matches!(
        finished,
        Some(OpheliaEvent::TransferChanged { .. })
    ));
    assert!(matches!(
        coalescer.drain_events().as_slice(),
        [OpheliaEvent::TransferBytesWritten { id: event_id, bytes }]
            if *event_id == id && *bytes == 32
    ));
}

#[test]
fn service_install_kind_is_inferred_from_owner_binary_path() {
    assert_eq!(
        infer_install_kind(Some(Path::new(
            "/Applications/Ophelia.app/Contents/MacOS/ophelia-service"
        ))),
        OpheliaInstallKind::AppBundle
    );
    assert_eq!(
        infer_install_kind(Some(Path::new(
            "/opt/homebrew/Cellar/ophelia/0.1.0/bin/ophelia-service"
        ))),
        OpheliaInstallKind::HomebrewFormula
    );
    assert_eq!(
        infer_install_kind(Some(Path::new(
            "/Users/me/src/ophelia/target/debug/ophelia-service"
        ))),
        OpheliaInstallKind::Development
    );
    assert_eq!(infer_install_kind(None), OpheliaInstallKind::Unknown);
}

#[tokio::test(flavor = "multi_thread")]
async fn session_host_rejects_second_owner_for_profile() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = OpheliaService::start_with_engine_config(
        &Handle::current(),
        EngineConfig::default(),
        paths.clone(),
    )
    .expect("first session should start");

    let second = OpheliaService::start_with_engine_config(
        &Handle::current(),
        EngineConfig::default(),
        paths.clone(),
    );

    assert!(matches!(second, Err(OpheliaError::LockHeld { .. })));
    host.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn stale_lock_with_dead_pid_is_reclaimed() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    fs::write(service_lock_path(&paths), "pid=999999\n").unwrap();

    let host = OpheliaService::start_with_engine_config(
        &Handle::current(),
        EngineConfig::default(),
        paths.clone(),
    )
    .expect("stale lock should not block startup");

    assert_eq!(file_mode(&service_lock_path(&paths)), 0o600);
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn new_subscriber_receives_snapshot_before_live_events() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = OpheliaService::start_with_engine_config(
        &Handle::current(),
        EngineConfig::default(),
        paths,
    )
    .expect("session should start");

    let subscription = host.client().subscribe().await.unwrap();

    assert!(subscription.snapshot.transfers.is_empty());
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn history_loads_through_session_client() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = OpheliaService::start_with_engine_config(
        &Handle::current(),
        EngineConfig::default(),
        paths,
    )
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
async fn service_info_reports_owner_and_profile_paths() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = OpheliaService::start_with_engine_config(
        &Handle::current(),
        EngineConfig::default(),
        paths.clone(),
    )
    .expect("service should start");

    let info = host.client().service_info().await.unwrap();

    assert_eq!(info.service_name, OPHELIA_MACH_SERVICE_NAME);
    assert_eq!(info.profile.database_path, paths.database_path);
    assert_eq!(info.profile.settings_path, paths.settings_path);
    assert_eq!(info.profile.service_lock_path, paths.service_lock_path);
    assert_eq!(info.owner.pid, std::process::id());
    assert!(info.owner.executable.is_some());
    host.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_artifact_after_finished_transfer_uses_session_state() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let config = EngineConfig::default_with_download_dir(dir.path().join("downloads"));
    let host = OpheliaService::start_with_engine_config(&Handle::current(), config, paths)
        .expect("session should start");
    let client = host.client();
    let mut subscription = client.subscribe().await.unwrap();
    let destination = dir.path().join("finished.bin");
    let url = spawn_single_response_server(b"hello").await;

    let id = client
        .add(TransferRequest {
            source: TransferRequestSource::Http { url },
            destination: TransferDestination::ExplicitPath(destination.clone()),
        })
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let OpheliaEvent::TransferChanged { snapshot } =
                subscription.next_event().await.unwrap()
                && snapshot.id == id
                && snapshot.status == TransferStatus::Finished
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
            if let OpheliaEvent::TransferRemoved {
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

#[tokio::test(flavor = "multi_thread")]
async fn download_request_resolves_explicit_destination() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("picked.bin");
    let config = EngineConfig::default_with_download_dir(dir.path().join("downloads"));
    let request = TransferRequest {
        source: TransferRequestSource::Http {
            url: "https://example.com/file.bin".into(),
        },
        destination: TransferDestination::ExplicitPath(destination.clone()),
    };

    let spec = request.into_spec(&config).unwrap();

    assert_eq!(spec.destination(), destination);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn service_lock_and_data_dir_are_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = OpheliaService::start_with_engine_config(
        &Handle::current(),
        EngineConfig::default(),
        paths.clone(),
    )
    .expect("service should start");

    assert_eq!(file_mode(&paths.data_dir), 0o700);
    assert_eq!(file_mode(&service_lock_path(&paths)), 0o600);

    host.shutdown().await.unwrap();
}

#[test]
fn wire_command_payload_roundtrips_snapshot() {
    let command = OpheliaWireCommand {
        id: 7,
        command: OpheliaCommand::Snapshot,
    };

    let payload = command_to_payload(&command).unwrap();
    let decoded = command_from_payload(&payload).unwrap();

    assert!(matches!(decoded.command, OpheliaCommand::Snapshot));
    assert_eq!(decoded.id, 7);
}

#[test]
fn wire_frame_payload_roundtrips_response() {
    let frame = OpheliaWireFrame::Response {
        id: 8,
        response: OpheliaResponse::Snapshot {
            snapshot: OpheliaSnapshot::default(),
        },
    };

    let payload = frame_to_payload(&frame).unwrap();
    let decoded = frame_from_payload(&payload).unwrap();

    assert!(matches!(
        decoded,
        OpheliaWireFrame::Response {
            id: 8,
            response: OpheliaResponse::Snapshot { .. }
        }
    ));
}

#[test]
fn wire_frame_payload_roundtrips_service_info() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let frame = OpheliaWireFrame::Response {
        id: 9,
        response: OpheliaResponse::ServiceInfo {
            info: OpheliaServiceInfo::current(&paths),
        },
    };

    let payload = frame_to_payload(&frame).unwrap();
    let decoded = frame_from_payload(&payload).unwrap();

    assert!(matches!(
        decoded,
        OpheliaWireFrame::Response {
            id: 9,
            response: OpheliaResponse::ServiceInfo { .. }
        }
    ));
}

#[test]
fn malformed_wire_command_is_bad_request() {
    let error = command_from_payload(b"not json").unwrap_err();

    assert!(matches!(error, OpheliaError::BadRequest { .. }));
}
