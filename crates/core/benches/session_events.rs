use std::hint::black_box;
use std::path::PathBuf;

use criterion::{Criterion, criterion_group, criterion_main};
use ophelia::engine::{
    TransferControlSupport, TransferId, TransferKind, TransferStatus, TransferSummary,
};
use ophelia::service::{OpheliaUpdateBatch, TransferSummaryTable};

fn summary(id: TransferId, downloaded_bytes: u64) -> TransferSummary {
    TransferSummary {
        id,
        kind: TransferKind::Direct,
        provider_kind: "http".into(),
        source_label: "https://example.com/file.bin".into(),
        destination: PathBuf::from("file.bin"),
        status: TransferStatus::Downloading,
        downloaded_bytes,
        total_bytes: Some(100_000),
        speed_bytes_per_sec: 64 * 1024,
        control_support: TransferControlSupport::all(),
    }
}

fn build_transfer_table(count: u64) -> TransferSummaryTable {
    let mut table = TransferSummaryTable::default();
    for index in 0..count {
        table.push_summary(summary(TransferId(index), index * 64 * 1024));
    }
    table
}

fn build_hot_update_batch(count: u64) -> OpheliaUpdateBatch {
    let mut batch = OpheliaUpdateBatch::default();
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

fn apply_hot_update_batch(mut table: TransferSummaryTable, batch: &OpheliaUpdateBatch) -> u64 {
    for row in 0..batch.progress_known_total.ids.len() {
        if let Some(index) = table
            .ids
            .iter()
            .position(|id| *id == batch.progress_known_total.ids[row])
        {
            table.downloaded_bytes[index] = batch.progress_known_total.downloaded_bytes[row];
            table.speed_bytes_per_sec[index] = batch.progress_known_total.speed_bytes_per_sec[row];
            table.set_total(index, Some(batch.progress_known_total.total_bytes[row]));
        }
    }

    table.downloaded_bytes.iter().copied().sum()
}

fn bench_transfer_table_build(c: &mut Criterion) {
    c.bench_function("transfer_summary_table_build_1000", |bench| {
        bench.iter(|| black_box(build_transfer_table(1_000)));
    });
}

fn bench_update_batch_build(c: &mut Criterion) {
    c.bench_function("ophelia_update_batch_build_10000_hot_rows", |bench| {
        bench.iter(|| black_box(build_hot_update_batch(10_000)));
    });
}

fn bench_update_batch_apply(c: &mut Criterion) {
    let table = build_transfer_table(10_000);
    let batch = build_hot_update_batch(10_000);

    c.bench_function("ophelia_update_batch_apply_10000_hot_rows", |bench| {
        bench.iter(|| black_box(apply_hot_update_batch(table.clone(), black_box(&batch))));
    });
}

criterion_group!(
    benches,
    bench_transfer_table_build,
    bench_update_batch_build,
    bench_update_batch_apply
);
criterion_main!(benches);
