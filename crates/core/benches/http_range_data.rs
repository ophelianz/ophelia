use criterion::{Criterion, criterion_group, criterion_main};

mod engine {
    pub use ophelia::engine::{ChunkMapCellState, HttpChunkMapSnapshot};

    pub(crate) mod http {
        pub(crate) use crate::events;
        pub(crate) use crate::ranges;
        pub(crate) use crate::scheduler;
    }
}

#[allow(dead_code)]
#[path = "../src/engine/http/chunk_map.rs"]
mod chunk_map;
#[allow(unused_imports)]
#[path = "../src/engine/http/events.rs"]
pub(crate) mod events;
#[allow(unused_imports)]
#[path = "../src/engine/http/ranges.rs"]
pub(crate) mod ranges;
#[allow(unused_imports)]
#[path = "../src/engine/http/scheduler.rs"]
pub(crate) mod scheduler;

use chunk_map::snapshot_from_covered_ranges;
use events::WorkerEvent;
use ranges::{ByteRange, RangeSet};
use scheduler::RangeScheduler;

const TOTAL_BYTES: u64 = 1_024 * 1_024 * 1_024;
const WRITE_BYTES: u64 = 64 * 1024;
const SCHEDULER_RANGE_COUNT: u64 = 4096;
const SCHEDULER_RANGE_SIZE: u64 = 64 * 1024;

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

fn scheduler_ranges(count: u64) -> Vec<ByteRange> {
    (0..count)
        .map(|slot| {
            let start = slot * SCHEDULER_RANGE_SIZE;
            range(start, start + SCHEDULER_RANGE_SIZE)
        })
        .collect()
}

fn bench_scheduler_apply_worker_events(c: &mut Criterion) {
    c.bench_function("scheduler_apply_worker_events_4096_ranges", |bench| {
        let ranges = scheduler_ranges(SCHEDULER_RANGE_COUNT);
        bench.iter(|| {
            let mut scheduler = RangeScheduler::new(
                SCHEDULER_RANGE_COUNT * SCHEDULER_RANGE_SIZE,
                ranges.iter().copied(),
            );
            while let Some(attempt) = scheduler.start_next_attempt() {
                scheduler.apply_worker_event(WorkerEvent::BytesWritten {
                    attempt: attempt.id(),
                    written: attempt.range(),
                });
                scheduler.apply_worker_event(WorkerEvent::Finished {
                    attempt: attempt.id(),
                });
            }
            scheduler.downloaded_bytes()
        });
    });
}

fn bench_scheduler_steal_largest(c: &mut Criterion) {
    c.bench_function("scheduler_steal_largest_1024_active", |bench| {
        let ranges = scheduler_ranges(1024);
        bench.iter(|| {
            let mut scheduler =
                RangeScheduler::new(1024 * SCHEDULER_RANGE_SIZE, ranges.iter().copied());
            while scheduler.start_next_attempt().is_some() {}
            scheduler.steal_largest(0, 8 * 1024, 1)
        });
    });
}

fn bench_scheduler_start_largest_hedge(c: &mut Criterion) {
    c.bench_function("scheduler_start_largest_hedge_1024_active", |bench| {
        let ranges = scheduler_ranges(1024);
        bench.iter(|| {
            let mut scheduler =
                RangeScheduler::new(1024 * SCHEDULER_RANGE_SIZE, ranges.iter().copied());
            while scheduler.start_next_attempt().is_some() {}
            scheduler.start_largest_hedge(8 * 1024)
        });
    });
}

criterion_group!(
    benches,
    bench_range_set_insert_ordered,
    bench_range_set_insert_overlapping,
    bench_range_set_insert_fragmented,
    bench_chunk_map_snapshot,
    bench_scheduler_apply_worker_events,
    bench_scheduler_steal_largest,
    bench_scheduler_start_largest_hedge
);
criterion_main!(benches);
