use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

use crate::workspace::{fatal, workspace_root};

pub(crate) fn profiling_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command.current_dir(workspace_root());
    command.env("RUSTFLAGS", profiling_rustflags());
    command
}

pub(crate) fn run_status(mut command: Command, message: &str) {
    let status = command
        .status()
        .unwrap_or_else(|_| fatal("failed to run command"));
    if !status.success() {
        fatal(message);
    }
}

pub(crate) fn print_command_output(output: &Output) {
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
}

pub(crate) fn bench_executable(bench_name: &str, cargo_json: &[u8]) -> PathBuf {
    for line in String::from_utf8_lossy(cargo_json).lines() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if message.get("reason").and_then(|value| value.as_str()) != Some("compiler-artifact") {
            continue;
        }
        let Some(target) = message.get("target") else {
            continue;
        };
        if target.get("name").and_then(|value| value.as_str()) != Some(bench_name) {
            continue;
        }
        let is_bench = target
            .get("kind")
            .and_then(|value| value.as_array())
            .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some("bench")));
        if !is_bench {
            continue;
        }
        if let Some(executable) = message.get("executable").and_then(|value| value.as_str()) {
            return PathBuf::from(executable);
        }
    }
    fatal("could not find Criterion bench executable");
}

pub(crate) fn doctor_command(cli_binary: &Path, service_binary: &Path) -> Command {
    let mut command = Command::new(cli_binary);
    command
        .current_dir(workspace_root())
        .env("OPHELIA_SERVICE_START_MODE", "dev-launchctl")
        .env("OPHELIA_SERVICE_BINARY", service_binary)
        .args(["--color", "never", "doctor"]);
    command
}

fn profiling_rustflags() -> String {
    let profiling_flags = "-C force-frame-pointers=yes -C symbol-mangling-version=v0";
    match std::env::var("RUSTFLAGS") {
        Ok(flags) if !flags.is_empty() => format!("{flags} {profiling_flags}"),
        _ => profiling_flags.to_string(),
    }
}
