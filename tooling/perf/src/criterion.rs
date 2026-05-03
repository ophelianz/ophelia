use std::{fs, io::Write};

use crate::{
    command::{profiling_command, run_logged},
    workspace::{CRITERION_PROFILE_DIR, fatal, timestamp_utc, workspace_root},
};

const CORE_BENCHES: &[&str] = &[
    "http_range_data",
    "session_events",
    "disk_data",
    "service_codec_data",
];

pub(crate) fn command(args: &[String]) {
    let Some(command) = args.first().map(String::as_str) else {
        fatal("missing criterion command: baseline or compare");
    };
    let Some(name) = args.get(1) else {
        fatal("missing Criterion baseline name");
    };
    match command {
        "baseline" => run("baseline", name, "--save-baseline"),
        "compare" => run("compare", name, "--baseline"),
        _ => fatal("unknown criterion command"),
    }
}

fn run(action: &str, baseline: &str, criterion_flag: &str) {
    let timestamp = timestamp_utc();
    let log_dir = workspace_root().join(CRITERION_PROFILE_DIR);
    fs::create_dir_all(&log_dir)
        .unwrap_or_else(|_| fatal("failed to create Criterion profile directory"));
    let log_path = log_dir.join(format!("criterion-{action}-{baseline}-{timestamp}.log"));
    let mut log =
        fs::File::create(&log_path).unwrap_or_else(|_| fatal("failed to create Criterion log"));

    for bench in CORE_BENCHES {
        let header = format!("==> Criterion {action} '{baseline}' for {bench}\n");
        print!("{header}");
        log.write_all(header.as_bytes())
            .unwrap_or_else(|_| fatal("failed to write Criterion log"));

        let mut command = profiling_command("cargo");
        command.args([
            "bench",
            "-p",
            "ophelia",
            "--profile",
            "profiling",
            "--bench",
            bench,
            "--",
            criterion_flag,
            baseline,
        ]);
        run_logged(command, &mut log);
    }

    eprintln!("wrote {}", log_path.display());
}
