use std::time::{Duration, Instant};

use ophelia::engine::destination::DestinationPolicy;
use ophelia::engine::http::HttpDownloadConfig;
use ophelia::engine::{
    ArtifactState, DownloadEngine, DownloadSpec, DownloadStatus, EngineNotification,
    LiveTransferRemovalAction,
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
        DestinationPolicy::manual(),
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
        DestinationPolicy::manual(),
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
