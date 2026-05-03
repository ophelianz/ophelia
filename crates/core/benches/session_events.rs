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

use std::collections::HashMap;
use std::hint::black_box;
use std::path::PathBuf;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use ophelia::engine::{
    TransferControlSupport, TransferId, TransferKind, TransferStatus, TransferSummary,
};
use ophelia::service::{OpheliaUpdateBatch, TransferSummaryTable};

const STANDARD_SIZES: &[u64] = &[100, 1_000, 10_000, 50_000, 100_000];
const LINEAR_REFERENCE_SIZES: &[u64] = &[100, 1_000, 10_000];

#[derive(Clone)]
struct DenseProgressTable {
    downloaded_bytes: Vec<u64>,
    total_bytes: Vec<u64>,
    speed_bytes_per_sec: Vec<u64>,
}

struct DenseProgressBatch {
    rows: Vec<u32>,
    downloaded_bytes: Vec<u64>,
    total_bytes: Vec<u64>,
    speed_bytes_per_sec: Vec<u64>,
}

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

fn build_row_map(table: &TransferSummaryTable) -> HashMap<TransferId, usize> {
    let mut row_by_id = HashMap::with_capacity(table.ids.len());
    for (row, id) in table.ids.iter().copied().enumerate() {
        row_by_id.insert(id, row);
    }
    row_by_id
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

fn build_dense_progress_table(count: u64) -> DenseProgressTable {
    let mut table = DenseProgressTable {
        downloaded_bytes: Vec::with_capacity(count as usize),
        total_bytes: Vec::with_capacity(count as usize),
        speed_bytes_per_sec: Vec::with_capacity(count as usize),
    };
    for index in 0..count {
        table.downloaded_bytes.push(index * 64 * 1024);
        table.total_bytes.push(100_000);
        table.speed_bytes_per_sec.push(64 * 1024);
    }
    table
}

fn build_dense_progress_batch(count: u64) -> DenseProgressBatch {
    let mut batch = DenseProgressBatch {
        rows: Vec::with_capacity(count as usize),
        downloaded_bytes: Vec::with_capacity(count as usize),
        total_bytes: Vec::with_capacity(count as usize),
        speed_bytes_per_sec: Vec::with_capacity(count as usize),
    };
    for index in 0..count {
        batch.rows.push(index as u32);
        batch.downloaded_bytes.push(index * 64 * 1024);
        batch.total_bytes.push(count * 64 * 1024);
        batch.speed_bytes_per_sec.push(64 * 1024);
    }
    batch
}

fn linear_reference_apply(mut table: TransferSummaryTable, batch: &OpheliaUpdateBatch) -> u64 {
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

fn row_map_apply(
    mut table: TransferSummaryTable,
    row_by_id: &HashMap<TransferId, usize>,
    batch: &OpheliaUpdateBatch,
) -> u64 {
    for row in 0..batch.progress_known_total.ids.len() {
        if let Some(&index) = row_by_id.get(&batch.progress_known_total.ids[row]) {
            table.downloaded_bytes[index] = batch.progress_known_total.downloaded_bytes[row];
            table.speed_bytes_per_sec[index] = batch.progress_known_total.speed_bytes_per_sec[row];
            table.set_total(index, Some(batch.progress_known_total.total_bytes[row]));
        }
    }

    table.downloaded_bytes.iter().copied().sum()
}

fn dense_row_apply(mut table: DenseProgressTable, batch: &DenseProgressBatch) -> u64 {
    for row in 0..batch.rows.len() {
        let index = batch.rows[row] as usize;
        table.downloaded_bytes[index] = batch.downloaded_bytes[row];
        table.total_bytes[index] = batch.total_bytes[row];
        table.speed_bytes_per_sec[index] = batch.speed_bytes_per_sec[row];
    }

    table.downloaded_bytes.iter().copied().sum()
}

fn bench_transfer_table_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("transfer_summary_table_build");
    for &count in STANDARD_SIZES {
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |bench, &count| {
                bench.iter(|| black_box(build_transfer_table(count)));
            },
        );
    }
    group.finish();
}

fn bench_update_batch_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("ophelia_update_batch_build");
    for &count in STANDARD_SIZES {
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |bench, &count| {
                bench.iter(|| black_box(build_hot_update_batch(count)));
            },
        );
    }
    group.finish();
}

fn bench_linear_reference_apply(c: &mut Criterion) {
    let mut group = c.benchmark_group("linear_reference");
    for &count in LINEAR_REFERENCE_SIZES {
        let table = build_transfer_table(count);
        let batch = build_hot_update_batch(count);
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |bench, _| {
            bench.iter_batched(
                || table.clone(),
                |table| black_box(linear_reference_apply(table, black_box(&batch))),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_row_map_apply(c: &mut Criterion) {
    let mut group = c.benchmark_group("row_map_apply");
    for &count in STANDARD_SIZES {
        let table = build_transfer_table(count);
        let row_by_id = build_row_map(&table);
        let batch = build_hot_update_batch(count);
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |bench, _| {
            bench.iter_batched(
                || table.clone(),
                |table| {
                    black_box(row_map_apply(
                        table,
                        black_box(&row_by_id),
                        black_box(&batch),
                    ))
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_dense_row_apply(c: &mut Criterion) {
    let mut group = c.benchmark_group("dense_row_apply");
    for &count in STANDARD_SIZES {
        let table = build_dense_progress_table(count);
        let batch = build_dense_progress_batch(count);
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |bench, _| {
            bench.iter_batched(
                || table.clone(),
                |table| black_box(dense_row_apply(table, black_box(&batch))),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_transfer_table_build,
    bench_update_batch_build,
    bench_linear_reference_apply,
    bench_row_map_apply,
    bench_dense_row_apply
);
criterion_main!(benches);
