# Core Rewrite Progress Log

This file keeps the rewrite honest. Add to it after each slice.

## Baseline

Branch: `refactor/http-core`

Audit target commit: `84db2ae perf: add range hot-path benchmarks`

Cargo package state: one package named `ophelia`

Docs state: `docs/` is ignored, so selected architecture docs must be force-added

## Current Benchmark Baseline

Last known `cargo bench --bench http_range_data` baseline from the range hot-path benchmark slice:

| benchmark | observed range |
| --- | --- |
| `range_set_insert_ordered_64k` | about 394 to 421 us |
| `range_set_insert_overlapping_hedge_64k` | about 368 to 384 us |
| `range_set_insert_fragmented_64k` | about 151 to 156 ms |
| `chunk_map_snapshot_128_cells` | about 83 to 87 us |

The big lesson is that fragmented range insertion is much slower than ordered insertion. That supports rewriting the hot progress data path before the engine grows much larger.

## Audit Lanes Run For Slice 1

Boundary/API lane:

The engine still imports `Settings` and app platform paths. The next code slice should add core config and path types before moving files.

Async runtime lane:

The engine creates its own Tokio runtime today. Active pause waits inside the actor loop, which can block other engine messages while the task drains.

Disk I/O lane:

Range workers do synchronous positioned writes inside Tokio tasks. This can block runtime worker threads on slow filesystems.

Data layout lane:

`ChunkList` and app transfer arrays are useful parallel-vector layouts. `RangeSet` insert and normalize work is likely too expensive on the write hot path. The next benchmark should feed realistic `BytesWritten` events through the scheduler, not only `RangeSet`.

Persistence lane:

SQLite, resume rows, restore loading, DB worker, and history belong in core. Core should receive paths instead of calling app platform path helpers.

GUI adapter lane:

`Downloads` is the current GUI bridge. It should become the place where `Settings` turn into core config, modal fields turn into download requests, and engine events turn into GUI rows.

CLI adapter lane:

A tiny CLI can drive the engine without `app::Downloads`. The clean path is a core API. The smallest current smoke path would be `DownloadEngine` plus `DownloadSpec`, but it still pulls `Settings` and a DB event channel.

## Known Risks Before Code Slice 2

- Engine accepts the full `Settings` object
- Engine persistence chooses app data paths directly
- Active pause can block the actor loop
- Range workers can block Tokio worker threads with sync file writes
- Hot progress tracking uses `RangeSet` insert, sort, and merge
- Unbounded channels are used in hot event paths
- GUI mirrors restored row state separately from engine restore state
- Settings reset can change behavior of restored downloads

## Next Slice

Build the core-first boundary.

Work items:

- Create the workspace shape earlier if that makes core cleaner
- Make `ophelia-core` compile without GPUI as soon as practical
- Add core-facing config and path types
- Convert GUI `Settings` into those types at the app boundary
- Change engine entry points to accept core config, not `Settings`
- Document any temporary GUI breakage instead of bending core around it
- Add tests for settings-to-core conversion
- Add a scan or test proving engine files no longer import `crate::settings`

Policy update:

Core quality, measured performance, and clean async Rust are the hard constraints. The GUI does not need to keep working after every core slice.

## Check Log

Slice 1 docs check:

- `cargo fmt --check` passed
- `git diff --check` passed after trimming EOF whitespace
- `git diff --cached --check` passed after force-adding docs
- `cargo clippy --all-targets` passed
- `cargo test` passed
