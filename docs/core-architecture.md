# Ophelia Core Architecture

This document describes the architecture we are moving toward. It also says what exists today, because the rewrite should be grounded in the current code instead of a wishful diagram.

## The Big Picture

Ophelia should have a download core that can run under both the GUI and the CLI.

The core receives plain data:

- paths
- limits
- destination rules
- download requests
- pause, resume, cancel, and remove commands

The core sends plain events:

- progress
- write stats
- destination changes
- control support changes
- chunk-map state
- finished, failed, paused, or cancelled status

The GUI turns those events into rows, buttons, filters, notifications, menus, and graphs. The CLI turns those events into terminal output and exit codes.

## What Exists Today

Today everything is one crate. `src/lib.rs` exports engine, IPC, platform, and settings together. `src/main.rs` starts GPUI and opens the app window.

`Downloads` in `src/app.rs` is the current bridge. It owns the engine, settings, IPC, DB worker handle, live transfer rows, history rows, and metric sampling.

`DownloadEngine` in `src/engine/actor.rs` is the current engine handle. It creates a Tokio runtime, starts one engine actor, sends commands through an unbounded channel, and exposes polling methods for progress and notifications.

The range runner is already much cleaner than the old slot model. Workers report events. The scheduler owns pending ranges, completed ranges, active attempts, optional stealing state, and optional hedge state.

## Current Leaks

The engine still accepts the full `Settings` object. That pulls GUI and app choices into core code. The most important imports are:

- `src/engine/actor.rs` imports `Settings`
- `src/engine/destination.rs` imports `Settings`, `DestinationRule`, and `CollisionStrategy`
- `src/engine/spec.rs` imports `Settings`
- `src/engine/provider.rs` imports `Settings`
- `src/engine/http/config.rs` imports `Settings` and `HttpDownloadOrderingMode`
- `src/engine/state/db.rs` imports app platform path helpers

This is the first thing to fix. The engine should receive core config and paths, not the whole app settings file.

## Target Core Inputs

`CoreConfig` should contain download behavior:

- max concurrent downloads
- max connections per download
- max connections per server
- global speed limit
- HTTP range ordering
- sequential extension list
- live range strategy flags
- retry and timeout knobs
- destination routing policy

`CorePaths` should contain filesystem roots:

- app data directory
- database path
- default download directory

`DownloadRequest` should describe one user request:

- source URL
- explicit output path when provided
- suggested filename when a browser or CLI gives one

The GUI can still save a larger `Settings` struct. The adapter turns it into `CoreConfig` and `CorePaths`.

## Target Core Outputs

`EngineEvent` should be the single stream that frontends read. The current code has separate progress and notification queues. That works today, but the final shape should make event ordering and backpressure easier to reason about.

Useful event groups:

- `TransferAdded`
- `TransferProgress`
- `TransferWriteStats`
- `TransferDestinationChanged`
- `TransferControlSupportChanged`
- `TransferChunkMapChanged`
- `TransferPaused`
- `TransferFinished`
- `TransferFailed`
- `TransferCancelled`
- `TransferRemoved`

The GUI can convert those into `TransferListRow` and `HistoryListRow`. The CLI can print them.

## Runtime Ownership

The final core should expose async APIs. It should not secretly create the only Tokio runtime.

The current GUI needs a sync-ish handle, so we can keep a GUI bridge around while rewriting. The target is:

- GUI owns the runtime bridge it needs
- CLI uses `tokio::main`
- core tasks run on the runtime given by the frontend
- shutdown uses cancel tokens and waits for tasks where cleanup matters

The current actor has one high-risk pause path: active pause waits inside the actor loop. If the download task takes time to drain, the actor is not polling other engine messages.

## Persistence Ownership

SQLite belongs in core. The current `src/engine/state` code is already close to that idea:

- DB open and migration
- write worker
- restore loader
- resume chunk rows
- history reader
- artifact state

The part that must change is path ownership. Core should receive paths through `CorePaths`, not call app platform path helpers.

The GUI should not reconstruct restored live rows independently from core state forever. It can render rows, but restore and resume meaning should have one owner.

## Disk Ownership

Range downloads currently share an `Arc<std::fs::File>` across range workers. Each worker performs positioned writes inside an async Tokio task.

That is a real risk. A slow filesystem call can block a Tokio worker thread. It is bounded by connection count and buffer size, but it is still the wrong long-term shape.

The target is one disk writer owner per download:

- workers download bytes
- workers send write jobs through a bounded queue
- the disk writer owns the file
- the disk writer confirms writes
- progress counts written bytes only after confirmation
- pause drains or stops the writer safely
- write errors fail the transfer

This also gives us better logical disk write metrics for the UI.

## Range Engine Shape

Balanced and sequential are ordering choices.

Balanced builds many small pending ranges and lets workers pull work. Sequential builds ordered ranges and uses one request. Single-stream is the fallback when range download is not possible.

Stealing, hedging, and health retry are live strategies. They can stay available, but they should stay optional. The normal balanced path should be fast because the work units are small and cheap to schedule.

The hot-path data layout still needs work. `ChunkList` and app transfer rows are useful parallel-vector layouts. `RangeSet` is compact and easy to read, but repeated insert, sort, and merge on every written range is likely too expensive as the engine grows.

## Frontend Adapters

`ophelia-gui` should own:

- GPUI app state
- settings window
- download modal
- IPC server
- popup notifications
- updater and tray
- row filtering
- row selection
- file-manager opening

`ophelia-cli` should own:

- argument parsing
- terminal progress
- exit codes
- CLI config files later

Neither frontend should own HTTP range scheduling, DB schema, resume logic, or disk write accounting.

## Invariants

- Core does not import GPUI, views, IPC, updater, tray, or GUI row types
- Core does not accept the full GUI `Settings`
- Frontends pass paths into core
- Correctness events are lossless
- UI progress can be coalesced
- Bytes are counted as written after the write succeeds
- Hedging does not double-count bytes
- Stealing does not lose ranges
- Pause state matches bytes that core believes are reusable
- A tiny CLI can run a download without constructing the GUI app
