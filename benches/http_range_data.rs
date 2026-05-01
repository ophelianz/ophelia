use criterion::{Criterion, criterion_group, criterion_main};

mod engine {
    pub use ophelia::engine::{ChunkMapCellState, HttpChunkMapSnapshot};
}

#[allow(dead_code)]
#[path = "../src/engine/http/chunk_map.rs"]
mod chunk_map;
#[allow(unused_imports)]
#[path = "../src/engine/http/ranges.rs"]
mod ranges;

use chunk_map::snapshot_from_covered_ranges;
use ranges::{ByteRange, RangeSet};

const TOTAL_BYTES: u64 = 1_024 * 1_024 * 1_024;
const WRITE_BYTES: u64 = 64 * 1024;

fn range(start: u64, end: u64) -> ByteRange {
    ByteRange::new(start, end).unwrap()
}

fn bench_range_set_insert_ordered(c: &mut Criterion) {
    c.bench_function("range_set_insert_ordered_64k", |bench| {
        bench.iter(|| {
            let mut set = RangeSet::new();
            for start in (0..TOTAL_BYTES).step_by(WRITE_BYTES as usize) {
                let end = (start + WRITE_BYTES).min(TOTAL_BYTES);
                set.insert_and_count_new(range(start, end));
            }
            set.total_len()
        });
    });
}

fn bench_range_set_insert_overlapping(c: &mut Criterion) {
    c.bench_function("range_set_insert_overlapping_hedge_64k", |bench| {
        bench.iter(|| {
            let mut set = RangeSet::new();
            for start in (0..TOTAL_BYTES).step_by((WRITE_BYTES * 2) as usize) {
                let first_end = (start + WRITE_BYTES * 2).min(TOTAL_BYTES);
                let second_start = (start + WRITE_BYTES).min(TOTAL_BYTES);
                let second_end = (start + WRITE_BYTES * 3).min(TOTAL_BYTES);
                set.insert_and_count_new(range(start, first_end));
                if second_start < second_end {
                    set.insert_and_count_new(range(second_start, second_end));
                }
            }
            set.total_len()
        });
    });
}

fn bench_range_set_insert_fragmented(c: &mut Criterion) {
    c.bench_function("range_set_insert_fragmented_64k", |bench| {
        let ranges: Vec<_> = (0..TOTAL_BYTES)
            .step_by(WRITE_BYTES as usize)
            .map(|start| range(start, (start + WRITE_BYTES).min(TOTAL_BYTES)))
            .collect();

        bench.iter(|| {
            let mut set = RangeSet::new();
            for &range in ranges.iter().step_by(2) {
                set.insert_and_count_new(range);
            }
            for &range in ranges.iter().skip(1).step_by(2) {
                set.insert_and_count_new(range);
            }
            set.total_len()
        });
    });
}

fn bench_chunk_map_snapshot(c: &mut Criterion) {
    c.bench_function("chunk_map_snapshot_128_cells", |bench| {
        let ranges: Vec<_> = (0..TOTAL_BYTES)
            .step_by((WRITE_BYTES * 4) as usize)
            .map(|start| (start, (start + WRITE_BYTES * 2).min(TOTAL_BYTES)))
            .collect();

        bench.iter(|| snapshot_from_covered_ranges(TOTAL_BYTES, ranges.iter().copied()));
    });
}

criterion_group!(
    benches,
    bench_range_set_insert_ordered,
    bench_range_set_insert_overlapping,
    bench_range_set_insert_fragmented,
    bench_chunk_map_snapshot
);
criterion_main!(benches);
