# Core Rewrite Progress Log

This file keeps the rewrite honest. Add to it after each slice.

## Baseline

Branch: `refcontroller/http-core`

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

Async runtime lane at audit time:

The engine created its own Tokio runtime. Slice 3 fixed that. The audit also found that active pause waited inside the controller loop. Slice 5 fixed that by letting task final state flow through `TaskRuntimeUpdate::Done`.

Disk I/O lane at audit time:

Range workers did synchronous positioned writes inside Tokio tasks. The disk writer slice fixed this for range downloads by moving positioned writes into one `spawn_blocking` writer per download.

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

## Risks Listed After Slice 2

- Active pause could block the controller loop. Slice 5 closed this
- Range workers could block Tokio worker threads with sync file writes. The disk writer slice closed this for range downloads
- Hot progress tracking uses `RangeSet` insert, sort, and merge
- GUI adapter is not wired as a package yet
- Restored downloads still rebuild provider config from current core config

## Slice 3 Result

Runtime ownership is now caller-owned. `DownloadEngine::spawn_on` takes a Tokio `Handle` instead of creating a runtime internally.

Core engine tests now run under Tokio tests and wait on async engine output:

- async progress reads
- async notification reads

Removed:

- hidden `Runtime::new`
- the runtime field inside `DownloadEngine`
- sync polling loops from engine notification tests

Runtime issue found next:

Active pause waited for the download task inside the controller loop. Slice 5 later moved task final state into the task update channel so the controller can keep polling.

## Next Slice

Fix event shape before deeper hot-path rewrites.

Work items:

- Replace separate progress and notification channels with one ordered event stream
- Decide what events must be lossless and what can be coalesced
- Keep a small sync bridge only if it lives outside the core package
- Keep documenting GUI breakage instead of bending core around it
- Add a scan or test proving core files stay free of GUI imports

Policy update:

Core quality, measured performance, and clean async Rust are the hard constraints. The GUI does not need to keep working after every core slice.

## Slice 4 Result

Core output now uses one public event stream.

Added:

- `EngineEvent`
- `DownloadEngine::next_event`
- `EngineError::Closed`
- bounded async channels for engine commands, public events, task runtime updates, and worker events
- `TaskRuntimeUpdate::Progress`, so download tasks report progress through the controller

Removed:

- public split progress reads
- public split notification reads
- direct task-to-public progress sending
- unbounded Tokio channels in core source

Current capacities:

- engine commands: 64
- public events: 512
- task runtime updates: 256
- worker events: 256

Follow-up found after review:

The first version still had two bugs in the new shape. Active pause awaited the task from inside the controller, and task final state used a separate channel that could race ahead of late task updates.

## Slice 5 Result

The event-stream slice now has the missing backpressure fixes.

Changed:

- command methods wait for controller replies through oneshot replies
- unknown ids now return `EngineError::NotFound`
- unsupported controls now return `EngineError::Unsupported`
- artifact deletion takes only a download id
- task final state now uses `TaskRuntimeUpdate::Done`
- active pause only cancels the pause token, then the controller keeps draining updates
- add and restore emit `TransferSnapshot` events

Why this matters:

Commands now tell the caller whether the controller accepted or rejected the request. Pause acceptance is not the same as paused-on-disk; the paused state still arrives later as an event. Delete-by-id cannot fall back to a frontend path. Task updates and final task state stay ordered on one channel.

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

Slice 3 runtime ownership check:

- `cargo fmt --check -p ophelia` passed
- `git diff --check` passed
- stale core import/runtime scan returned no matches
- `cargo clippy -p ophelia --all-targets` passed
- `cargo test -p ophelia --tests` passed
- `cargo bench -p ophelia --bench http_range_data --no-run` passed

Slice 4 event stream and backpressure check:

- `cargo fmt --check -p ophelia` passed
- `git diff --check` passed
- stale event/channel scan returned no matches
- core GUI-import scan returned no matches
- `cargo clippy -p ophelia --all-targets` passed
- `cargo test -p ophelia --tests` passed
- `cargo bench -p ophelia --bench http_range_data --no-run` passed

Slice 5 controller correctness check:

- `cargo fmt --check -p ophelia` passed
- `git diff --check` passed
- stale task-final and old event API scan returned no matches
- core GUI-import scan returned no matches
- `cargo clippy -p ophelia --all-targets` passed
- `cargo test -p ophelia --tests` passed
- `cargo bench -p ophelia --bench http_range_data --no-run` passed

## Slice 6 Result

Range downloads now have one disk writer owner per download.

Changed:

- the engine owner file became `controller.rs`
- the internal owner type became `EngineController`
- range workers send `RangeWriteJob` values instead of writing to `std::fs::File`
- `RangeDiskWriter` owns the file and runs on `spawn_blocking`
- `WorkerEvent::BytesWritten` is emitted only after write confirmation
- writer shutdown returns completed writes from workers that were already aborted on failure paths
- range pause drains accepted writes before saving the pause snapshot

Why this matters:

Network workers no longer block Tokio worker threads on range file writes. Disk write metrics now line up with confirmed writes in the range path.

Checks:

- `cargo fmt --check -p ophelia` passed
- `git diff --check` passed
- stale controller rename scan returned no matches
- stale direct range-worker file-write scan returned no matches
- `cargo clippy -p ophelia --all-targets` passed
- `cargo test -p ophelia --tests` passed
- `cargo bench -p ophelia --bench http_range_data --no-run` passed
