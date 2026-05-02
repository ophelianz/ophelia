use std::path::{Path, PathBuf};
use std::time::Duration;

use ophelia::engine::destination::part_path_for;
use ophelia::engine::{
    ArtifactState, DbEvent, LiveTransferRemovalAction, PersistedDownloadSource, ProviderResumeData,
    TransferChunkMapState, TransferControlSupport, TransferId, TransferStatus, TransferSummary,
};
use ophelia::service::{
    OpheliaClient, OpheliaError, OpheliaEvent, OpheliaService, OpheliaSubscription,
    TransferDestination, TransferRequest, TransferRequestSource,
};
use ophelia::{ProfilePaths, ServiceSettings};
use tokio::runtime::Handle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct ServiceHarness {
    _profile: tempfile::TempDir,
    _service: OpheliaService,
    client: OpheliaClient,
    subscription: OpheliaSubscription,
}

fn profile_paths(profile: &tempfile::TempDir, downloads: &Path) -> ProfilePaths {
    ProfilePaths::new(profile.path().join("downloads.db"), downloads)
}

async fn start_service(max_concurrent_transfers: usize, downloads: &Path) -> ServiceHarness {
    let profile = tempfile::tempdir().unwrap();
    let paths = profile_paths(&profile, downloads);
    let mut settings = ServiceSettings::default_for_paths(&paths);
    settings.max_concurrent_transfers = max_concurrent_transfers;
    settings.default_download_dir = Some(downloads.to_path_buf());
    let service = OpheliaService::start_with_settings(&Handle::current(), paths, settings).unwrap();
    let client = service.client();
    let subscription = client.subscribe().await.unwrap();
    ServiceHarness {
        _profile: profile,
        _service: service,
        client,
        subscription,
    }
}

async fn start_service_with_paths(
    paths: ProfilePaths,
    profile: tempfile::TempDir,
    max_concurrent_transfers: usize,
) -> ServiceHarness {
    let mut settings = ServiceSettings::default_for_paths(&paths);
    settings.max_concurrent_transfers = max_concurrent_transfers;
    settings.default_download_dir = Some(paths.default_download_dir.clone());
    let service = OpheliaService::start_with_settings(&Handle::current(), paths, settings).unwrap();
    let client = service.client();
    let subscription = client.subscribe().await.unwrap();
    ServiceHarness {
        _profile: profile,
        _service: service,
        client,
        subscription,
    }
}

fn http_request(url: impl Into<String>, destination: impl Into<PathBuf>) -> TransferRequest {
    TransferRequest {
        source: TransferRequestSource::Http { url: url.into() },
        destination: TransferDestination::ExplicitPath(destination.into()),
    }
}

fn automatic_http_request(
    url: impl Into<String>,
    suggested_filename: Option<String>,
) -> TransferRequest {
    TransferRequest {
        source: TransferRequestSource::Http { url: url.into() },
        destination: TransferDestination::Automatic { suggested_filename },
    }
}

fn seed_paused_http_download(
    paths: &ProfilePaths,
    id: TransferId,
    url: String,
    destination: PathBuf,
    resume_data: Option<ProviderResumeData>,
) {
    let bootstrap = ophelia::engine::state::bootstrap(paths).unwrap();
    bootstrap
        .db_tx
        .send(DbEvent::Added {
            id,
            source: PersistedDownloadSource::Http { url },
            destination,
        })
        .unwrap();
    bootstrap
        .db_tx
        .send(DbEvent::Paused {
            id,
            downloaded_bytes: resume_data
                .as_ref()
                .map(ProviderResumeData::downloaded_bytes)
                .unwrap_or(0),
            resume_data,
        })
        .unwrap();
    drop(bootstrap.db_tx);
    drop(bootstrap.worker);
}

async fn wait_for_matching_event(
    subscription: &mut OpheliaSubscription,
    mut predicate: impl FnMut(&OpheliaEvent) -> bool,
) -> OpheliaEvent {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut seen = Vec::new();

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            panic!(
                "timed out waiting for matching service event\nseen events:\n{}",
                seen.join("\n")
            );
        }

        let event = tokio::time::timeout(remaining, subscription.next_event())
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "timed out waiting for matching service event: {error:?}\nseen events:\n{}",
                    seen.join("\n")
                )
            })
            .expect("service event channel closed");
        let event_debug = format!("{event:?}");

        if predicate(&event) {
            return event;
        }

        seen.push(event_debug);
    }
}

