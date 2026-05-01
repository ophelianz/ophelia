# Core Rewrite Progress Log

This file keeps the rewrite honest. Add to it after each slice.

## Baseline

Branch: `refactor/http-core`

Audit target commit: `84db2ae perf: add range hot-path benchmarks`

Cargo package state: workspace with default package `ophelia` in `crates/core`

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

Boundary/API lane at audit time:

The engine imported `Settings` and app platform paths. Slice 2 fixed this by adding core config and path types.

Async runtime lane:

The engine creates its own Tokio runtime today. Active pause waits inside the actor loop, which can block other engine messages while the task drains.

Disk I/O lane:

Range workers do synchronous positioned writes inside Tokio tasks. This can block runtime worker threads on slow filesystems.

Data layout lane:

`ChunkList` and app transfer arrays are useful parallel-vector layouts. `RangeSet` insert and normalize work is likely too expensive on the write hot path. The next benchmark should feed realistic `BytesWritten` events through the scheduler, not only `RangeSet`.

Persistence lane:

SQLite, resume rows, restore loading, DB worker, and history belong in core. Core should receive paths instead of calling app platform path helpers.

GUI adapter lane:

`Downloads` was the current GUI bridge before the move. It should become the place where GUI settings turn into core config, modal fields turn into download requests, and engine events turn into GUI rows.

CLI adapter lane:

A tiny CLI can drive the engine without the GUI bridge. The clean path is a core API. After slice 2, the smallest smoke path is `DownloadEngine` plus `DownloadSpec`, but runtime ownership still needs work.

## Slice 2 Result

The core package now lives at `crates/core` and is named `ophelia`. That keeps the public API path clean for a future crates.io package.

Moved into core:

- engine modules
- state and DB modules
- HTTP range benchmark
- HTTP and engine integration tests

Parked outside the checked workspace:

- GUI source under `crates/ophelia-gui`
- GUI i18n test under `crates/ophelia-gui/tests`

Core boundary changes:

- Engine code uses `CoreConfig`, `HttpCoreConfig`, `DestinationPolicyConfig`, and `CorePaths`
- Engine code no longer imports `crate::settings` or `crate::platform`
- DB open paths are injected through `CorePaths`
- Core tests import `ophelia` directly

Scan used:

```sh
rg -n "crate::settings|crate::platform|gpui|views|ipc|updater|tray" crates/core/src crates/core/benches
```

No matches remain.

## Remaining Risks After Slice 2

- Active pause can block the actor loop
- Range workers can block Tokio worker threads with sync file writes
- Hot progress tracking uses `RangeSet` insert, sort, and merge
- Unbounded channels are used in hot event paths
- GUI adapter is not wired as a package yet
- Runtime ownership is still hidden inside `DownloadEngine`
- Restored downloads still rebuild provider config from current core config

## Next Slice

Make runtime ownership honest.

Work items:

- Replace hidden runtime ownership with async core APIs
- Decide the command/event channel shape before adding the CLI
- Keep a small sync bridge only if it lives outside the core package
- Keep documenting GUI breakage instead of bending core around it
- Add a scan or test proving core files stay free of GUI imports

Policy update:

Core quality, measured performance, and clean async Rust are the hard constraints. The GUI does not need to keep working after every core slice.

## Check Log

Slice 1 docs check:

- `cargo fmt --check` passed
- `git diff --check` passed after trimming EOF whitespace
- `git diff --cached --check` passed after force-adding docs
- `cargo clippy --all-targets` passed
- `cargo test` passed

Slice 2 core extraction check:

- `cargo fmt --check -p ophelia` passed
- `git diff --check` passed
- core GUI-import scan returned no matches
- `cargo check -p ophelia --all-targets` passed
- `cargo test -p ophelia --tests` passed
- `cargo bench -p ophelia --bench http_range_data --no-run` passed
- `cargo clippy -p ophelia --all-targets` passed
