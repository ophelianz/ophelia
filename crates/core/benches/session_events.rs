use std::collections::HashMap;
use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::{Criterion, criterion_group, criterion_main};
use ophelia::engine::{
    ArtifactState, DownloadControlAction, DownloadId, DownloadStatus, EngineEvent,
    LiveTransferRemovalAction, ProgressUpdate, TransferChunkMapState, TransferControlSupport,
    TransferSnapshot,
};
use ophelia::session::{SessionEvent, SessionSnapshot};

#[path = "../src/session/read_model.rs"]
mod read_model;

use read_model::{SessionEventCoalescer, SessionReadModel};

fn snapshot(id: DownloadId) -> TransferSnapshot {
    TransferSnapshot {
        id,
        provider_kind: "http".into(),
        source_label: "https://example.com/file.bin".into(),
        destination: PathBuf::from("file.bin"),
        status: DownloadStatus::Downloading,
        downloaded_bytes: 0,
        total_bytes: Some(100_000),
        speed_bytes_per_sec: 0,
        control_support: TransferControlSupport::all(),
        chunk_map_state: TransferChunkMapState::Unsupported,
    }
}

fn apply_hot_events(count: u64) -> (usize, u64, u64, u64) {
    let id = DownloadId(1);
    let mut read_model = SessionReadModel::default();
    let mut coalescer = SessionEventCoalescer::default();
    let _ = read_model.apply_engine_event(
        EngineEvent::TransferAdded {
            snapshot: snapshot(id),
        },
        &mut coalescer,
    );

    for step in 0..count {
        let downloaded_bytes = step.saturating_mul(64 * 1024);
        let _ = read_model.apply_engine_event(
            EngineEvent::Progress(ProgressUpdate {
                id,
                status: DownloadStatus::Downloading,
                downloaded_bytes,
                total_bytes: Some(count.saturating_mul(64 * 1024)),
                speed_bytes_per_sec: 64 * 1024,
            }),
            &mut coalescer,
        );
        let _ = read_model.apply_engine_event(
            EngineEvent::DownloadBytesWritten {
                id,
                bytes: 64 * 1024,
            },
            &mut coalescer,
        );
    }

    let snapshot_len = read_model.snapshot().transfers.len();
    let has_destination = u64::from(read_model.destination(id).is_some());
    read_model.remove(DownloadId(999_999));
    let emitted = coalescer.drain_events().len();
    let stats = coalescer.stats();
    (
        emitted,
        stats.coalesced_transfer_updates(),
        stats.coalesced_write_updates(),
        snapshot_len as u64 + has_destination,
    )
}

fn bench_session_event_coalescing(c: &mut Criterion) {
    c.bench_function("session_event_coalescing_1000_hot_updates", |bench| {
        bench.iter(|| black_box(apply_hot_events(1_000)));
    });
}

fn bench_session_event_json_transfer_changed(c: &mut Criterion) {
    let event = SessionEvent::TransferChanged {
        snapshot: snapshot(DownloadId(1)),
    };

    c.bench_function("session_event_json_transfer_changed", |bench| {
        bench.iter(|| serde_json::to_vec(black_box(&event)).unwrap());
    });
}

fn bench_session_event_json_write_bytes(c: &mut Criterion) {
    let event = SessionEvent::DownloadBytesWritten {
        id: DownloadId(1),
        bytes: 64 * 1024,
    };

    c.bench_function("session_event_json_write_bytes", |bench| {
        bench.iter(|| serde_json::to_vec(black_box(&event)).unwrap());
    });
}

fn bench_session_event_json_removed(c: &mut Criterion) {
    let event = SessionEvent::TransferRemoved {
        id: DownloadId(1),
        action: LiveTransferRemovalAction::DeleteArtifact,
        artifact_state: ArtifactState::Deleted,
    };

    c.bench_function("session_event_json_removed", |bench| {
        bench.iter(|| serde_json::to_vec(black_box(&event)).unwrap());
    });
}

fn bench_session_event_json_control_unsupported(c: &mut Criterion) {
    let event = SessionEvent::ControlUnsupported {
        id: DownloadId(1),
        action: DownloadControlAction::Pause,
    };

    c.bench_function("session_event_json_control_unsupported", |bench| {
        bench.iter(|| serde_json::to_vec(black_box(&event)).unwrap());
    });
}

criterion_group!(
    benches,
    bench_session_event_coalescing,
    bench_session_event_json_transfer_changed,
    bench_session_event_json_write_bytes,
    bench_session_event_json_removed,
    bench_session_event_json_control_unsupported
);
criterion_main!(benches);