async fn wait_for_matching_transfer(
    subscription: &mut OpheliaSubscription,
    mut predicate: impl FnMut(&TransferSummary) -> bool,
) -> TransferSummary {
    match wait_for_matching_event(
        subscription,
        |event| matches!(event, OpheliaEvent::TransferChanged { snapshot } if predicate(snapshot)),
    )
    .await
    {
        OpheliaEvent::TransferChanged { snapshot } => snapshot,
        other => panic!("expected transfer update, got {other:?}"),
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
async fn queued_pause_resume_cancel_and_delete_emit_distinct_events() {
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let mut harness = start_service(0, tempdir.path()).await;

    let id = harness
        .client
        .add(http_request("https://example.com/file.bin", &destination))
        .await
        .unwrap();

    harness.client.pause(id).await.unwrap();
    let paused = wait_for_matching_transfer(&mut harness.subscription, |snapshot| {
        snapshot.id == id && snapshot.status == TransferStatus::Paused
    })
    .await;
    assert_eq!(paused.downloaded_bytes, 0);
    assert_eq!(paused.total_bytes, None);

    harness.client.resume(id).await.unwrap();
    let pending = wait_for_matching_transfer(&mut harness.subscription, |snapshot| {
        snapshot.id == id && snapshot.status == TransferStatus::Pending
    })
    .await;
    assert_eq!(pending.downloaded_bytes, 0);
    assert_eq!(pending.total_bytes, None);

    harness.client.cancel(id).await.unwrap();
    match wait_for_matching_event(&mut harness.subscription, |event| {
        matches!(
            event,
            OpheliaEvent::TransferRemoved { id: removed, .. } if *removed == id
        )
    })
    .await
    {
        OpheliaEvent::TransferRemoved {
            id: removed,
            action,
            artifact_state,
        } => {
            assert_eq!(removed, id);
            assert_eq!(action, LiveTransferRemovalAction::Cancelled);
            assert_eq!(artifact_state, ArtifactState::Missing);
        }
        other => panic!("expected cancelled removal event, got {other:?}"),
    }

    let id = harness
        .client
        .add(http_request("https://example.com/file.bin", &destination))
        .await
        .unwrap();
    std::fs::write(&destination, b"partial").unwrap();
    harness.client.delete_artifact(id).await.unwrap();
    match wait_for_matching_event(&mut harness.subscription, |event| {
        matches!(
            event,
            OpheliaEvent::TransferRemoved { id: removed, .. } if *removed == id
        )
    })
    .await
    {
        OpheliaEvent::TransferRemoved {
            id: removed,
            action,
            artifact_state,
        } => {
            assert_eq!(removed, id);
            assert_eq!(action, LiveTransferRemovalAction::DeleteArtifact);
            assert_eq!(artifact_state, ArtifactState::Deleted);
            assert!(!destination.exists());
        }
        other => panic!("expected removed event, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn add_emits_transfer_snapshot_for_frontends() {
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let mut harness = start_service(0, tempdir.path()).await;

    let id = harness
        .client
        .add(http_request("https://example.com/file.bin", &destination))
        .await
        .unwrap();

    let snapshot =
        wait_for_matching_transfer(&mut harness.subscription, |snapshot| snapshot.id == id).await;
    assert_eq!(snapshot.provider_kind, "http");
    assert_eq!(snapshot.source_label, "https://example.com/file.bin");
    assert_eq!(snapshot.destination, destination);
    assert_eq!(snapshot.status, TransferStatus::Pending);
    assert_eq!(snapshot.downloaded_bytes, 0);
    assert_eq!(snapshot.total_bytes, None);
    assert_eq!(snapshot.chunk_map_state, TransferChunkMapState::Unsupported);
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_returns_transfer_snapshot_for_frontends() {
    let profile = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let paths = profile_paths(&profile, downloads.path());
    let destination = downloads.path().join("file.bin");
    let id = TransferId(77);
    seed_paused_http_download(
        &paths,
        id,
        "https://example.com/file.bin".to_string(),
        destination.clone(),
        None,
    );

    let harness = start_service_with_paths(paths, profile, 3).await;
    let snapshot = harness
        .subscription
        .snapshot
        .transfers
        .iter()
        .find(|snapshot| snapshot.id == id)
        .expect("restored transfer was not in the initial snapshot");

    assert_eq!(snapshot.destination, destination);
    assert_eq!(snapshot.status, TransferStatus::Paused);
    assert_eq!(snapshot.downloaded_bytes, 0);
    assert_eq!(snapshot.total_bytes, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_delete_rejects_id_and_leaves_caller_path_alone() {
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("caller-owned.bin");
    std::fs::write(&destination, b"do not delete").unwrap();
    let harness = start_service(3, tempdir.path()).await;

    let err = harness
        .client
        .delete_artifact(TransferId(999))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        OpheliaError::NotFound {
            id: TransferId(999)
        }
    ));
    assert_eq!(std::fs::read(&destination).unwrap(), b"do not delete");
}

#[tokio::test(flavor = "multi_thread")]
async fn single_stream_http_emits_runtime_control_support_narrowing() {
    let server = spawn_no_content_length_server(vec![7u8; 2048]);
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let mut harness = start_service(3, tempdir.path()).await;
    let id = harness
        .client
        .add(http_request(
            format!("http://{server}/file.bin"),
            &destination,
        ))
        .await
        .unwrap();

    let snapshot = wait_for_matching_transfer(&mut harness.subscription, |snapshot| {
        snapshot.id == id
            && snapshot.control_support
                == TransferControlSupport {
                    can_pause: false,
                    can_resume: false,
                    can_cancel: true,
                    can_restore: false,
                }
    })
    .await;
    assert_eq!(snapshot.id, id);
}

#[tokio::test(flavor = "multi_thread")]
async fn single_stream_http_emits_download_bytes_written_event() {
    let server = spawn_no_content_length_server(vec![3u8; 4096]);
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let mut harness = start_service(3, tempdir.path()).await;
    let id = harness
        .client
        .add(http_request(
            format!("http://{server}/file.bin"),
            &destination,
        ))
        .await
        .unwrap();

    match wait_for_matching_event(&mut harness.subscription, |event| {
        matches!(event, OpheliaEvent::TransferBytesWritten { id: changed, bytes } if *changed == id && *bytes > 0)
    })
    .await
    {
        OpheliaEvent::TransferBytesWritten { id: changed, bytes } => {
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
                .insert_header("content-length", "2048")
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

    let tempdir = tempfile::tempdir().unwrap();
    let server_destination = tempdir.path().join("server.bin");
    let mut harness = start_service(3, tempdir.path()).await;
    let id = harness
        .client
        .add(automatic_http_request(
            format!("{}/file.bin", server.uri()),
            Some("file.bin".to_string()),
        ))
        .await
        .unwrap();

    let mut saw_destination_change = false;
    let _ = wait_for_matching_event(&mut harness.subscription, |event| {
        let OpheliaEvent::TransferChanged { snapshot } = event else {
            return false;
        };
        if snapshot.id != id {
            return false;
        }
        if snapshot.destination == server_destination {
            saw_destination_change = true;
        }
        if snapshot.status == TransferStatus::Finished {
            return saw_destination_change;
        }
        false
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_during_probe_before_single_stream_fallback_exits_cleanly() {
    let server = spawn_slow_no_content_length_server(vec![7u8; 2048], Duration::from_millis(100));
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let mut harness = start_service(3, tempdir.path()).await;
    let id = harness
        .client
        .add(http_request(
            format!("http://{server}/file.bin"),
            &destination,
        ))
        .await
        .unwrap();

    harness.client.pause(id).await.unwrap();

    let snapshot = wait_for_matching_transfer(&mut harness.subscription, |snapshot| {
        snapshot.id == id && snapshot.status == TransferStatus::Error
    })
    .await;
    assert_eq!(snapshot.downloaded_bytes, 0);
    assert!(!destination.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn chunked_http_emits_loading_snapshot_and_terminal_unsupported() {
    let server = spawn_slow_range_server(vec![5u8; 32 * 1024], 512, Duration::from_millis(25));
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let mut harness = start_service(3, tempdir.path()).await;
    let id = harness
        .client
        .add(http_request(
            format!("http://{server}/file.bin"),
            &destination,
        ))
        .await
        .unwrap();

    let loading = wait_for_matching_transfer(&mut harness.subscription, |snapshot| {
        snapshot.id == id && snapshot.chunk_map_state == TransferChunkMapState::Loading
    })
    .await;
    assert_eq!(loading.id, id);

    let http = wait_for_matching_transfer(&mut harness.subscription, |snapshot| {
        matches!(snapshot.chunk_map_state, TransferChunkMapState::Http(_)) && snapshot.id == id
    })
    .await;
    match http.chunk_map_state {
        TransferChunkMapState::Http(snapshot) => assert_eq!(snapshot.cells.len(), 128),
        other => panic!("expected http chunk-map snapshot, got {other:?}"),
    }

    let terminal = wait_for_matching_transfer(&mut harness.subscription, |snapshot| {
        snapshot.id == id && snapshot.chunk_map_state == TransferChunkMapState::Unsupported
    })
    .await;
    assert_eq!(terminal.id, id);
}

#[tokio::test(flavor = "multi_thread")]
async fn pausing_active_http_clears_chunk_map_to_unsupported() {
    let server = spawn_slow_range_server(vec![9u8; 32 * 1024], 512, Duration::from_millis(25));
    let tempdir = tempfile::tempdir().unwrap();
    let destination = tempdir.path().join("file.bin");
    let mut harness = start_service(3, tempdir.path()).await;
    let id = harness
        .client
        .add(http_request(
            format!("http://{server}/file.bin"),
            &destination,
        ))
        .await
        .unwrap();

    let _ = wait_for_matching_transfer(&mut harness.subscription, |snapshot| {
        matches!(snapshot.chunk_map_state, TransferChunkMapState::Http(_)) && snapshot.id == id
    })
    .await;

    harness.client.pause(id).await.unwrap();
    let snapshot = wait_for_matching_transfer(&mut harness.subscription, |snapshot| {
        snapshot.id == id && snapshot.chunk_map_state == TransferChunkMapState::Unsupported
    })
    .await;
    assert_eq!(snapshot.id, id);
}

#[tokio::test(flavor = "multi_thread")]
async fn pausing_active_http_starts_next_queued_download() {
    let first_server =
        spawn_slow_range_server(vec![1u8; 128 * 1024], 512, Duration::from_millis(25));
    let second_server =
        spawn_slow_range_server(vec![2u8; 32 * 1024], 512, Duration::from_millis(25));

    let tempdir = tempfile::tempdir().unwrap();
    let first_destination = tempdir.path().join("first.bin");
    let second_destination = tempdir.path().join("second.bin");
    let mut harness = start_service(1, tempdir.path()).await;

    let first_id = harness
        .client
        .add(http_request(
            format!("http://{first_server}/first.bin"),
            &first_destination,
        ))
        .await
        .unwrap();
    let second_id = harness
        .client
        .add(http_request(
            format!("http://{second_server}/second.bin"),
            &second_destination,
        ))
        .await
        .unwrap();

    let _ = wait_for_matching_transfer(&mut harness.subscription, |snapshot| {
        matches!(snapshot.chunk_map_state, TransferChunkMapState::Http(_))
            && snapshot.id == first_id
    })
    .await;

    harness.client.pause(first_id).await.unwrap();

    let snapshot = wait_for_matching_transfer(&mut harness.subscription, |snapshot| {
        snapshot.id == second_id && snapshot.status == TransferStatus::Downloading
    })
    .await;
    assert_eq!(snapshot.id, second_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn restored_http_without_resume_data_discards_stale_part_file_before_restart() {
    let data = vec![8u8; 32 * 1024];
    let server = spawn_slow_range_server(data.clone(), 8192, Duration::from_millis(0));

    let profile = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let paths = profile_paths(&profile, downloads.path());
    let destination = downloads.path().join("file.bin");
    let part_path = part_path_for(&destination);
    std::fs::write(&part_path, b"stale partial bytes").unwrap();

    let id = TransferId(77);
    seed_paused_http_download(
        &paths,
        id,
        format!("http://{server}/file.bin"),
        destination.clone(),
        None,
    );

    let mut harness = start_service_with_paths(paths, profile, 3).await;
    harness.client.resume(id).await.unwrap();

    let snapshot = wait_for_matching_transfer(&mut harness.subscription, |snapshot| {
        snapshot.id == id && snapshot.status == TransferStatus::Finished
    })
    .await;
    assert_eq!(snapshot.downloaded_bytes, data.len() as u64);
    assert_eq!(std::fs::read(&destination).unwrap(), data);
    assert!(!part_path.exists());
}
