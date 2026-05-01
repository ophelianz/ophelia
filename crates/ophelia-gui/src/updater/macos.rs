/***************************************************
** This file is part of Ophelia.
** Copyright © 2026 Viktor Luna <viktor@hystericca.dev>
** Released under the GPL License, version 3 or later.
**
** If you found a weird little bug in here, tell the cat:
** viktor@hystericca.dev
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs behave plz, we're all trying our best )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RESTART_SCRIPT: &str = r#"#!/bin/sh
set -eu

pid="$1"
staged_app="$2"
target_app="$3"
work_dir="$4"
backup_app="${target_app}.backup"
temp_app="${target_app}.incoming"
installed=0

while kill -0 "$pid" 2>/dev/null; do
  sleep 1
done

cleanup() {
  if [ "$installed" -eq 0 ] && [ ! -d "$target_app" ] && [ -d "$backup_app" ]; then
    mv "$backup_app" "$target_app"
  fi
  rm -rf "$temp_app"
}

trap cleanup EXIT INT TERM HUP

rm -rf "$backup_app" "$temp_app"
if [ -d "$target_app" ]; then
  mv "$target_app" "$backup_app"
fi

/usr/bin/ditto "$staged_app" "$temp_app"
rm -rf "$target_app"
mv "$temp_app" "$target_app"
installed=1
rm -rf "$backup_app"
open "$target_app" || true
rm -rf "$work_dir"
trap - EXIT INT TERM HUP
"#;

pub(super) fn prepare_install(archive_path: &Path, working_dir: &Path) -> Result<PathBuf, String> {
    let extract_dir = working_dir.join("extract");
    if extract_dir.exists() {
        fs::remove_dir_all(&extract_dir)
            .map_err(|error| format!("failed to clear extract dir: {error}"))?;
    }
    fs::create_dir_all(&extract_dir)
        .map_err(|error| format!("failed to create extract dir: {error}"))?;

    let status = Command::new("/usr/bin/ditto")
        .arg("-x")
        .arg("-k")
        .arg(archive_path)
        .arg(&extract_dir)
        .status()
        .map_err(|error| format!("failed to extract update archive: {error}"))?;
    if !status.success() {
        return Err("failed to extract update archive".into());
    }

    let staged_app_path = find_app_bundle(&extract_dir)?
        .ok_or_else(|| "extracted update did not contain an app bundle".to_string())?;
    verify_signed_app(&staged_app_path)?;
    Ok(staged_app_path)
}

pub(super) fn restart_to_update(staged_app_path: &Path, working_dir: &Path) -> Result<(), String> {
    let running_app_path = running_app_path()?;
    let restart_script = working_dir.join("restart.sh");
    fs::write(&restart_script, RESTART_SCRIPT)
        .map_err(|error| format!("failed to write restart helper: {error}"))?;

    let mut command = Command::new("/bin/sh");
    command
        .arg(&restart_script)
        .arg(std::process::id().to_string())
        .arg(staged_app_path)
        .arg(&running_app_path)
        .arg(working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
        .spawn()
        .map_err(|error| format!("failed to launch restart helper: {error}"))?;
    Ok(())
}

fn verify_signed_app(app_bundle: &Path) -> Result<(), String> {
    let codesign = Command::new("/usr/bin/codesign")
        .arg("--verify")
        .arg("--deep")
        .arg("--strict")
        .arg("--check-notarization")
        .arg(app_bundle)
        .status()
        .map_err(|error| format!("failed to run codesign verification: {error}"))?;
    if !codesign.success() {
        return Err("codesign verification failed for staged app".into());
    }

    let spctl = Command::new("/usr/sbin/spctl")
        .arg("--assess")
        .arg("--type")
        .arg("execute")
        .arg(app_bundle)
        .status()
        .map_err(|error| format!("failed to run spctl verification: {error}"))?;
    if !spctl.success() {
        return Err("Gatekeeper assessment failed for staged app".into());
    }

    Ok(())
}

fn running_app_path() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))?
        .ancestors()
        .find(|path| path.extension().and_then(OsStr::to_str) == Some("app"))
        .map(Path::to_path_buf)
        .ok_or_else(|| "current Ophelia process is not running from an app bundle".into())
}

fn find_app_bundle(root: &Path) -> Result<Option<PathBuf>, String> {
    let entries =
        fs::read_dir(root).map_err(|error| format!("failed to read extract dir: {error}"))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read extract entry: {error}"))?;
        let path = entry.path();
        if path.extension().and_then(OsStr::to_str) == Some("app") {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_script_references_staging_and_cleanup_paths() {
        assert!(RESTART_SCRIPT.contains("staged_app"));
        assert!(RESTART_SCRIPT.contains("rm -rf \"$work_dir\""));
        assert!(RESTART_SCRIPT.contains("open \"$target_app\""));
        assert!(RESTART_SCRIPT.contains("mv \"$backup_app\" \"$target_app\""));
        assert!(RESTART_SCRIPT.contains("temp_app"));
    }
}
