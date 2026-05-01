# Core Rewrite Bug Ledger

This ledger includes bugs, likely unintended behavior, and growth risks found during the first audit. It is not a refactor wish list. Each item needs a test or measurement before a large fix.

## Closed In Slice 2

### Engine Accepts Full GUI Settings

Exact behavior:

Fixed. The engine now accepts core-owned config types instead of GUI `Settings`.

Files involved:

- `crates/core/src/engine/actor.rs`
- `crates/core/src/engine/destination.rs`
- `crates/core/src/engine/spec.rs`
- `crates/core/src/engine/provider.rs`
- `crates/core/src/engine/http/config.rs`
- `crates/core/src/engine/state/db.rs`

Why it matters:

This was the first clean-crate blocker. Core can now compile, test, and benchmark without the GUI settings module.

Likely test:

Keep a scan or CI guard that fails if core imports GUI modules:

```sh
rg -n "crate::settings|crate::platform|gpui|views|ipc|updater|tray" crates/core/src crates/core/benches
```

Priority: closed

## High

### Active Pause Can Stall The Engine Actor

Exact behavior:

`EngineActor::run` handles `Pause` by awaiting `handle_pause`. `handle_pause` cancels the download token and then awaits the download task. While that await is pending, the actor is not polling other commands, task completions, or runtime updates.

Files involved:

- `crates/core/src/engine/actor.rs`
- `crates/core/src/engine/provider.rs`
- `crates/core/src/engine/http/range_runner.rs`
- `crates/core/src/engine/http/range_worker.rs`

Why it matters:

A slow pause can block unrelated engine work. This gets worse if the task is draining slow disk writes.

Likely test:

Pause one active range download while another download sends runtime updates or completes. Assert the actor still processes the other message within a short timeout.

Priority: high

### Range Workers Do Sync Disk Writes Inside Tokio Tasks

Exact behavior:

Range workers receive `Arc<std::fs::File>` and call positioned `write_all_at` or `seek_write` from inside async worker tasks.

Files involved:

- `crates/core/src/engine/http/task.rs`
- `crates/core/src/engine/http/range_runner.rs`
- `crates/core/src/engine/http/range_worker.rs`
- `crates/core/src/engine/alloc.rs`

Why it matters:

A slow disk write can block a Tokio worker thread. With multiple downloads, this can delay network reads, timers, cancellation, health checks, and progress events.

Likely test:

Add a slow disk writer test or instrumentation around the write path. Measure pause latency and runtime responsiveness during heavy range writes.

Priority: high

### Unbounded Channels On Hot Paths

Exact behavior:

The engine uses unbounded channels for commands, progress, notifications, done events, runtime updates, and worker events.

Files involved:

- `crates/core/src/engine/actor.rs`
- `crates/core/src/engine/provider.rs`
- `crates/core/src/engine/http/range_runner.rs`
- `crates/core/src/engine/http/range_worker.rs`
- `crates/ophelia-gui/src/app.rs`

Why it matters:

Fast producers can outrun the actor or GUI polling loop. Memory grows instead of applying backpressure or coalescing noisy updates.

Likely test:

Stress local downloads while delaying GUI polling. Track message counts, memory, and max queue depth.

Priority: high

### Resume Trusts Part Files Too Much

Exact behavior:

Startup restore checks that a needed part file exists. It does not prove the file length or bytes match the saved resume rows. Pause writes resume rows to SQLite, but the part file is not synced.

Files involved:

- `crates/core/src/engine/state/db.rs`
- `crates/core/src/engine/state/http.rs`
- `crates/core/src/engine/http/range_runner.rs`
- `crates/core/src/engine/http/task.rs`

Why it matters:

If a part file is truncated, changed, or not durable after a crash, Ophelia can trust bad bytes.

Likely test:

Pause a download, alter or truncate the part file, restart, and verify core rejects the stale resume data or redownloads unsafe ranges.

Priority: high

## Medium

### Hot Progress Tracking Sorts And Merges Ranges

Exact behavior:

