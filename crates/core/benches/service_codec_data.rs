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

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ophelia::ProfilePaths;
use ophelia::config;
use ophelia::engine::{
    HistoryFilter, HistoryRow, TransferControlAction, TransferControlSupport, TransferId,
    TransferKind, TransferStatus, TransferSummary,
};
use ophelia::service::{
    DirectDetailsTable, OpheliaError, OpheliaServiceInfo, OpheliaSnapshot, OpheliaUpdateBatch,
    TransferDestination, TransferLifecycleBatch, TransferLifecycleCode, TransferRequest,
    TransferRequestSource, TransferSummaryTable,
};
use serde::{Deserialize, Serialize};

const CODEC_TABLE_SIZES: &[u64] = &[100, 1_000, 10_000, 50_000, 100_000];

mod service {
    use super::*;
    use ophelia::engine::*;
    use ophelia::service::*;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(tag = "type", content = "data", rename_all = "snake_case")]
    pub(crate) enum OpheliaCommand {
        Add {
            request: TransferRequest,
        },
        Pause {
            id: TransferId,
        },
        Resume {
            id: TransferId,
        },
        Cancel {
            id: TransferId,
        },
        DeleteArtifact {
            id: TransferId,
        },
        UpdateSettings {
            settings: config::ServiceSettings,
        },
        LoadHistory {
            filter: HistoryFilter,
            query: String,
        },
        ServiceInfo,
        Snapshot,
        Subscribe,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(tag = "type", content = "data", rename_all = "snake_case")]
    pub(crate) enum OpheliaResponse {
        Ack,
        TransferAdded { id: TransferId },
        History { rows: Vec<HistoryRow> },
        ServiceInfo { info: Box<OpheliaServiceInfo> },
        Snapshot { snapshot: Box<OpheliaSnapshot> },
    }

    #[allow(dead_code)]
    mod codec {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/service/codec.rs"));
    }

    pub(crate) fn encode_command(command: OpheliaCommand) -> Vec<u8> {
        codec::command_to_body(&codec::OpheliaCommandEnvelope { id: 1, command }).unwrap()
    }

    pub(crate) fn decode_command(body: &[u8]) -> OpheliaCommand {
        codec::command_from_body(body).unwrap().command
    }

    pub(crate) fn encode_response(response: OpheliaResponse) -> Vec<u8> {
        codec::frame_to_body(&codec::OpheliaFrameEnvelope::Response {
            id: 1,
            response: Box::new(response),
        })
        .unwrap()
    }

    pub(crate) fn decode_response(body: &[u8]) -> OpheliaResponse {
        match codec::frame_from_body(body).unwrap() {
            codec::OpheliaFrameEnvelope::Response { response, .. } => *response,
            frame => panic!("expected response frame, got {frame:?}"),
        }
    }

    pub(crate) fn encode_update(update: OpheliaUpdateBatch) -> Vec<u8> {
        codec::frame_to_body(&codec::OpheliaFrameEnvelope::Update {
            update: Box::new(update),
        })
        .unwrap()
    }

    pub(crate) fn decode_update(body: &[u8]) -> OpheliaUpdateBatch {
        match codec::frame_from_body(body).unwrap() {
            codec::OpheliaFrameEnvelope::Update { update } => *update,
            frame => panic!("expected update frame, got {frame:?}"),
        }
    }

    pub(crate) fn encode_error(error: OpheliaError) -> Vec<u8> {
        codec::frame_to_body(&codec::OpheliaFrameEnvelope::Error { id: 1, error }).unwrap()
    }

    pub(crate) fn decode_error(body: &[u8]) -> OpheliaError {
        match codec::frame_from_body(body).unwrap() {
            codec::OpheliaFrameEnvelope::Error { error, .. } => error,
            frame => panic!("expected error frame, got {frame:?}"),
        }
    }

    pub(crate) fn decode_malformed(body: &[u8]) -> OpheliaError {
        codec::frame_from_body(body).unwrap_err()
    }

    fn control_action_from_code(code: u8) -> TransferControlAction {
        match code {
            0 => TransferControlAction::Pause,
            1 => TransferControlAction::Resume,
            2 => TransferControlAction::Cancel,
            3 => TransferControlAction::Restore,
            _ => TransferControlAction::Cancel,
        }
    }

    fn transfer_status_from_code(code: u8) -> TransferStatus {
        match code {
            0 => TransferStatus::Pending,
            1 => TransferStatus::Downloading,
            2 => TransferStatus::Paused,
            3 => TransferStatus::Finished,
            4 => TransferStatus::Error,
            5 => TransferStatus::Cancelled,
            _ => TransferStatus::Error,
        }
    }

    fn artifact_state_from_code(code: u8) -> ArtifactState {
        match code {
            0 => ArtifactState::Present,
            1 => ArtifactState::Deleted,
            2 => ArtifactState::Missing,
            _ => ArtifactState::Missing,
        }
    }
}

