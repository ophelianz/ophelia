use super::host::SessionRequest;
use super::wire::{
    SessionWireFrame, dispatch_socket, read_wire_frame, subscribe_socket, unexpected_wire_frame,
};
use super::*;

#[cfg(unix)]
pub(super) type SocketLines =
    tokio::io::Lines<tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>>;

#[cfg(unix)]
pub(super) type SocketWriter = tokio::net::unix::OwnedWriteHalf;

pub struct SessionSubscription {
    pub snapshot: SessionSnapshot,
    inner: SessionSubscriptionInner,
}

enum SessionSubscriptionInner {
    InProcess {
        events: broadcast::Receiver<SessionEvent>,
    },
    #[cfg(unix)]
    Socket {
        lines: SocketLines,
        _writer: SocketWriter,
    },
}

impl SessionSubscription {
    pub(super) fn in_process(
        snapshot: SessionSnapshot,
        events: broadcast::Receiver<SessionEvent>,
    ) -> Self {
        Self {
            snapshot,
            inner: SessionSubscriptionInner::InProcess { events },
        }
    }

    #[cfg(unix)]
    pub(super) fn socket(
        snapshot: SessionSnapshot,
        lines: SocketLines,
        writer: SocketWriter,
    ) -> Self {
        Self {
            snapshot,
            inner: SessionSubscriptionInner::Socket {
                lines,
                _writer: writer,
            },
        }
    }

    pub async fn next_event(&mut self) -> Result<SessionEvent, SessionError> {
        match &mut self.inner {
            SessionSubscriptionInner::InProcess { events } => {
                events.recv().await.map_err(|error| match error {
                    broadcast::error::RecvError::Closed => SessionError::Closed,
                    broadcast::error::RecvError::Lagged(skipped) => {
                        SessionError::Lagged { skipped }
                    }
                })
            }
            #[cfg(unix)]
            SessionSubscriptionInner::Socket { lines, .. } => match read_wire_frame(lines).await? {
                SessionWireFrame::Event { event } => Ok(event),
                SessionWireFrame::Error { error, .. } => Err(error),
                frame => Err(unexpected_wire_frame("event", frame)),
            },
        }
    }
}

#[derive(Clone)]
pub struct SessionClient {
    inner: Arc<SessionClientInner>,
}

pub(super) enum SessionClientInner {
    InProcess {
        tx: mpsc::Sender<SessionRequest>,
    },
    #[cfg(unix)]
    Socket {
        path: PathBuf,
        next_id: AtomicU64,
    },
}

impl SessionClient {
    pub(super) fn in_process(tx: mpsc::Sender<SessionRequest>) -> Self {
        Self {
            inner: Arc::new(SessionClientInner::InProcess { tx }),
        }
    }

    pub fn connect_local(descriptor: &SessionDescriptor) -> Result<Self, SessionError> {
        if descriptor.protocol_version != SESSION_PROTOCOL_VERSION {
            return Err(SessionError::Transport {
                message: format!(
                    "unsupported session protocol version {}",
                    descriptor.protocol_version
                ),
            });
        }
        Self::connect_socket(descriptor.socket_path.clone())
    }

    #[cfg(unix)]
    pub fn connect_socket(path: impl Into<PathBuf>) -> Result<Self, SessionError> {
        Ok(Self {
            inner: Arc::new(SessionClientInner::Socket {
                path: path.into(),
                next_id: AtomicU64::new(1),
            }),
        })
    }

    #[cfg(not(unix))]
    pub fn connect_socket(_path: impl Into<PathBuf>) -> Result<Self, SessionError> {
        Err(SessionError::Transport {
            message: "native local transport is not implemented on this platform yet".into(),
        })
    }

    pub async fn add(&self, request: DownloadRequest) -> Result<DownloadId, SessionError> {
        match self.dispatch(SessionCommand::Add { request }).await? {
            SessionResponse::DownloadAdded { id } => Ok(id),
            _ => Err(unexpected_response("add")),
        }
    }

    pub async fn pause(&self, id: DownloadId) -> Result<(), SessionError> {
        self.expect_ack(SessionCommand::Pause { id }).await
    }

