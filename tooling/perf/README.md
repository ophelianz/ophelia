# Ophelia Perf Runner

## Usage

Run service update perf tests 

```sh
cargo perf-service-updates
```

Save a JSON run under `.perf-runs/`:

```sh
cargo perf-service-updates -- --json=before-row-identity
```

Compare two saved runs:

```sh
cargo perf-compare after-row-identity before-row-identity
```

Filter by importance:

```sh
cargo perf-service-updates -- --critical
cargo perf-service-updates -- --important
```

Use more samples when checking a change:

```sh
cargo perf-service-updates -- --samples=25 --min-sample-ms=500
```

Run Criterion baselines through the same Rust runner:

```sh
cargo perf-criterion-baseline before-change
cargo perf-criterion-compare before-change
```

Criterion logs are written under `target/profiles/criterion/`. Instruments profiling lives in `tooling/profiling` and is exposed through `cargo profile-bench` / `cargo profile-service`.

## How Tests Are Marked

The metadata test prints lines like:

```text
OPHELIA_PERF_META importance critical
OPHELIA_PERF_META weight 100
```

It may also pin an iteration count:

```text
OPHELIA_PERF_META iterations 32
```

## Current Service Update Cases

- `service_snapshot_table_build_100k`
- `service_update_batch_build_100k`
- `service_update_row_map_apply_100k`
- `service_update_dense_row_apply_100k`

## Benchmarking Coverage

- `http_range_data`: HTTP range scheduler, chunk map transforms, hedging, and stealing data paths
- `session_events`: transfer summary tables, update batch construction, row-map update application, and dense-row upper-bound application
- `disk_data`: disk session bookkeeping, write confirmation, logical confirmation, commit checks, and artifact classification
- `service_codec_data`: service command/frame encode-decode, snapshot tables, update batches, errors, and malformed bodies

The runner sets frame pointers and line-table debug info through the `profiling` Cargo profile so Criterion output is suitable for follow-up profiling. `samply`, `cargo-instruments`, allocator swaps, and metrics crates are not yet implemented
