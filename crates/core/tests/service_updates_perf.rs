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

use ophelia::engine::{
    TransferControlSupport, TransferId, TransferKind, TransferStatus, TransferSummary,
};
use ophelia::service::{OpheliaUpdateBatch, TransferSummaryTable};

const PERF_ITER_ENV: &str = "OPHELIA_PERF_ITER";
const ROW_COUNT: u64 = 100_000;

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

fn perf_iterations() -> usize {
    std::env::var(PERF_ITER_ENV)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

fn print_metadata(importance: &str, weight: u8, iterations: Option<usize>) {
    if let Some(iterations) = iterations {
        println!("OPHELIA_PERF_META iterations {iterations}");
    }
    println!("OPHELIA_PERF_META importance {importance}");
    println!("OPHELIA_PERF_META weight {weight}");
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

fn dense_row_apply(mut table: DenseProgressTable, batch: &DenseProgressBatch) -> u64 {
    for row in 0..batch.rows.len() {
        let index = batch.rows[row] as usize;
        table.downloaded_bytes[index] = batch.downloaded_bytes[row];
        table.total_bytes[index] = batch.total_bytes[row];
        table.speed_bytes_per_sec[index] = batch.speed_bytes_per_sec[row];
    }
    table.downloaded_bytes.iter().copied().sum()
}

#[test]
#[ignore = "run with cargo perf-service-updates"]
fn service_snapshot_table_build_100k_perf_case() {
    for _ in 0..perf_iterations() {
        black_box(build_transfer_table(ROW_COUNT));
    }
}

#[test]
#[ignore = "metadata for cargo perf-service-updates"]
fn service_snapshot_table_build_100k_perf_metadata() {
    print_metadata("important", 70, None);
}

#[test]
#[ignore = "run with cargo perf-service-updates"]
fn service_update_batch_build_100k_perf_case() {
    for _ in 0..perf_iterations() {
        black_box(build_hot_update_batch(ROW_COUNT));
    }
}

#[test]
#[ignore = "metadata for cargo perf-service-updates"]
fn service_update_batch_build_100k_perf_metadata() {
    print_metadata("important", 70, None);
}

#[test]
#[ignore = "run with cargo perf-service-updates"]
fn service_update_row_map_apply_100k_perf_case() {
    let table = build_transfer_table(ROW_COUNT);
    let row_by_id = build_row_map(&table);
    let batch = build_hot_update_batch(ROW_COUNT);

    for _ in 0..perf_iterations() {
        black_box(row_map_apply(
            table.clone(),
            black_box(&row_by_id),
            black_box(&batch),
        ));
    }
}

#[test]
#[ignore = "metadata for cargo perf-service-updates"]
fn service_update_row_map_apply_100k_perf_metadata() {
    print_metadata("critical", 100, None);
}

#[test]
#[ignore = "run with cargo perf-service-updates"]
fn service_update_dense_row_apply_100k_perf_case() {
    let table = build_dense_progress_table(ROW_COUNT);
    let batch = build_dense_progress_batch(ROW_COUNT);

    for _ in 0..perf_iterations() {
        black_box(dense_row_apply(table.clone(), black_box(&batch)));
    }
}

#[test]
#[ignore = "metadata for cargo perf-service-updates"]
fn service_update_dense_row_apply_100k_perf_metadata() {
    print_metadata("average", 50, None);
}
