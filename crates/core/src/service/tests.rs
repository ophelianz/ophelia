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

use super::codec::{
    OpheliaCommandEnvelope, OpheliaFrameEnvelope, command_from_body, command_to_body,
    frame_from_body, frame_to_body,
};
use super::host::should_flush_before_immediate;
use super::read_model::{OpheliaReadModel, OpheliaUpdateBuilder};
use super::transfer_runtime::TransferRuntimeEvent;
use super::*;
use crate::engine::{
    DirectChunkMapState, TransferControlSupport, TransferDetails, TransferKind, TransferStatus,
};
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
        kind: TransferKind::Direct,
        provider_kind: "http".into(),
        source_label: "https://example.com/file.bin".into(),
        destination: PathBuf::from("file.bin"),
        status,
        downloaded_bytes: 0,
        total_bytes: Some(100),
        speed_bytes_per_sec: 0,
        control_support: TransferControlSupport::all(),
    }
}

#[test]
fn transfer_summary_table_stores_known_totals_as_dense_bits() {
    let mut table = TransferSummaryTable::default();
    let mut unknown = test_snapshot(TransferId(2), TransferStatus::Downloading);
    unknown.total_bytes = None;

    table.push_summary(test_snapshot(TransferId(1), TransferStatus::Downloading));
    table.push_summary(unknown);
    table.push_summary(test_snapshot(TransferId(3), TransferStatus::Downloading));

    assert_eq!(table.total_bytes(0), Some(100));
    assert_eq!(table.total_bytes(1), None);
    assert_eq!(table.total_bytes(2), Some(100));
    assert_eq!(table.total_bytes, vec![100, 0, 100]);
    assert_eq!(table.known_total_words[0] & 0b111, 0b101);

    table.set_total(1, Some(200));
    assert_eq!(table.total_bytes(1), Some(200));
    assert_eq!(table.known_total_words[0] & 0b111, 0b111);

    table.set_total(0, None);
    assert_eq!(table.total_bytes(0), None);
    assert_eq!(table.total_bytes[0], 0);
    assert_eq!(table.known_total_words[0] & 0b111, 0b110);
}

#[test]
fn transfer_summary_table_remove_row_preserves_total_membership() {
    let mut table = TransferSummaryTable::default();
    for id in 0..4 {
        let mut snapshot = test_snapshot(TransferId(id), TransferStatus::Downloading);
        if id == 1 {
            snapshot.total_bytes = None;
        } else {
            snapshot.total_bytes = Some(100 + id);
        }
        table.push_summary(snapshot);
    }

    table.remove_row(1);

    assert_eq!(table.total_bytes(0), Some(100));
    assert_eq!(table.total_bytes(1), Some(102));
    assert_eq!(table.total_bytes(2), Some(103));
    assert_eq!(table.known_total_words[0] & 0b111, 0b111);
}

#[test]
fn service_update_builder_coalesces_hot_batches_by_family() {
    let id = TransferId(1);
    let mut read_model = OpheliaReadModel::default();
    let mut builder = OpheliaUpdateBuilder::default();

    read_model.apply_transfer_event(
        TransferRuntimeEvent::TransferAdded {
            snapshot: test_snapshot(id, TransferStatus::Pending),
        },
        &mut builder,
    );
    read_model.apply_transfer_event(
        TransferRuntimeEvent::Progress(ProgressUpdate {
            id,
            status: TransferStatus::Downloading,
            downloaded_bytes: 40,
            total_bytes: Some(100),
            speed_bytes_per_sec: 10,
        }),
        &mut builder,
    );
    read_model.apply_transfer_event(
        TransferRuntimeEvent::DetailsChanged {
            id,
            details: TransferDetails::direct(DirectChunkMapState::Loading),
        },
        &mut builder,
    );
    read_model.apply_transfer_event(
        TransferRuntimeEvent::TransferBytesWritten { id, bytes: 10 },
        &mut builder,
    );
    read_model.apply_transfer_event(
        TransferRuntimeEvent::TransferBytesWritten { id, bytes: 15 },
        &mut builder,
    );

    let batch = builder.drain_batch().unwrap();
    assert_eq!(batch.lifecycle.lifecycle_codes.len(), 1);
    assert_eq!(batch.progress_known_total.ids, vec![id]);
    assert_eq!(batch.progress_known_total.downloaded_bytes, vec![40]);
    assert_eq!(batch.progress_known_total.total_bytes, vec![100]);
    assert_eq!(batch.physical_write.bytes, vec![25]);
    assert_eq!(batch.direct_details.loading_ids, vec![id]);
}

