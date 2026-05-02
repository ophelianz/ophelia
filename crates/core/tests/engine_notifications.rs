use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ophelia::engine::destination::part_path_for;
use ophelia::engine::{
    ArtifactState, DbEvent, DirectChunkMapState, LiveTransferRemovalAction,
    PersistedDownloadSource, RunnerResumeData, TransferControlSupport, TransferId, TransferKind,
    TransferStatus, TransferSummary,
};
use ophelia::service::{
    OpheliaClient, OpheliaError, OpheliaService, OpheliaSubscription, OpheliaUpdateBatch,
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
    resume_data: Option<RunnerResumeData>,
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
                .map(RunnerResumeData::downloaded_bytes)
                .unwrap_or(0),
            resume_data,
        })
        .unwrap();
    drop(bootstrap.db_tx);
    drop(bootstrap.worker);
}

async fn wait_for_matching_update(
    subscription: &mut OpheliaSubscription,
    mut predicate: impl FnMut(&OpheliaUpdateBatch) -> bool,
) -> OpheliaUpdateBatch {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let mut seen = Vec::new();

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            panic!(
                "timed out waiting for matching service update\nseen updates:\n{}",
                seen.join("\n")
            );
        }

        let update = tokio::time::timeout(remaining, subscription.next_update())
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "timed out waiting for matching service update: {error:?}\nseen updates:\n{}",
                    seen.join("\n")
                )
            })
            .expect("service update channel closed");
        let update_debug = format!("{update:?}");

        if predicate(&update) {
            return update;
        }

        seen.push(update_debug);
    }
}

async fn wait_for_matching_transfer(
    subscription: &mut OpheliaSubscription,
    mut predicate: impl FnMut(&TransferSummary) -> bool,
) -> TransferSummary {
    let mut cache: HashMap<TransferId, TransferSummary> = subscription
        .snapshot
        .transfers
        .summaries()
        .into_iter()
        .map(|snapshot| (snapshot.id, snapshot))
        .collect();

    loop {
        let update = wait_for_matching_update(subscription, |_| true).await;
        for snapshot in apply_update_to_transfer_cache(&mut cache, &update) {
            if predicate(&snapshot) {
                return snapshot;
            }
        }
    }
}

async fn wait_for_direct_detail(
    subscription: &mut OpheliaSubscription,
    id: TransferId,
    mut predicate: impl FnMut(&DirectChunkMapState) -> bool,
) -> DirectChunkMapState {
    let update = wait_for_matching_update(subscription, |update| {
        direct_details_contains(&update.direct_details, id)
            && predicate(&update.direct_details.state_for(id))
    })
    .await;
    update.direct_details.state_for(id)
}

fn direct_details_contains(table: &ophelia::service::DirectDetailsTable, id: TransferId) -> bool {
    table.unsupported_ids.contains(&id)
        || table.loading_ids.contains(&id)
        || table.segment_ids.contains(&id)
}

fn apply_update_to_transfer_cache(
    cache: &mut HashMap<TransferId, TransferSummary>,
    update: &OpheliaUpdateBatch,
) -> Vec<TransferSummary> {
    let mut changed = Vec::new();

    for snapshot in update.lifecycle.transfers.summaries() {
        cache.insert(snapshot.id, snapshot.clone());
        changed.push(snapshot);
    }

    for row in 0..update.progress_known_total.ids.len() {
        let id = update.progress_known_total.ids[row];
        let snapshot = cache.entry(id).or_insert_with(|| placeholder_summary(id));
        snapshot.status = TransferStatus::Downloading;
        snapshot.downloaded_bytes = update.progress_known_total.downloaded_bytes[row];
        snapshot.total_bytes = Some(update.progress_known_total.total_bytes[row]);
        snapshot.speed_bytes_per_sec = update.progress_known_total.speed_bytes_per_sec[row];
        changed.push(snapshot.clone());
    }

    for row in 0..update.progress_unknown_total.ids.len() {
        let id = update.progress_unknown_total.ids[row];
        let snapshot = cache.entry(id).or_insert_with(|| placeholder_summary(id));
        snapshot.status = TransferStatus::Downloading;
        snapshot.downloaded_bytes = update.progress_unknown_total.downloaded_bytes[row];
        snapshot.total_bytes = None;
        snapshot.speed_bytes_per_sec = update.progress_unknown_total.speed_bytes_per_sec[row];
        changed.push(snapshot.clone());
    }

    for row in 0..update.destination.ids.len() {
        let id = update.destination.ids[row];
        let snapshot = cache.entry(id).or_insert_with(|| placeholder_summary(id));
        snapshot.destination = update.destination.destinations[row].clone();
        changed.push(snapshot.clone());
    }

    for row in 0..update.control_support.ids.len() {
        let id = update.control_support.ids[row];
        let snapshot = cache.entry(id).or_insert_with(|| placeholder_summary(id));
        if let Some(support) = update.control_support.support(row) {
            snapshot.control_support = support;
        }
        changed.push(snapshot.clone());
    }

    for id in &update.removal.ids {
        cache.remove(id);
    }

    changed
}