fn summary(id: TransferId, downloaded_bytes: u64) -> TransferSummary {
    TransferSummary {
        id,
        kind: TransferKind::Direct,
        provider_kind: "http".into(),
        source_label: format!("https://example.com/file-{}.bin", id.0),
        destination: PathBuf::from(format!("file-{}.bin", id.0)),
        status: TransferStatus::Downloading,
        downloaded_bytes,
        total_bytes: Some(100_000),
        speed_bytes_per_sec: 64 * 1024,
        control_support: TransferControlSupport::all(),
    }
}

fn transfer_table(count: u64) -> TransferSummaryTable {
    let mut table = TransferSummaryTable::default();
    for index in 0..count {
        table.push_summary(summary(TransferId(index), index * 64 * 1024));
    }
    table
}

fn update_batch(count: u64) -> OpheliaUpdateBatch {
    let mut batch = OpheliaUpdateBatch {
        lifecycle: TransferLifecycleBatch {
            transfers: transfer_table(count.min(10)),
            lifecycle_codes: Vec::new(),
        },
        ..OpheliaUpdateBatch::default()
    };
    batch.lifecycle.lifecycle_codes.resize(
        batch.lifecycle.transfers.len(),
        TransferLifecycleCode::Added as u8,
    );
    for index in 0..count {
        let id = TransferId(index);
        batch.progress_known_total.ids.push(id);
        batch
            .progress_known_total
            .downloaded_bytes
            .push(index * 64 * 1024);
        batch
            .progress_known_total
            .total_bytes
            .push(count * 64 * 1024);
        batch
            .progress_known_total
            .speed_bytes_per_sec
            .push(64 * 1024);
        batch.physical_write.ids.push(id);
        batch.physical_write.bytes.push(64 * 1024);
    }
    batch
}

fn snapshot(count: u64) -> OpheliaSnapshot {
    OpheliaSnapshot {
        transfers: transfer_table(count),
        direct_details: DirectDetailsTable::default(),
        settings: config::ServiceSettings::default(),
    }
}

fn add_command() -> service::OpheliaCommand {
    service::OpheliaCommand::Add {
        request: TransferRequest {
            source: TransferRequestSource::Http {
                url: "https://example.com/file.bin".into(),
            },
            destination: TransferDestination::Automatic {
                suggested_filename: Some("file.bin".into()),
            },
        },
    }
}

fn service_info_response() -> service::OpheliaResponse {
    let paths = ProfilePaths::new(
        "/tmp/ophelia-profile/downloads.db",
        "/tmp/ophelia-downloads",
    );
    service::OpheliaResponse::ServiceInfo {
        info: Box::new(OpheliaServiceInfo::current(&paths)),
    }
}

fn bench_command_encode_decode(c: &mut Criterion) {
    c.bench_function("service_codec_command_add_encode_decode", |bench| {
        bench.iter(|| {
            let body = service::encode_command(add_command());
            black_box(service::decode_command(black_box(&body)))
        });
    });
}

fn bench_snapshot_response_encode_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("service_codec_snapshot_response_encode_decode");
    for &count in CODEC_TABLE_SIZES {
        let snapshot = snapshot(count);
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &snapshot,
            |bench, snapshot| {
                bench.iter(|| {
                    let body = service::encode_response(service::OpheliaResponse::Snapshot {
                        snapshot: Box::new(snapshot.clone()),
                    });
                    black_box(service::decode_response(black_box(&body)))
                });
            },
        );
    }
    group.finish();
}

fn bench_update_encode_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("service_codec_update_batch_encode_decode");
    for &count in CODEC_TABLE_SIZES {
        let batch = update_batch(count);
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &batch,
            |bench, batch| {
                bench.iter(|| {
                    let body = service::encode_update(batch.clone());
                    black_box(service::decode_update(black_box(&body)))
                });
            },
        );
    }
    group.finish();
}

fn bench_error_encode_decode(c: &mut Criterion) {
    c.bench_function("service_codec_error_encode_decode", |bench| {
        bench.iter(|| {
            let body = service::encode_error(OpheliaError::Unsupported {
                id: TransferId(42),
                action: TransferControlAction::Pause,
            });
            black_box(service::decode_error(black_box(&body)))
        });
    });
}

fn bench_malformed_decode(c: &mut Criterion) {
    c.bench_function("service_codec_malformed_decode", |bench| {
        bench.iter(|| {
            black_box(service::decode_malformed(black_box(
                b"not an ophelia frame",
            )))
        });
    });
}

fn bench_service_info_response_encode_decode(c: &mut Criterion) {
    let response = service_info_response();
    c.bench_function("service_codec_service_info_encode_decode", |bench| {
        bench.iter(|| {
            let body = service::encode_response(response.clone());
            black_box(service::decode_response(black_box(&body)))
        });
    });
}

criterion_group!(
    benches,
    bench_command_encode_decode,
    bench_snapshot_response_encode_decode,
    bench_update_encode_decode,
    bench_error_encode_decode,
    bench_malformed_decode,
    bench_service_info_response_encode_decode
);
criterion_main!(benches);
