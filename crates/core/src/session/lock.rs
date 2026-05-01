use super::*;

pub(super) struct SessionLock {
    path: PathBuf,
    _file: File,
}

impl SessionLock {
    pub(super) fn acquire(
        path: PathBuf,
        descriptor_path: PathBuf,
        socket_path: PathBuf,
    ) -> Result<Self, SessionError> {
        if let Some(parent) = path.parent() {
            create_owner_only_dir(parent)?;
        }

        let mut file = match create_lock_file(&path)? {
            Some(file) => file,
            None => {
                if existing_session_is_live(&path, &descriptor_path, &socket_path) {
                    return Err(SessionError::LockHeld { path });
                }
                remove_stale_session_files(&path, &descriptor_path, &socket_path)?;
                create_lock_file(&path)?
                    .ok_or_else(|| SessionError::LockHeld { path: path.clone() })?
            }
        };
        let _ = writeln!(file, "pid={}", std::process::id());
        set_owner_only_file(&path)?;
        Ok(Self { path, _file: file })
    }
}

impl Drop for SessionLock {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.path)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %self.path.display(),
                "failed to remove session lock: {error}"
            );
        }
    }
}

fn create_lock_file(path: &Path) -> Result<Option<File>, SessionError> {
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
        Err(error) => Err(SessionError::Io {
            message: error.to_string(),
        }),
    }
}

fn existing_session_is_live(lock_path: &Path, descriptor_path: &Path, socket_path: &Path) -> bool {
    socket_accepts_connections(socket_path)
        || descriptor_pid_is_alive(descriptor_path)
        || lock_pid_is_alive(lock_path)
}

fn descriptor_pid_is_alive(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|body| serde_json::from_str::<SessionDescriptor>(&body).ok())
        .is_some_and(|descriptor| process_is_alive(descriptor.pid))
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

fn remove_stale_session_files(
    lock_path: &Path,
    descriptor_path: &Path,
    socket_path: &Path,
) -> Result<(), SessionError> {
    for path in [lock_path, descriptor_path, socket_path] {
        if let Err(error) = fs::remove_file(path)
            && error.kind() != io::ErrorKind::NotFound
        {
            return Err(SessionError::Io {
                message: error.to_string(),
            });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn socket_accepts_connections(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

#[cfg(not(unix))]
fn socket_accepts_connections(_path: &Path) -> bool {
    false
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

#[cfg(unix)]
pub(super) fn write_owner_only_file(path: &Path, body: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(body)
}

#[cfg(not(unix))]
pub(super) fn write_owner_only_file(path: &Path, body: &[u8]) -> io::Result<()> {
    fs::write(path, body)
}

pub(super) fn set_owner_only_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn session_lock_path(paths: &CorePaths) -> PathBuf {
    session_dir(paths).join("downloads.session.lock")
}

pub fn session_descriptor_path(paths: &CorePaths) -> PathBuf {
    session_dir(paths).join("downloads.session.json")
}

pub fn session_socket_path(paths: &CorePaths) -> PathBuf {
    session_dir(paths).join("downloads.session.sock")
}

pub(super) fn session_dir(paths: &CorePaths) -> PathBuf {
    paths
        .database_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
