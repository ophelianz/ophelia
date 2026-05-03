use std::{
    collections::BTreeMap,
    fs,
    num::NonZeroUsize,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crate::{
    model::{FailureKind, Importance, Options, PerfRun, TestMetadata, TimingResult},
    table::{self, Cell, Theme, Tone},
    workspace::{RUNS_DIR, fatal, workspace_root},
};

const PERF_TEST_SUFFIX: &str = "_perf_case";
const PERF_META_SUFFIX: &str = "_perf_metadata";
const ITER_ENV_VAR: &str = "OPHELIA_PERF_ITER";
const META_PREFIX: &str = "OPHELIA_PERF_META ";
const DEFAULT_ITERATIONS: NonZeroUsize = NonZeroUsize::new(1).unwrap();

pub(crate) fn run_perf_tests(test_binary: &str, args: &[String]) {
    let options = parse_options(args);
    let mut run = PerfRun::default();

    let tests = discover_perf_tests(test_binary);
    if !options.quiet {
        eprintln!("discovered {} perf test(s)", tests.len());
    }

    for (index, (test_name, metadata_name)) in tests.iter().enumerate() {
        if !options.quiet {
            eprintln!(
                "profiling {}/{} {}",
                index + 1,
                tests.len(),
                display_test_name(test_name)
            );
        }

        let display_name = display_test_name(test_name);
        let metadata = match read_metadata(test_binary, metadata_name) {
            Ok(metadata) => metadata,
            Err(kind) => {
                run.push_failure(display_name, None, None, kind);
                continue;
            }
        };

        if metadata.importance < options.min_importance {
            run.push_failure(display_name, Some(metadata), None, FailureKind::Skipped);
            continue;
        }

        let Some(iterations) = metadata
            .iterations
            .or_else(|| triage_iterations(test_binary, test_name, options.min_sample_time))
        else {
            run.push_failure(display_name, Some(metadata), None, FailureKind::Triage);
            continue;
        };

        match profile_test(test_binary, test_name, iterations, options.samples) {
            Some(result) => run.push_success(display_name, metadata, iterations, result),
            None => run.push_failure(
                display_name,
                Some(metadata),
                Some(iterations),
                FailureKind::Run,
            ),
        }
    }

    if run.is_empty() {
        if !options.quiet {
            eprintln!("no perf tests matched");
        }
        return;
    }

    if let Some(json_name) = options.output_json.as_ref() {
        write_json_run(json_name, test_binary, &run);
    }

    print_run_table(&run, &Theme::stdout());
}

pub(crate) fn compare_runs(args: &[String]) {
    let Some(new_name) = args.first() else {
        fatal("missing new run name");
    };
    let Some(old_name) = args.get(1) else {
        fatal("missing old run name");
    };

    let new_run = read_named_run(new_name);
    let old_run = read_named_run(old_name);
    let old_by_name = old_run
        .tests
        .iter()
        .map(|test| (test.name.as_str(), test))
        .collect::<BTreeMap<_, _>>();

    let mut rows = Vec::new();
    for new_test in &new_run.tests {
        let Some(old_test) = old_by_name.get(new_test.name.as_str()) else {
            continue;
        };
        let (Ok(new_timing), Ok(old_timing)) = (&new_test.result, &old_test.result) else {
            continue;
        };
        let delta = (new_timing.mean_nanos as f64 / old_timing.mean_nanos as f64 - 1.0) * 100.0;
        rows.push(vec![
            Cell::plain(new_test.name.clone()),
            Cell::plain(table::format_duration(old_timing.mean_nanos)),
            Cell::plain(table::format_duration(new_timing.mean_nanos)),
            Cell::toned(format!("{delta:+.1}%"), table::delta_tone(delta)),
        ]);
    }
    table::print(&Theme::stdout(), &["test", "old", "new", "delta"], &rows);
}

fn parse_options(args: &[String]) -> Options {
    let mut options = Options::default();
    for arg in args {
        match arg.as_str() {
            "--critical" => options.min_importance = Importance::Critical,
            "--important" => options.min_importance = Importance::Important,
            "--average" => options.min_importance = Importance::Average,
            "--iffy" => options.min_importance = Importance::Iffy,
            "--fluff" => options.min_importance = Importance::Fluff,
            "--quiet" => options.quiet = true,
            _ if arg.starts_with("--json=") => {
                options.output_json = Some(arg.trim_start_matches("--json=").to_string());
            }
            _ if arg.starts_with("--samples=") => {
                options.samples = arg
                    .trim_start_matches("--samples=")
                    .parse()
                    .unwrap_or_else(|_| fatal("invalid --samples value"));
            }
            _ if arg.starts_with("--min-sample-ms=") => {
                let millis = arg
                    .trim_start_matches("--min-sample-ms=")
                    .parse()
                    .unwrap_or_else(|_| fatal("invalid --min-sample-ms value"));
                options.min_sample_time = Duration::from_millis(millis);
            }
            _ => {}
        }
    }
    options.samples = options.samples.max(1);
    options
}

fn discover_perf_tests(test_binary: &str) -> Vec<(String, String)> {
    let output = Command::new(test_binary)
        .args(["--list", "--format=terse", "--ignored"])
        .output()
        .unwrap_or_else(|_| fatal("failed to list perf tests"));
    if !output.status.success() {
        fatal("test binary failed while listing perf tests");
    }

    let mut names = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (name, kind) = line.split_once(':')?;
            (kind.trim() == "test").then(|| name.to_string())
        })
        .filter(|name| name.ends_with(PERF_TEST_SUFFIX) || name.ends_with(PERF_META_SUFFIX))
        .collect::<Vec<_>>();
    names.sort_unstable();

    let mut tests = BTreeMap::<String, (Option<String>, Option<String>)>::new();
    for name in names {
        if name.ends_with(PERF_TEST_SUFFIX) {
            let base = name.trim_end_matches(PERF_TEST_SUFFIX).to_string();
            tests.entry(base).or_default().0 = Some(name);
        } else if name.ends_with(PERF_META_SUFFIX) {
            let base = name.trim_end_matches(PERF_META_SUFFIX).to_string();
            tests.entry(base).or_default().1 = Some(name);
        }
    }

    tests
        .into_values()
        .map(|(test, metadata)| {
            (
                test.unwrap_or_else(|| fatal("perf test is missing its runnable pair")),
                metadata.unwrap_or_else(|| fatal("perf test is missing metadata pair")),
            )
        })
        .collect()
}