#[test]
fn coalescer_stats_count_raw_and_emitted_hot_events() {
    let id = TransferId(2);
    let mut read_model = OpheliaReadModel::default();
    let mut builder = OpheliaUpdateBuilder::default();
    read_model.apply_transfer_event(
        TransferRuntimeEvent::TransferAdded {
            snapshot: test_snapshot(id, TransferStatus::Pending),
        },
        &mut builder,
    );
    let _ = builder.drain_batch();

    for downloaded_bytes in [10, 20, 30] {
        read_model.apply_transfer_event(
            TransferRuntimeEvent::Progress(ProgressUpdate {
                id,
                status: TransferStatus::Downloading,
                downloaded_bytes,
                total_bytes: Some(100),
                speed_bytes_per_sec: downloaded_bytes,
            }),
            &mut builder,
        );
    }
    read_model.apply_transfer_event(
        TransferRuntimeEvent::TransferBytesWritten { id, bytes: 10 },
        &mut builder,
    );
    read_model.apply_transfer_event(
        TransferRuntimeEvent::TransferBytesWritten { id, bytes: 20 },
        &mut builder,
    );

    let batch = builder.drain_batch().unwrap();
    let stats = builder.stats();

    assert_eq!(batch.progress_known_total.ids.len(), 1);
    assert_eq!(batch.physical_write.ids.len(), 1);
    assert_eq!(stats.raw_transfer_updates, 4);
    assert_eq!(stats.raw_write_updates, 2);
    assert_eq!(stats.emitted_transfer_updates, 2);
    assert_eq!(stats.emitted_write_updates, 1);
    assert_eq!(stats.coalesced_transfer_updates(), 2);
    assert_eq!(stats.coalesced_write_updates(), 1);
}

#[test]
fn terminal_progress_clears_stale_coalesced_updates() {
    let id = TransferId(3);
    let mut read_model = OpheliaReadModel::default();
    let mut builder = OpheliaUpdateBuilder::default();
    read_model.apply_transfer_event(
        TransferRuntimeEvent::TransferAdded {
            snapshot: test_snapshot(id, TransferStatus::Pending),
        },
        &mut builder,
    );
    let _ = builder.drain_batch();
    read_model.apply_transfer_event(
        TransferRuntimeEvent::Progress(ProgressUpdate {
            id,
            status: TransferStatus::Downloading,
            downloaded_bytes: 50,
            total_bytes: Some(100),
            speed_bytes_per_sec: 10,
        }),
        &mut builder,
    );

    read_model.apply_transfer_event(
        TransferRuntimeEvent::Progress(ProgressUpdate {
            id,
            status: TransferStatus::Finished,
            downloaded_bytes: 100,
            total_bytes: Some(100),
            speed_bytes_per_sec: 0,
        }),
        &mut builder,
    );

    let batch = builder.drain_batch().unwrap();
    assert!(batch.progress_known_total.is_empty());
    assert_eq!(
        batch.lifecycle.transfers.summaries()[0].status,
        TransferStatus::Finished
    );
    assert_eq!(
        batch.lifecycle.transfers.summaries()[0].downloaded_bytes,
        100
    );
}

