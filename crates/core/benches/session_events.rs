use std::hint::black_box;
use std::path::PathBuf;

use criterion::{Criterion, criterion_group, criterion_main};
use ophelia::ServiceSettings;
use ophelia::engine::{
    ArtifactState, LiveTransferRemovalAction, ProgressUpdate, TransferChunkMapState,
    TransferControlAction, TransferControlSupport, TransferId, TransferStatus, TransferSummary,
};
use ophelia::service::{OpheliaEvent, OpheliaSnapshot};

#[path = "../src/service/read_model.rs"]
mod read_model;

use read_model::{OpheliaEventCoalescer, OpheliaReadModel};

#[allow(dead_code)]
#[derive(Debug, Clone)]
enum TransferRuntimeEvent {
    TransferAdded {
        snapshot: TransferSummary,
    },
    TransferRestored {
        snapshot: TransferSummary,
    },
    Progress(ProgressUpdate),
    TransferBytesWritten {
        id: TransferId,
        bytes: u64,
    },
    DestinationChanged {
        id: TransferId,
        destination: PathBuf,
    },
    ControlSupportChanged {
        id: TransferId,
        support: TransferControlSupport,
    },
    ChunkMapChanged {
        id: TransferId,
        state: TransferChunkMapState,
    },
    TransferRemoved {
        id: TransferId,
        action: LiveTransferRemovalAction,
        artifact_state: ArtifactState,
    },
    ControlUnsupported {
        id: TransferId,
        action: TransferControlAction,
    },
}

fn snapshot(id: TransferId) -> TransferSummary {
    TransferSummary {
        id,
        provider_kind: "http".into(),
        source_label: "https://example.com/file.bin".into(),
        destination: PathBuf::from("file.bin"),
        status: TransferStatus::Downloading,
        downloaded_bytes: 0,
        total_bytes: Some(100_000),
        speed_bytes_per_sec: 0,
        control_support: TransferControlSupport::all(),
        chunk_map_state: TransferChunkMapState::Unsupported,
    }
}

fn apply_hot_events(count: u64) -> (usize, u64, u64, u64) {
    let id = TransferId(1);
    let mut read_model = OpheliaReadModel::default();
    let mut coalescer = OpheliaEventCoalescer::default();
    let _ = read_model.apply_transfer_event(
        TransferRuntimeEvent::TransferAdded {
            snapshot: snapshot(id),
        },
        &mut coalescer,
    );

    for step in 0..count {
        let downloaded_bytes = step.saturating_mul(64 * 1024);
        let _ = read_model.apply_transfer_event(
            TransferRuntimeEvent::Progress(ProgressUpdate {
                id,
                status: TransferStatus::Downloading,
                downloaded_bytes,
                total_bytes: Some(count.saturating_mul(64 * 1024)),
                speed_bytes_per_sec: 64 * 1024,
            }),
            &mut coalescer,
        );
        let _ = read_model.apply_transfer_event(
            TransferRuntimeEvent::TransferBytesWritten {
                id,
                bytes: 64 * 1024,
            },
            &mut coalescer,
        );
    }

    let settings = ServiceSettings::default();
    let snapshot_len = read_model.snapshot(&settings).transfers.len();
    let has_destination = u64::from(read_model.destination(id).is_some());
    let has_running = u64::from(read_model.has_running_transfers());
    read_model.remove(TransferId(999_999));
    let emitted = coalescer.drain_events().len();
    let stats = coalescer.stats();
    (
        emitted,
        stats.coalesced_transfer_updates(),
        stats.coalesced_write_updates(),
        snapshot_len as u64 + has_destination + has_running,
    )
}

fn bench_session_event_coalescing(c: &mut Criterion) {
    c.bench_function("session_event_coalescing_1000_hot_updates", |bench| {
        bench.iter(|| black_box(apply_hot_events(1_000)));
    });
}

fn bench_session_event_json_transfer_changed(c: &mut Criterion) {
    let event = OpheliaEvent::TransferChanged {
        snapshot: snapshot(TransferId(1)),
    };

    c.bench_function("session_event_json_transfer_changed", |bench| {
        bench.iter(|| serde_json::to_vec(black_box(&event)).unwrap());
    });
}

fn bench_session_event_json_write_bytes(c: &mut Criterion) {
    let event = OpheliaEvent::TransferBytesWritten {
        id: TransferId(1),
        bytes: 64 * 1024,
    };

    c.bench_function("session_event_json_write_bytes", |bench| {
        bench.iter(|| serde_json::to_vec(black_box(&event)).unwrap());
    });
}

fn bench_session_event_json_removed(c: &mut Criterion) {
    let event = OpheliaEvent::TransferRemoved {
        id: TransferId(1),
        action: LiveTransferRemovalAction::DeleteArtifact,
        artifact_state: ArtifactState::Deleted,
    };

    c.bench_function("session_event_json_removed", |bench| {
        bench.iter(|| serde_json::to_vec(black_box(&event)).unwrap());
    });
}

fn bench_session_event_json_control_unsupported(c: &mut Criterion) {
    let event = OpheliaEvent::ControlUnsupported {
        id: TransferId(1),
        action: TransferControlAction::Pause,
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