fn read_metadata(test_binary: &str, metadata_name: &str) -> Result<TestMetadata, FailureKind> {
    let output = Command::new(test_binary)
        .args([metadata_name, "--exact", "--ignored", "--nocapture"])
        .output()
        .map_err(|_| FailureKind::BadMetadata)?;
    if !output.status.success() {
        return Err(FailureKind::BadMetadata);
    }

    let mut metadata = TestMetadata::default();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Some(line) = line.strip_prefix(META_PREFIX) else {
            continue;
        };
        let mut pieces = line.split_whitespace();
        match pieces.next().ok_or(FailureKind::BadMetadata)? {
            "iterations" => {
                let iterations = pieces
                    .next()
                    .ok_or(FailureKind::BadMetadata)?
                    .parse()
                    .map_err(|_| FailureKind::BadMetadata)?;
                metadata.iterations = NonZeroUsize::new(iterations);
            }
            "importance" => {
                metadata.importance =
                    Importance::parse(pieces.next().ok_or(FailureKind::BadMetadata)?)
                        .ok_or(FailureKind::BadMetadata)?;
            }
            "weight" => {
                metadata.weight = pieces
                    .next()
                    .ok_or(FailureKind::BadMetadata)?
                    .parse()
                    .map_err(|_| FailureKind::BadMetadata)?;
            }
            _ => return Err(FailureKind::BadMetadata),
        }
    }
    Ok(metadata)
}

fn triage_iterations(
    test_binary: &str,
    test_name: &str,
    min_time: Duration,
) -> Option<NonZeroUsize> {
    let _ = run_once(test_binary, test_name, DEFAULT_ITERATIONS)?;
    let mut iterations = DEFAULT_ITERATIONS;
    loop {
        let duration = run_once(test_binary, test_name, iterations)?;
        if duration >= min_time {
            return Some(iterations);
        }
        iterations = iterations.checked_mul(NonZeroUsize::new(4).unwrap())?;
    }
}