#[test]
fn snapshot_reflects_pending_hot_updates_before_flush() {
    let id = TransferId(4);
    let mut read_model = OpheliaReadModel::default();
    let mut builder = OpheliaUpdateBuilder::default();
    read_model.apply_transfer_event(
        TransferRuntimeEvent::TransferAdded {
            snapshot: test_snapshot(id, TransferStatus::Pending),
        },
        &mut builder,
    );
    read_model.apply_transfer_event(
        TransferRuntimeEvent::Progress(ProgressUpdate {
            id,
            status: TransferStatus::Downloading,
            downloaded_bytes: 80,
            total_bytes: Some(100),
            speed_bytes_per_sec: 12,
        }),
        &mut builder,
    );
    read_model.apply_transfer_event(
        TransferRuntimeEvent::DetailsChanged {
            id,
            details: TransferDetails::direct(DirectChunkMapState::Loading),
        },
        &mut builder,
    );

    let snapshot = read_model.snapshot(&ServiceSettings::default());

    assert_eq!(snapshot.transfers.len(), 1);
    assert_eq!(snapshot.transfers.summaries()[0].downloaded_bytes, 80);
    assert_eq!(
        snapshot.direct_details.state_for(id),
        DirectChunkMapState::Loading
    );
}

#[test]
fn terminal_events_flush_hot_updates_first() {
    assert!(!should_flush_before_immediate(
        &TransferRuntimeEvent::TransferBytesWritten {
            id: TransferId(5),
            bytes: 8,
        }
    ));
    assert!(!should_flush_before_immediate(
        &TransferRuntimeEvent::Progress(ProgressUpdate {
            id: TransferId(5),
            status: TransferStatus::Downloading,
            downloaded_bytes: 8,
            total_bytes: Some(100),
            speed_bytes_per_sec: 8,
        })
    ));
    assert!(should_flush_before_immediate(
        &TransferRuntimeEvent::Progress(ProgressUpdate {
            id: TransferId(5),
            status: TransferStatus::Finished,
            downloaded_bytes: 100,
            total_bytes: Some(100),
            speed_bytes_per_sec: 0,
        })
    ));
    assert!(should_flush_before_immediate(
        &TransferRuntimeEvent::TransferRemoved {
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
    let mut builder = OpheliaUpdateBuilder::default();
    read_model.apply_transfer_event(
        TransferRuntimeEvent::TransferAdded {
            snapshot: test_snapshot(id, TransferStatus::Pending),
        },
        &mut builder,
    );
    let _ = builder.drain_batch();
    read_model.apply_transfer_event(
        TransferRuntimeEvent::TransferBytesWritten { id, bytes: 32 },
        &mut builder,
    );

    read_model.apply_transfer_event(
        TransferRuntimeEvent::Progress(ProgressUpdate {
            id,
            status: TransferStatus::Finished,
            downloaded_bytes: 100,
            total_bytes: Some(100),
            speed_bytes_per_sec: 0,
        }),
        &mut builder,
    );

    let batch = builder.drain_batch().unwrap();
    assert_eq!(batch.physical_write.ids, vec![id]);
    assert_eq!(batch.physical_write.bytes, vec![32]);
    assert_eq!(
        batch.lifecycle.transfers.summaries()[0].status,
        TransferStatus::Finished
    );
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
async fn service_idle_exit_closes_backend_when_no_clients_or_running_transfers() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = OpheliaService::start_with_engine_config_and_idle_timeout(
        &Handle::current(),
        EngineConfig::default(),
        paths,
        Duration::from_millis(20),
    )
    .expect("session should start");
    let client = host.client();

    tokio::time::sleep(Duration::from_millis(80)).await;

    assert!(matches!(client.snapshot().await, Err(OpheliaError::Closed)));
    host.wait().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn service_subscription_keeps_idle_backend_alive() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let host = OpheliaService::start_with_engine_config_and_idle_timeout(
        &Handle::current(),
        EngineConfig::default(),
        paths,
        Duration::from_millis(20),
    )
    .expect("session should start");
    let client = host.client();
    let subscription = client.subscribe().await.unwrap();

    tokio::time::sleep(Duration::from_millis(80)).await;

    assert!(client.snapshot().await.is_ok());
    drop(subscription);
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(matches!(client.snapshot().await, Err(OpheliaError::Closed)));
    host.wait().await;
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
    assert_eq!(info.helper.pid, std::process::id());
    assert_eq!(info.helper.executable, info.owner.executable);
    assert!(info.helper.executable_sha256.is_some());
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
            let update = subscription.next_update().await.unwrap();
            if update
                .lifecycle
                .transfers
                .summaries()
                .iter()
                .any(|snapshot| snapshot.id == id && snapshot.status == TransferStatus::Finished)
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
            let update = subscription.next_update().await.unwrap();
            if let Some(row) = update
                .removal
                .ids
                .iter()
                .position(|removed_id| *removed_id == id)
            {
                return (
                    update.removal.action(row).unwrap(),
                    update.removal.artifact_state(row).unwrap(),
                );
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
fn xpc_command_body_roundtrips_snapshot() {
    let command = OpheliaCommandEnvelope {
        id: 7,
        command: OpheliaCommand::Snapshot,
    };

    let body = command_to_body(&command).unwrap();
    let decoded = command_from_body(&body).unwrap();

    assert!(matches!(decoded.command, OpheliaCommand::Snapshot));
    assert_eq!(decoded.id, 7);
}

#[test]
fn xpc_frame_body_roundtrips_response() {
    let frame = OpheliaFrameEnvelope::Response {
        id: 8,
        response: Box::new(OpheliaResponse::Snapshot {
            snapshot: Box::new(OpheliaSnapshot::default()),
        }),
    };

    let body = frame_to_body(&frame).unwrap();
    let decoded = frame_from_body(&body).unwrap();

    assert!(matches!(
        decoded,
        OpheliaFrameEnvelope::Response {
            id: 8,
            response
        } if matches!(*response, OpheliaResponse::Snapshot { .. })
    ));
}

#[test]
fn xpc_frame_body_roundtrips_service_info() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let frame = OpheliaFrameEnvelope::Response {
        id: 9,
        response: Box::new(OpheliaResponse::ServiceInfo {
            info: Box::new(OpheliaServiceInfo::current(&paths)),
        }),
    };

    let body = frame_to_body(&frame).unwrap();
    let decoded = frame_from_body(&body).unwrap();

    assert!(matches!(
        decoded,
        OpheliaFrameEnvelope::Response {
            id: 9,
            response
        } if matches!(*response, OpheliaResponse::ServiceInfo { .. })
    ));
}

#[test]
fn xpc_frame_body_roundtrips_update_batch() {
    let id = TransferId(11);
    let mut transfers = TransferSummaryTable::default();
    transfers.push_summary(test_snapshot(id, TransferStatus::Downloading));

    let mut direct_details = DirectDetailsTable::default();
    direct_details.push_state(id, DirectChunkMapState::Loading);

    let batch = OpheliaUpdateBatch {
        lifecycle: TransferLifecycleBatch {
            transfers,
            lifecycle_codes: vec![TransferLifecycleCode::Added as u8],
        },
        progress_known_total: ProgressKnownTotalBatch {
            ids: vec![id],
            downloaded_bytes: vec![10],
            total_bytes: vec![100],
            speed_bytes_per_sec: vec![5],
        },
        direct_details,
        ..OpheliaUpdateBatch::default()
    };
    let frame = OpheliaFrameEnvelope::Update {
        update: Box::new(batch.clone()),
    };

    let body = frame_to_body(&frame).unwrap();
    let decoded = frame_from_body(&body).unwrap();

    assert!(matches!(
        decoded,
        OpheliaFrameEnvelope::Update { update } if *update == batch
    ));
}

#[test]
fn xpc_frame_body_roundtrips_approval_required_error() {
    let frame = OpheliaFrameEnvelope::Error {
        id: 10,
        error: OpheliaError::ServiceApprovalRequired {
            service_name: OPHELIA_MACH_SERVICE_NAME.to_string(),
        },
    };

    let body = frame_to_body(&frame).unwrap();
    let decoded = frame_from_body(&body).unwrap();

    assert!(matches!(
        decoded,
        OpheliaFrameEnvelope::Error {
            id: 10,
            error: OpheliaError::ServiceApprovalRequired { service_name },
        } if service_name == OPHELIA_MACH_SERVICE_NAME
    ));
}

#[test]
fn malformed_xpc_command_is_bad_request() {
    let error = command_from_body(b"not an ophelia binary body").unwrap_err();

    assert!(matches!(error, OpheliaError::BadRequest { .. }));
}
