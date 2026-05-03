use std::{
    fs,
    io::Write,
    process::{Command, Output},
};

use crate::workspace::{fatal, workspace_root};

pub(crate) fn profiling_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command.current_dir(workspace_root());
    command.env("RUSTFLAGS", profiling_rustflags());
    command
}

pub(crate) fn run_logged(mut command: Command, log: &mut fs::File) {
    let output = command
        .output()
        .unwrap_or_else(|_| fatal("failed to run command"));
    print_command_output(&output);
    log.write_all(&output.stdout)
        .unwrap_or_else(|_| fatal("failed to write command stdout to log"));
    log.write_all(&output.stderr)
        .unwrap_or_else(|_| fatal("failed to write command stderr to log"));
    if !output.status.success() {
        fatal("command failed");
    }
}

pub(crate) fn print_command_output(output: &Output) {
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
}

fn profiling_rustflags() -> String {
    let profiling_flags = "-C force-frame-pointers=yes -C symbol-mangling-version=v0";
    match std::env::var("RUSTFLAGS") {
        Ok(flags) if !flags.is_empty() => format!("{flags} {profiling_flags}"),
        _ => profiling_flags.to_string(),
    }
}