fn profile_test(
    test_binary: &str,
    test_name: &str,
    iterations: NonZeroUsize,
    samples: usize,
) -> Option<TimingResult> {
    let _ = run_once(test_binary, test_name, iterations)?;
    let mut values = Vec::with_capacity(samples);
    for _ in 0..samples {
        values.push(run_once(test_binary, test_name, iterations)?.as_nanos());
    }
    Some(timing_result(&values))
}

fn run_once(test_binary: &str, test_name: &str, iterations: NonZeroUsize) -> Option<Duration> {
    let start = Instant::now();
    let status = Command::new(test_binary)
        .args([test_name, "--exact", "--ignored"])
        .env(ITER_ENV_VAR, iterations.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    status.success().then(|| start.elapsed())
}

fn timing_result(values: &[u128]) -> TimingResult {
    let mean = values.iter().copied().sum::<u128>() / values.len() as u128;
    let variance = values
        .iter()
        .map(|value| value.abs_diff(mean))
        .map(|delta| delta * delta)
        .sum::<u128>()
        / values.len() as u128;
    TimingResult {
        mean_nanos: mean,
        stddev_nanos: integer_sqrt(variance),
    }
}

fn integer_sqrt(value: u128) -> u128 {
    if value < 2 {
        return value;
    }
    let mut x = value;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + value / x) / 2;
    }
    x
}

fn print_run_table(run: &PerfRun, theme: &Theme) {
    let mut rows = Vec::with_capacity(run.tests.len());
    for test in &run.tests {
        let importance_text = test
            .metadata
            .as_ref()
            .map_or("unknown".to_string(), |metadata| {
                metadata.importance.label().to_string()
            });
        let importance_tone = test.metadata.as_ref().map_or(Tone::Muted, |metadata| {
            table::importance_tone(metadata.importance)
        });
        let iterations = test
            .iterations
            .map_or("-".to_string(), |iterations| iterations.to_string());
        match &test.result {
            Ok(result) => rows.push(vec![
                Cell::plain(test.name.clone()),
                Cell::toned(importance_text, importance_tone),
                Cell::plain(iterations),
                Cell::toned(table::format_duration(result.mean_nanos), Tone::Good),
                Cell::plain(table::format_duration(result.stddev_nanos)),
            ]),
            Err(kind) => rows.push(vec![
                Cell::plain(test.name.clone()),
                Cell::toned(importance_text, importance_tone),
                Cell::plain(iterations),
                Cell::toned(format!("{kind:?}"), Tone::Bad),
                Cell::plain("-"),
            ]),
        }
    }

    table::print(
        theme,
        &["test", "importance", "iterations", "mean", "stddev"],
        &rows,
    );
}

fn write_json_run(name: &str, test_binary: &str, run: &PerfRun) {
    let runs_dir = workspace_root().join(RUNS_DIR);
    fs::create_dir_all(&runs_dir).unwrap_or_else(|_| fatal("failed to create .perf-runs"));
    let crate_name = Path::new(test_binary)
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.rsplit_once('-').map(|(name, _)| name))
        .unwrap_or("unknown");
    let path = runs_dir.join(format!("{name}.{crate_name}.json"));
    let bytes = serde_json::to_vec_pretty(run).unwrap_or_else(|_| fatal("failed to encode run"));
    fs::write(&path, bytes).unwrap_or_else(|_| fatal("failed to write JSON run"));
    eprintln!("wrote {}", path.display());
}

fn read_named_run(name: &str) -> PerfRun {
    let mut merged = PerfRun::default();
    let entries = fs::read_dir(workspace_root().join(RUNS_DIR))
        .unwrap_or_else(|_| fatal("missing .perf-runs directory"));
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !file_name.starts_with(&format!("{name}.")) || !file_name.ends_with(".json") {
            continue;
        }
        let prefix = file_name
            .trim_start_matches(&format!("{name}."))
            .trim_end_matches(".json");
        let bytes = fs::read(&path).unwrap_or_else(|_| fatal("failed to read perf run"));
        let run =
            serde_json::from_slice(&bytes).unwrap_or_else(|_| fatal("failed to parse perf run"));
        merged.merge_prefixed(run, prefix);
    }
    merged
}

fn display_test_name(test_name: &str) -> String {
    let stripped = test_name.trim_end_matches(PERF_TEST_SUFFIX);
    stripped
        .rsplit_once("::")
        .map_or(stripped, |(_, name)| name)
        .to_string()
}