fn placeholder_summary(id: TransferId) -> TransferSummary {
    TransferSummary {
        id,
        kind: TransferKind::Direct,
        provider_kind: "http".into(),
        source_label: String::new(),
        destination: PathBuf::new(),
        status: TransferStatus::Pending,
        downloaded_bytes: 0,
        total_bytes: None,
        speed_bytes_per_sec: 0,
        control_support: TransferControlSupport {
            can_pause: false,
            can_resume: false,
            can_cancel: false,
            can_restore: false,
        },
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
    let update = wait_for_matching_update(&mut harness.subscription, |update| {
        update.removal.ids.contains(&id)
    })
    .await;
    let row = update
        .removal
        .ids
        .iter()
        .position(|removed| *removed == id)
        .unwrap();
    assert_eq!(
        update.removal.action(row),
        Some(LiveTransferRemovalAction::Cancelled)
    );
    assert_eq!(
        update.removal.artifact_state(row),
        Some(ArtifactState::Missing)
    );

    let id = harness
        .client
        .add(http_request("https://example.com/file.bin", &destination))
        .await
        .unwrap();
    std::fs::write(&destination, b"partial").unwrap();
    harness.client.delete_artifact(id).await.unwrap();
    let update = wait_for_matching_update(&mut harness.subscription, |update| {
        update.removal.ids.contains(&id)
    })
    .await;
    let row = update
        .removal
        .ids
        .iter()
        .position(|removed| *removed == id)
        .unwrap();
    assert_eq!(
        update.removal.action(row),
        Some(LiveTransferRemovalAction::DeleteArtifact)
    );
    assert_eq!(
        update.removal.artifact_state(row),
        Some(ArtifactState::Deleted)
    );
    assert!(!destination.exists());
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
    assert_eq!(
        harness.subscription.snapshot.direct_details.state_for(id),
        DirectChunkMapState::Unsupported
    );
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
        .summaries()
        .into_iter()
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

    let update = wait_for_matching_update(&mut harness.subscription, |update| {
        update
            .physical_write
            .ids
            .iter()
            .zip(update.physical_write.bytes.iter())
            .any(|(changed, bytes)| *changed == id && *bytes > 0)
    })
    .await;
    let row = update
        .physical_write
        .ids
        .iter()
        .position(|changed| *changed == id)
        .unwrap();
    assert!(update.physical_write.bytes[row] > 0);
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
    let _ = wait_for_matching_update(&mut harness.subscription, |update| {
        for (changed, destination) in update
            .destination
            .ids
            .iter()
            .zip(update.destination.destinations.iter())
        {
            if *changed == id && *destination == server_destination {
                saw_destination_change = true;
            }
        }

        for snapshot in update.lifecycle.transfers.summaries() {
            if snapshot.id == id && snapshot.destination == server_destination {
                saw_destination_change = true;
            }
            if snapshot.id == id && snapshot.status == TransferStatus::Finished {
                return saw_destination_change;
            }
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

    let loading = wait_for_direct_detail(&mut harness.subscription, id, |state| {
        *state == DirectChunkMapState::Loading
    })
    .await;
    assert_eq!(loading, DirectChunkMapState::Loading);

    let http = wait_for_direct_detail(&mut harness.subscription, id, |state| {
        matches!(state, DirectChunkMapState::Segments(_))
    })
    .await;
    match http {
        DirectChunkMapState::Segments(snapshot) => assert_eq!(snapshot.cells.len(), 128),
        other => panic!("expected http chunk-map snapshot, got {other:?}"),
    }

    let terminal = wait_for_direct_detail(&mut harness.subscription, id, |state| {
        *state == DirectChunkMapState::Unsupported
    })
    .await;
    assert_eq!(terminal, DirectChunkMapState::Unsupported);
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

    let _ = wait_for_direct_detail(&mut harness.subscription, id, |state| {
        matches!(state, DirectChunkMapState::Segments(_))
    })
    .await;

    harness.client.pause(id).await.unwrap();
    let state = wait_for_direct_detail(&mut harness.subscription, id, |state| {
        *state == DirectChunkMapState::Unsupported
    })
    .await;
    assert_eq!(state, DirectChunkMapState::Unsupported);
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

    let _ = wait_for_direct_detail(&mut harness.subscription, first_id, |state| {
        matches!(state, DirectChunkMapState::Segments(_))
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
