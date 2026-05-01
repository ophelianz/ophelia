use super::client::{SessionSubscription, SocketLines, SocketWriter};
use super::lock::{create_owner_only_dir, set_owner_only_file, write_owner_only_file};
use super::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionWireCommand {
    pub id: u64,
    pub command: SessionCommand,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum SessionWireFrame {
    Response { id: u64, response: SessionResponse },
    Error { id: u64, error: SessionError },
    Event { event: SessionEvent },
}

#[cfg(unix)]
pub(super) async fn dispatch_socket(
    path: &Path,
    id: u64,
    command: SessionCommand,
) -> Result<SessionResponse, SessionError> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(path).await.map_err(socket_io_error)?;
    let (reader, mut writer) = stream.into_split();
    send_wire_command(&mut writer, id, command).await?;
    let mut lines = BufReader::new(reader).lines();

    match read_wire_frame(&mut lines).await? {
        SessionWireFrame::Response {
            id: frame_id,
            response,
        } if frame_id == id => Ok(response),
        SessionWireFrame::Error {
            id: frame_id,
            error,
        } if frame_id == id => Err(error),
        frame => Err(unexpected_wire_frame("response", frame)),
    }
}

#[cfg(unix)]
pub(super) async fn subscribe_socket(
    path: &Path,
    id: u64,
) -> Result<SessionSubscription, SessionError> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(path).await.map_err(socket_io_error)?;
    let (reader, mut writer) = stream.into_split();
    send_wire_command(&mut writer, id, SessionCommand::Subscribe).await?;
    let mut lines = BufReader::new(reader).lines();

    match read_wire_frame(&mut lines).await? {
        SessionWireFrame::Response {
            id: frame_id,
            response: SessionResponse::Snapshot { snapshot },
        } if frame_id == id => Ok(SessionSubscription::socket(snapshot, lines, writer)),
        SessionWireFrame::Error {
            id: frame_id,
            error,
        } if frame_id == id => Err(error),
        frame => Err(unexpected_wire_frame("snapshot response", frame)),
    }
}

#[cfg(unix)]
async fn send_wire_command(
    writer: &mut SocketWriter,
    id: u64,
    command: SessionCommand,
) -> Result<(), SessionError> {
    let frame = SessionWireCommand { id, command };
    write_json_line(writer, &frame)
        .await
        .map_err(socket_io_error)
}

#[cfg(unix)]
pub(super) async fn read_wire_frame(
    lines: &mut SocketLines,
) -> Result<SessionWireFrame, SessionError> {
    let line = lines.next_line().await.map_err(socket_io_error)?;
    let Some(line) = line else {
        return Err(SessionError::Closed);
    };
    serde_json::from_str(&line).map_err(|error| SessionError::Transport {
        message: format!("failed to parse session frame: {error}"),
    })
}

#[cfg(unix)]
fn socket_io_error(error: io::Error) -> SessionError {
    match error.kind() {
        io::ErrorKind::BrokenPipe
        | io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::NotFound
        | io::ErrorKind::UnexpectedEof => SessionError::Closed,
        _ => SessionError::Transport {
            message: error.to_string(),
        },
    }
}

pub(super) fn unexpected_wire_frame(expected: &str, frame: SessionWireFrame) -> SessionError {
    SessionError::Transport {
        message: format!("expected session {expected}, got {frame:?}"),
    }
}

#[cfg(unix)]
pub struct LocalSessionServer {
    descriptor: SessionDescriptor,
    descriptor_path: PathBuf,
    socket_path: PathBuf,
    task: JoinHandle<()>,
}

#[cfg(unix)]
impl LocalSessionServer {
    pub(super) fn start(
        runtime: &Handle,
        descriptor: SessionDescriptor,
        client: SessionClient,
    ) -> Result<Self, SessionError> {
        use tokio::net::UnixListener;

        if let Some(parent) = descriptor.socket_path.parent() {
            create_owner_only_dir(parent)?;
        }
        if let Err(error) = fs::remove_file(&descriptor.socket_path)
            && error.kind() != io::ErrorKind::NotFound
        {
            return Err(SessionError::Transport {
                message: error.to_string(),
            });
        }

        let _runtime_guard = runtime.enter();
        let listener = UnixListener::bind(&descriptor.socket_path).map_err(|error| {
            SessionError::Transport {
                message: error.to_string(),
            }
        })?;
        set_owner_only_file(&descriptor.socket_path)?;
        let descriptor_path = descriptor
            .profile_database_path
            .parent()
            .map(|parent| parent.join("downloads.session.json"))
            .unwrap_or_else(|| PathBuf::from("downloads.session.json"));
        write_descriptor(&descriptor_path, &descriptor)?;

        let task = runtime.spawn(serve_local_session(listener, client));
        Ok(Self {
            socket_path: descriptor.socket_path.clone(),
            descriptor_path,
            descriptor,
            task,
        })
    }

