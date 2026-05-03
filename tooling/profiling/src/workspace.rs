use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const INSTRUMENTS_PROFILE_DIR: &str = "target/profiles/instruments";

pub(crate) fn workspace_root() -> PathBuf {
    let mut dir = std::env::current_dir().unwrap_or_else(|_| fatal("failed to read current dir"));
    loop {
        let manifest = dir.join("Cargo.toml");
        if fs::read_to_string(&manifest).is_ok_and(|contents| contents.contains("[workspace]")) {
            return dir;
        }
        if !dir.pop() {
            fatal("failed to find workspace root");
        }
    }
}

pub(crate) fn timestamp_utc() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix-{seconds}")
}

pub(crate) fn ensure_macos() {
    if !cfg!(target_os = "macos") {
        fatal("Instruments profiling is only supported on macOS");
    }
}

pub(crate) fn fatal(message: &str) -> ! {
    eprintln!("fatal: {message}");
    std::process::exit(2);
}
