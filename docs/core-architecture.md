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

Today the workspace builds the core package by default. The core lives in `crates/core`, and its package name is `ophelia`.

The old GUI source is parked in `crates/ophelia-gui`, but it is not a package yet. That means the GUI is intentionally behind the core rewrite right now.

`DownloadEngine` in `crates/core/src/engine/actor.rs` is the current engine handle. The caller gives it a Tokio `Handle`, and it spawns one engine actor on that runtime. Commands go through a bounded async channel. Frontends read one ordered stream with `next_event`.

The range runner is already much cleaner than the old slot model. Workers report events. The scheduler owns pending ranges, completed ranges, active attempts, optional stealing state, and optional hedge state.

## Current Leaks

The old `Settings` and platform path leaks are gone from core. The scan to keep honest is:

```sh
rg -n "crate::settings|crate::platform|gpui|views|ipc|updater|tray" crates/core/src crates/core/benches
```

The remaining large leak is hot-path ownership. Range workers still write to disk directly.

## Target Core Inputs

`CoreConfig` contains download behavior:

- max concurrent downloads
- max connections per download
- max connections per server
- global speed limit
- HTTP range ordering
- sequential extension list
- live range strategy flags
- retry and timeout knobs
- destination routing policy

`CorePaths` contains filesystem roots:

- database path
- optional legacy database path
- default download directory

`DownloadRequest` should describe one user request:

- source URL
- explicit output path when provided
- suggested filename when a browser or CLI gives one

The GUI can still save a larger `Settings` struct later. The adapter should turn it into `CoreConfig` and `CorePaths`.

## Target Core Outputs

`EngineEvent` is the single stream that frontends read. The actor is the only public sender, so progress, write stats, destination changes, control support, chunk maps, and removal events keep actor order.

Current event groups:

- `Progress`
- `DownloadBytesWritten`
- `DestinationChanged`
- `ControlSupportChanged`
- `ChunkMapChanged`
- `ControlUnsupported`
- `LiveTransferRemoved`

The GUI can convert those into `TransferListRow` and `HistoryListRow`. The CLI can print them.

## Runtime Ownership

The core now uses a caller-owned Tokio runtime. `DownloadEngine::spawn_on` takes a runtime handle and starts the engine actor there.

The target remains:

- GUI owns the runtime bridge it needs
- CLI uses `tokio::main`
- core tasks run on the runtime given by the frontend
- shutdown uses cancel tokens and waits for tasks where cleanup matters

The current actor has one high-risk pause path: active pause waits inside the actor loop. If the download task takes time to drain, the actor is not polling other engine messages.

## Persistence Ownership

SQLite belongs in core. The current `crates/core/src/engine/state` code is already close to that idea:

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