    pub(super) fn descriptor(&self) -> &SessionDescriptor {
        &self.descriptor
    }
}

#[cfg(unix)]
impl Drop for LocalSessionServer {
    fn drop(&mut self) {
        self.task.abort();
        if let Err(error) = fs::remove_file(&self.socket_path)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %self.socket_path.display(),
                "failed to remove session socket: {error}"
            );
        }
        if let Err(error) = fs::remove_file(&self.descriptor_path)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(
                path = %self.descriptor_path.display(),
                "failed to remove session descriptor: {error}"
            );
        }
    }
}

#[cfg(unix)]
async fn serve_local_session(listener: tokio::net::UnixListener, client: SessionClient) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let client = client.clone();
                tokio::spawn(async move {
                    handle_local_connection(stream, client).await;
                });
            }
            Err(error) => {
                tracing::warn!("local session accept failed: {error}");
                break;
            }
        }
    }
}

#[cfg(unix)]
async fn handle_local_connection(stream: tokio::net::UnixStream, client: SessionClient) {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let command = match serde_json::from_str::<SessionWireCommand>(&line) {
            Ok(command) => command,
            Err(error) => {
                let frame = SessionWireFrame::Error {
                    id: id_from_bad_request(&line),
                    error: SessionError::BadRequest {
                        message: error.to_string(),
                    },
                };
                let _ = write_wire_frame(&mut writer, &frame).await;
                continue;
            }
        };

        if matches!(command.command, SessionCommand::Subscribe) {
            match client.subscribe().await {
                Ok(mut subscription) => {
                    let frame = SessionWireFrame::Response {
                        id: command.id,
                        response: SessionResponse::Snapshot {
                            snapshot: subscription.snapshot.clone(),
                        },
                    };
                    if write_wire_frame(&mut writer, &frame).await.is_err() {
                        return;
                    }
                    while let Ok(event) = subscription.next_event().await {
                        let frame = SessionWireFrame::Event { event };
                        if write_wire_frame(&mut writer, &frame).await.is_err() {
                            return;
                        }
                    }
                }
                Err(error) => {
                    let frame = SessionWireFrame::Error {
                        id: command.id,
                        error,
                    };
                    let _ = write_wire_frame(&mut writer, &frame).await;
                }
            }
            return;
        }

        match client.dispatch(command.command).await {
            Ok(response) => {
                let frame = SessionWireFrame::Response {
                    id: command.id,
                    response,
                };
                if write_wire_frame(&mut writer, &frame).await.is_err() {
                    return;
                }
            }
            Err(error) => {
                let frame = SessionWireFrame::Error {
                    id: command.id,
                    error,
                };
                if write_wire_frame(&mut writer, &frame).await.is_err() {
                    return;
                }
            }
        }
    }
}

fn id_from_bad_request(line: &str) -> u64 {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|value| value.get("id").and_then(serde_json::Value::as_u64))
        .unwrap_or(0)
}

#[cfg(unix)]
async fn write_wire_frame(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    frame: &SessionWireFrame,
) -> io::Result<()> {
    write_json_line(writer, frame).await
}

#[cfg(unix)]
async fn write_json_line<W, T>(writer: &mut W, value: &T) -> io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
    T: Serialize,
{
    use tokio::io::AsyncWriteExt;

    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    writer.write_all(&line).await
}

#[cfg(not(unix))]
pub struct LocalSessionServer;

#[cfg(not(unix))]
impl LocalSessionServer {
    pub(super) fn start(
        _runtime: &Handle,
        _descriptor: SessionDescriptor,
        _client: SessionClient,
    ) -> Result<Self, SessionError> {
        Err(SessionError::Transport {
            message: "native local transport is not implemented on this platform yet".into(),
        })
    }

    pub(super) fn descriptor(&self) -> &SessionDescriptor {
        unreachable!("local session transport is not implemented on this platform yet")
    }
}

fn write_descriptor(path: &Path, descriptor: &SessionDescriptor) -> Result<(), SessionError> {
    if let Some(parent) = path.parent() {
        create_owner_only_dir(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(descriptor).map_err(|error| SessionError::Io {
        message: error.to_string(),
    })?;
    write_owner_only_file(&tmp, &body)?;
    fs::rename(&tmp, path)?;
    set_owner_only_file(path)?;
    Ok(())
}
