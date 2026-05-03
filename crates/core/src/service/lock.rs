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

use super::*;

pub(super) struct ServiceLock {
    path: PathBuf,
    _file: File,
}

impl ServiceLock {
    pub(super) fn acquire(path: PathBuf) -> Result<Self, OpheliaError> {
        if let Some(parent) = path.parent() {
            create_owner_only_dir(parent)?;
        }

        let mut file = match create_lock_file(&path)? {
            Some(file) => file,
            None => {
                if lock_pid_is_alive(&path) {
                    return Err(OpheliaError::LockHeld { path });
                }
                remove_stale_service_file(&path)?;
                create_lock_file(&path)?
                    .ok_or_else(|| OpheliaError::LockHeld { path: path.clone() })?
            }
        };
        let _ = writeln!(file, "pid={}", std::process::id());
        set_owner_only_file(&path)?;
        Ok(Self { path, _file: file })
    }
}

impl Drop for ServiceLock {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.path)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %self.path.display(),
                "failed to remove service lock: {error}"
            );
        }
    }
}

fn create_lock_file(path: &Path) -> Result<Option<File>, OpheliaError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    match options.open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(None),
        Err(error) => Err(OpheliaError::Io {
            message: error.to_string(),
        }),
    }
}

fn lock_pid_is_alive(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|body| {
            body.lines()
                .find_map(|line| line.strip_prefix("pid=")?.parse::<u32>().ok())
        })
        .is_some_and(process_is_alive)
}

fn remove_stale_service_file(path: &Path) -> Result<(), OpheliaError> {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != io::ErrorKind::NotFound
    {
        return Err(OpheliaError::Io {
            message: error.to_string(),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> bool {
    false
}

pub(super) fn create_owner_only_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub(super) fn set_owner_only_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn service_lock_path(paths: &ProfilePaths) -> PathBuf {
    paths.service_lock_path.clone()
}