    pub async fn resume(&self, id: DownloadId) -> Result<(), SessionError> {
        self.expect_ack(SessionCommand::Resume { id }).await
    }

    pub async fn cancel(&self, id: DownloadId) -> Result<(), SessionError> {
        self.expect_ack(SessionCommand::Cancel { id }).await
    }

    pub async fn delete_artifact(&self, id: DownloadId) -> Result<(), SessionError> {
        self.expect_ack(SessionCommand::DeleteArtifact { id }).await
    }

    pub async fn update_config(&self, config: CoreConfig) -> Result<(), SessionError> {
        self.expect_ack(SessionCommand::UpdateConfig { config })
            .await
    }

    pub async fn load_history(
        &self,
        filter: HistoryFilter,
        query: impl Into<String>,
    ) -> Result<Vec<HistoryRow>, SessionError> {
        match self
            .dispatch(SessionCommand::LoadHistory {
                filter,
                query: query.into(),
            })
            .await?
        {
            SessionResponse::History { rows } => Ok(rows),
            _ => Err(unexpected_response("load_history")),
        }
    }

    pub async fn snapshot(&self) -> Result<SessionSnapshot, SessionError> {
        match self.dispatch(SessionCommand::Snapshot).await? {
            SessionResponse::Snapshot { snapshot } => Ok(snapshot),
            _ => Err(unexpected_response("snapshot")),
        }
    }

    pub async fn subscribe(&self) -> Result<SessionSubscription, SessionError> {
        match &*self.inner {
            SessionClientInner::InProcess { tx } => {
                let (reply_tx, reply_rx) = oneshot::channel();
                tx.send(SessionRequest::Subscribe { reply: reply_tx })
                    .await
                    .map_err(|_| SessionError::Closed)?;
                reply_rx.await.map_err(|_| SessionError::Closed)?
            }
            #[cfg(unix)]
            SessionClientInner::Socket { path, next_id } => {
                let id = next_id.fetch_add(1, Ordering::Relaxed);
                subscribe_socket(path, id).await
            }
        }
    }

    pub(super) async fn shutdown_embedded(&self) -> Result<(), SessionError> {
        match &*self.inner {
            SessionClientInner::InProcess { tx } => {
                let (reply_tx, reply_rx) = oneshot::channel();
                tx.send(SessionRequest::Shutdown { reply: reply_tx })
                    .await
                    .map_err(|_| SessionError::Closed)?;
                match reply_rx.await.map_err(|_| SessionError::Closed)?? {
                    SessionResponse::Ack => Ok(()),
                    _ => Err(unexpected_response("shutdown")),
                }
            }
            #[cfg(unix)]
            SessionClientInner::Socket { .. } => Err(SessionError::BadRequest {
                message: "shutdown is only available to the embedded session owner".into(),
            }),
        }
    }

    pub(super) fn try_shutdown_embedded(&self) {
        if let SessionClientInner::InProcess { tx } = &*self.inner {
            let _ = tx.try_send(SessionRequest::Shutdown {
                reply: oneshot::channel().0,
            });
        }
    }

    async fn expect_ack(&self, command: SessionCommand) -> Result<(), SessionError> {
        match self.dispatch(command).await? {
            SessionResponse::Ack => Ok(()),
            _ => Err(unexpected_response("ack")),
        }
    }

    pub(super) async fn dispatch(
        &self,
        command: SessionCommand,
    ) -> Result<SessionResponse, SessionError> {
        match &*self.inner {
            SessionClientInner::InProcess { tx } => {
                let (reply_tx, reply_rx) = oneshot::channel();
                tx.send(SessionRequest::Command {
                    command,
                    reply: reply_tx,
                })
                .await
                .map_err(|_| SessionError::Closed)?;
                reply_rx.await.map_err(|_| SessionError::Closed)?
            }
            #[cfg(unix)]
            SessionClientInner::Socket { path, next_id } => {
                let id = next_id.fetch_add(1, Ordering::Relaxed);
                dispatch_socket(path, id, command).await
            }
        }
    }
}

fn unexpected_response(command: &str) -> SessionError {
    SessionError::BadRequest {
        message: format!("unexpected response for {command}"),
    }
}