Every `BytesWritten` event records progress through `RangeSet::insert_and_count_new`. That scans total bytes, inserts the range, sorts, merges, and scans total bytes again.

Files involved:

- `crates/core/src/engine/http/ranges.rs`
- `crates/core/src/engine/http/scheduler.rs`
- `crates/core/src/engine/http/range_worker.rs`
- `crates/core/benches/http_range_data.rs`

Why it matters:

This is probably okay for ordered writes, but fragmented writes are already much slower in benchmarks. The cost can grow as files, workers, and live strategies grow.

Likely test:

Add a scheduler-level benchmark that feeds realistic `BytesWritten` events through `RangeScheduler::apply_worker_event`.

Priority: medium

### Core Persistence Picked App Paths

Exact behavior:

Fixed. The DB code receives `CorePaths` and opens the database from injected paths.

Files involved:

- `crates/core/src/engine/state/db.rs`
- `crates/core/src/config.rs`

Why it matters:

CLI and GUI may need different config choices later. Core should not decide app shell paths on its own.

Likely test:

Keep the no-GUI-import scan. Add a focused `CorePaths` DB-open test if this code changes again.

Priority: closed

### GUI Mirrors Restored State Separately From Engine

Exact behavior:

On startup the GUI restores saved downloads into engine paused state and also pushes its own paused live rows with downloaded bytes and totals.

Files involved:

- `crates/ophelia-gui/src/app.rs`
- `crates/core/src/engine/actor.rs`
- `crates/core/src/engine/state/db.rs`

Why it matters:

Restore meaning has two owners. If they normalize resume bytes differently, the UI can show something different from what the engine will resume.

Likely test:

Restore a paused download with gapped or overlapping chunk rows. Assert the GUI row and engine resume plan agree on downloaded bytes.

Priority: medium

### Current Config Can Change Restored Downloads

Exact behavior:

Restored downloads rebuild HTTP config from current `CoreConfig` instead of persisted per-download config.

Files involved:

- `crates/core/src/engine/spec.rs`
- `crates/core/src/config.rs`
- `crates/core/src/engine/state/db.rs`

Why it matters:

A config change can alter how an old paused download resumes.

Likely test:

Pause with one ordering/config, change settings, restart, and verify whether the restored transfer keeps its old behavior or intentionally adopts the new one.

Priority: medium

### Single-Stream Pause Reports Error

Exact behavior:

The task can accept pause before probing discovers that the server requires single-stream fallback. Once in single-stream mode, pause cancellation returns an error state because single-stream has no resume data.

Files involved:

- `crates/core/src/engine/actor.rs`
- `crates/core/src/engine/http/task.rs`
- `crates/core/src/engine/http/single.rs`
- `tests/engine_notifications.rs`

Why it matters:

The behavior is tested, but the product meaning is odd. The UI can briefly believe pause is supported, then the result is error rather than paused.

Likely test:

Keep the existing test, then add a product decision test once core events can say `pause unsupported after probe`.

Priority: medium

## Low

### Parallel Vec Invariants Are Implicit

Exact behavior:

`ChunkList` and app live transfer rows store data in parallel vectors. Code indexes several vectors with the same row index.

Files involved:

- `crates/core/src/engine/chunk.rs`
- `crates/ophelia-gui/src/app.rs`

Why it matters:

The layout is good for simple scans, but a future partial push or remove bug can panic or read the wrong row.

Likely test:

Add debug assertions or constructors that keep vector lengths equal.

Priority: low

### UI Row Materialization Can Repeat Work

Exact behavior:

The GUI builds transfer row vectors for summary and list rendering. This clones small display data and filters rows.

Files involved:

- `crates/ophelia-gui/src/app.rs`
- `crates/ophelia-gui/src/views/main/main_window.rs`
- `crates/ophelia-gui/src/views/main/transfers_list.rs`

Why it matters:

This is fine for normal row counts. It only matters if Ophelia starts showing hundreds or thousands of live rows.

Likely test:

Benchmark row creation with 100, 1000, and 10000 transfers before changing it.

Priority: low
