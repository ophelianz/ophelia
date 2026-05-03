use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crate::{
    command::{
        bench_executable, doctor_command, print_command_output, profiling_command, run_status,
    },
    workspace::{INSTRUMENTS_PROFILE_DIR, ensure_macos, fatal, timestamp_utc, workspace_root},
};

pub(crate) fn bench(args: &[String]) {
    ensure_macos();
    let Some(bench_name) = args.first() else {
        fatal("missing bench name");
    };
    let Some(bench_filter) = args.get(1) else {
        fatal("missing Criterion filter");
    };
    let seconds = parse_seconds(&args[2..], 10);
    let timestamp = timestamp_utc();
    let profile_dir = workspace_root().join(INSTRUMENTS_PROFILE_DIR);
    fs::create_dir_all(&profile_dir)
        .unwrap_or_else(|_| fatal("failed to create Instruments profile directory"));
    let safe_filter = safe_filename(bench_filter);
    let trace_path = profile_dir.join(format!("{bench_name}-{safe_filter}-{timestamp}.trace"));
    let metadata_path =
        profile_dir.join(format!("{bench_name}-{safe_filter}-{timestamp}.cargo.json"));

    let mut build = profiling_command("cargo");
    build.args([
        "bench",
        "-p",
        "ophelia",
        "--profile",
        "profiling",
        "--bench",
        bench_name,
        "--no-run",
        "--message-format=json",
    ]);
    let output = build
        .output()
        .unwrap_or_else(|_| fatal("failed to build Criterion bench"));
    fs::write(&metadata_path, &output.stdout)
        .unwrap_or_else(|_| fatal("failed to write Cargo metadata"));
    if !output.status.success() {
        print_command_output(&output);
        fatal("failed to build Criterion bench");
    }

    let bench_exe = bench_executable(bench_name, &output.stdout);
    let mut criterion_args = vec![
        bench_filter.as_str(),
        "--profile-time",
        &seconds,
        "--color",
        "never",
        "--bench",
    ];
    if bench_filter.contains('/') {
        criterion_args.push("--exact");
    }

    let mut xctrace = Command::new("xcrun");
    xctrace.args([
        "xctrace",
        "record",
        "--quiet",
        "--template",
        "Time Profiler",
        "--output",
    ]);
    xctrace.arg(&trace_path);
    xctrace.args(["--launch", "--"]);
    xctrace.arg(&bench_exe);
    xctrace.args(criterion_args);
    run_status(xctrace, "failed to record Instruments bench trace");

    eprintln!("wrote {}", trace_path.display());
    eprintln!("cargo metadata: {}", metadata_path.display());
}

pub(crate) fn service_smoke(args: &[String]) {
    ensure_macos();
    let seconds = parse_seconds(args, 10);
    let seconds_value = seconds.parse::<u64>().unwrap_or(10);
    let timestamp = timestamp_utc();
    let profile_dir = workspace_root().join(INSTRUMENTS_PROFILE_DIR);
    fs::create_dir_all(&profile_dir)
        .unwrap_or_else(|_| fatal("failed to create Instruments profile directory"));
    let trace_path = profile_dir.join(format!("service-smoke-{timestamp}.trace"));
    let doctor_log = profile_dir.join(format!("service-smoke-{timestamp}.doctor.log"));
    let service_binary = workspace_root().join("target/profiling/ophelia-service");
    let cli_binary = workspace_root().join("target/profiling/oph");

    let mut service_build = profiling_command("cargo");
    service_build.args([
        "build",
        "--profile",
        "profiling",
        "-p",
        "ophelia-service",
        "--bin",
        "ophelia-service",
    ]);
    run_status(service_build, "failed to build ophelia-service");

    let mut cli_build = profiling_command("cargo");
    cli_build.args([
        "build",
        "--profile",
        "profiling",
        "-p",
        "ophelia-cli",
        "--bin",
        "oph",
    ]);
    run_status(cli_build, "failed to build oph");

    let doctor_output = run_doctor(&cli_binary, &service_binary);
    fs::write(&doctor_log, &doctor_output).unwrap_or_else(|_| fatal("failed to write doctor log"));
    let pid = service_pid(&doctor_output);

    let time_limit = format!("{seconds}s");
    let pid_string = pid.to_string();
    let mut xctrace = Command::new("xcrun");
    xctrace.args([
        "xctrace",
        "record",
        "--quiet",
        "--template",
        "Time Profiler",
        "--output",
    ]);
    xctrace.arg(&trace_path);
    xctrace.args([
        "--time-limit",
        time_limit.as_str(),
        "--attach",
        pid_string.as_str(),
    ]);
    let mut recorder = xctrace
        .spawn()
        .unwrap_or_else(|_| fatal("failed to start xctrace"));

    let deadline = Instant::now() + Duration::from_secs(seconds_value + 2);
    while Instant::now() < deadline {
        if recorder.try_wait().unwrap_or(None).is_some() {
            break;
        }
        let _ = doctor_command(&cli_binary, &service_binary)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        std::thread::sleep(Duration::from_millis(200));
    }

    let status = recorder
        .wait()
        .unwrap_or_else(|_| fatal("failed to wait for xctrace"));
    if !status.success() {
        fatal("failed to record service smoke trace");
    }

    eprintln!("wrote {}", trace_path.display());
    eprintln!("doctor log: {}", doctor_log.display());
}

fn run_doctor(cli_binary: &Path, service_binary: &Path) -> Vec<u8> {
    let output = doctor_command(cli_binary, service_binary)
        .output()
        .unwrap_or_else(|_| fatal("failed to run oph doctor"));
    if !output.status.success() {
        print_command_output(&output);
        fatal("oph doctor failed");
    }
    output.stdout
}

fn service_pid(doctor_output: &[u8]) -> u32 {
    for line in String::from_utf8_lossy(doctor_output).lines() {
        let mut pieces = line.split_whitespace();
        if pieces.next() == Some("pid")
            && let Some(pid) = pieces.next().and_then(|value| value.parse().ok())
        {
            return pid;
        }
    }
    fatal("could not read OpheliaService pid from doctor output");
}

fn parse_seconds(args: &[String], default_seconds: u64) -> String {
    if args.is_empty() {
        return default_seconds.to_string();
    }
    if args.len() == 1 && args[0].chars().all(|ch| ch.is_ascii_digit()) {
        return args[0].clone();
    }
    if args.len() == 2 && args[0] == "--seconds" && args[1].chars().all(|ch| ch.is_ascii_digit()) {
        return args[1].clone();
    }
    fatal("expected optional --seconds N");
}

fn safe_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}
