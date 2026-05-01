# Ophelia Core Rewrite Plan

This is the flight recorder for the long core split. It records what we know, what we are changing next, and what must stay true while the app keeps working.

The target shape is three crates:

- `ophelia-core` owns downloads, persistence, range scheduling, disk writes, pause, resume, retry, and engine events
- `ophelia-gui` owns GPUI, settings screens, rows, filters, menus, tray, updater, and browser IPC
- `ophelia-cli` starts tiny, with `ophelia URL --output PATH`, and later grows into a real command line app

The model is close to curl and libcurl. The GUI and CLI parse user choices. Core receives plain request and config structs. Core does not import GPUI, views, IPC, updater code, or GUI settings objects.

## Current State

The repo is still one Cargo package named `ophelia`. The library root exports `engine`, `ipc`, `platform`, and `settings` from the same crate.

The engine is useful but not cleanly split yet. Engine files import `Settings` in `src/engine/actor.rs`, `src/engine/destination.rs`, `src/engine/spec.rs`, `src/engine/provider.rs`, and `src/engine/http/config.rs`. Engine persistence imports app platform paths in `src/engine/state/db.rs`.

The GUI bridge is `Downloads` in `src/app.rs`. It owns `DownloadEngine`, the DB worker handle, IPC, settings, live transfer arrays, history, and metric sampling.

The current engine owns a Tokio runtime internally. `DownloadEngine::new` creates the runtime, starts the actor, and exposes sync polling methods for progress and notifications.

## Ground Rules

- Keep the app working after each slice
- Write docs and diagrams before major code movement
- Use subagents before every major slice
- Keep `benches/` because Cargo expects that name
- Do not move GUI-only ideas into core
- Do not move the whole tree into crates before the core boundary is plain
- Keep message passing between frontends and core
- Keep correctness events lossless
- Coalesce or bound noisy progress events only when correctness does not depend on each event
- Use benchmarks and tests before claiming a speed win

## Implementation Slices

1. Write this flight recorder
2. Define the core-facing boundary inside the current crate
3. Make runtime ownership frontend-owned
4. Clean persistence ownership
5. Rewrite hot range internals
6. Add one disk writer owner per download
7. Extract workspace crates
8. Add the tiny CLI smoke test

## Slice 1: Flight Recorder

Files added:

- `docs/core-rewrite-plan.md`
- `docs/core-architecture.md`
- `docs/core-diagrams.md`
- `docs/core-progress-log.md`
- `docs/core-bug-ledger.md`

These docs are under ignored `docs/`, so they must be added with `git add -f`.

The current branch is `refactor/http-core`. The audit target starts at commit `84db2ae perf: add range hot-path benchmarks`.

## Slice 2: Core Boundary In Place

The next code slice should add plain core-facing types while the project is still one crate:

- `CoreConfig`
- `CorePaths`
- `DownloadRequest`
- `EngineCommand`
- `EngineEvent`

The first useful cut is settings. `Settings` should stay in the GUI/app side. The engine should receive a smaller config that only contains download limits, destination routing, HTTP ordering, live range strategies, and paths.

Do not move files into `crates/` during this slice. We want a plain code boundary before a directory move makes every diff huge.

## Slice 3: Runtime Ownership

Core should expose async APIs. The GUI should own the Tokio bridge it needs, and the CLI can use `tokio::main`.

The current engine creates its own Tokio runtime in `DownloadEngine::new`. That is a good bridge for the current GUI, but it is not the final core shape.

## Slice 4: Persistence

Core should own SQLite schema, DB worker, restore loading, resume rows, history query rules, and artifact state.

Frontends should pass paths into core through `CorePaths`. Core should not call app platform path helpers directly.

Dev DB and settings reset is allowed during this rewrite, but it must be called out when a slice changes persisted data behavior.

## Slice 5: Hot Range Internals

The current `RangeSet` is clean but can be expensive on fragmented writes. Benchmarks already show fragmented inserts are much slower than ordered inserts.

The target is a work-unit progress table for the hot path, with exact duplicate byte counting for hedging. `RangeSet` can still be useful for pause, resume, and compact snapshots.

## Slice 6: Disk Writer

Range workers should not own file writes forever. The target is one disk writer owner per download. Workers send write jobs, the writer owns the file handle, and bytes count as written only after write confirmation.

This should also give us better write metrics than OS process counters.

## Slice 7: Workspace

Only after the boundary is clear:

- Root becomes a workspace
- Engine moves to `crates/ophelia-core`
- GUI moves to `crates/ophelia-gui`
- CLI starts at `crates/ophelia-cli`
- Core benchmarks move to `crates/ophelia-core/benches/`

## Slice 8: CLI Smoke Test

The first CLI is intentionally tiny:

```sh
ophelia URL --output PATH
```

It should print simple progress, write the file, and exit nonzero on failure.

The CLI must drive core directly, not `app::Downloads`.

## Checks After Each Slice

```sh
cargo fmt --check
git diff --check
cargo clippy --all-targets
cargo test
```

After range hot-path changes:

```sh
cargo bench --bench http_range_data
```
