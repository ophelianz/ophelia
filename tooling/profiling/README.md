# Ophelia Profiling

This is the native profiling workflow. It is separate from `tooling/perf`: perf compares benchmark/scenario timings, profiling records traces for Instruments and future runtime profilers.

## Instruments

Profile one Criterion benchmark with Apple's Time Profiler:

```sh
cargo profile-bench session_events row_map_apply/10000 --seconds 10
cargo profile-bench session_events dense_row_apply/100000 --seconds 10
```

Profile the local service command path by attaching to the development OpheliaService and driving repeated `oph doctor` requests:

```sh
cargo profile-service --seconds 10
```

Traces and Cargo metadata are written under `target/profiles/instruments/`.

## Notes

The runner builds with the workspace `profiling` Cargo profile and frame pointers so Instruments has useful symbols without using debug builds. This is intentionally macOS-first because XPC and Instruments are the current native service/profiling target.
