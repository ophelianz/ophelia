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
    LiveTransferRemovalAction, TransferControlSupport,
};
use ophelia::settings::Settings;

fn wait_for_notification(engine: &mut DownloadEngine) -> EngineNotification {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(notification) = engine.poll_notification() {
            return notification;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for engine notification"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_matching_notification(
    engine: &mut DownloadEngine,
    mut predicate: impl FnMut(&EngineNotification) -> bool,
) -> EngineNotification {
    let deadline = Instant::now() + Duration::from_secs(3);
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
    match wait_for_notification(&mut engine) {
        EngineNotification::Update(update) => {
            assert_eq!(update.id, id);
            assert_eq!(update.status, DownloadStatus::Paused);
            assert_eq!(update.downloaded_bytes, 0);
            assert_eq!(update.total_bytes, None);
        }
        other => panic!("expected pause update, got {other:?}"),
    }

    engine.resume(id);
    match wait_for_notification(&mut engine) {
        EngineNotification::Update(update) => {
            assert_eq!(update.id, id);
            assert_eq!(update.status, DownloadStatus::Pending);
            assert_eq!(update.downloaded_bytes, 0);
            assert_eq!(update.total_bytes, None);
        }
        other => panic!("expected pending update, got {other:?}"),
    }

    engine.cancel(id);
    match wait_for_notification(&mut engine) {
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
    match wait_for_notification(&mut engine) {
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
