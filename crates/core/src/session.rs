use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::config::{CoreConfig, CorePaths};
use crate::engine::state::{self, HistoryReader};
use crate::engine::{
    AddDownloadRequest, ArtifactState, DownloadControlAction, DownloadEngine, DownloadId,
    DownloadSpec, EngineError, EngineEvent, HistoryFilter, HistoryRow, LiveTransferRemovalAction,
    ProgressUpdate, RestoredDownload, TransferSnapshot,
};

const SESSION_COMMAND_CAPACITY: usize = 64;
const SESSION_EVENT_CAPACITY: usize = 512;
const SESSION_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub source: DownloadRequestSource,
    pub destination: DownloadDestination,
}

impl DownloadRequest {
    pub fn http(url: String) -> Self {
        Self {
            source: DownloadRequestSource::Http { url },
            destination: DownloadDestination::Automatic {
                suggested_filename: None,
            },
        }
    }

    pub fn from_add_request(request: AddDownloadRequest) -> Self {
        Self {
            source: DownloadRequestSource::Http {
                url: request.url().to_string(),
            },
            destination: DownloadDestination::Automatic {
                suggested_filename: request.suggested_filename,
            },
        }
    }

    pub fn into_spec(self, config: &CoreConfig) -> io::Result<DownloadSpec> {
        match (self.source, self.destination) {
            (
                DownloadRequestSource::Http { url },
                DownloadDestination::Automatic { suggested_filename },
            ) => DownloadSpec::from_auto_request(url, suggested_filename, config),
            (DownloadRequestSource::Http { url }, DownloadDestination::ExplicitPath(path)) => {
                DownloadSpec::from_user_input(url, path, config)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadRequestSource {
    Http { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadDestination {
    Automatic { suggested_filename: Option<String> },
    ExplicitPath(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionCommand {
    Add {
        request: DownloadRequest,
    },
    Pause {
        id: DownloadId,
    },
    Resume {
        id: DownloadId,
    },
    Cancel {
        id: DownloadId,
    },
    DeleteArtifact {
        id: DownloadId,
    },
    UpdateConfig {
        config: CoreConfig,
    },
    LoadHistory {
        filter: HistoryFilter,
        query: String,
    },
    Snapshot,
    Subscribe,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionResponse {
    Ack,
    DownloadAdded { id: DownloadId },
    History { rows: Vec<HistoryRow> },
    Snapshot { snapshot: SessionSnapshot },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvent {
    TransferChanged {
        snapshot: TransferSnapshot,
    },
    DownloadBytesWritten {
        id: DownloadId,
        bytes: u64,
    },
    TransferRemoved {
        id: DownloadId,
        action: LiveTransferRemovalAction,
        artifact_state: ArtifactState,
    },
    ControlUnsupported {
        id: DownloadId,
        action: DownloadControlAction,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub transfers: Vec<TransferSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionError {
    Closed,
    NotFound {
        id: DownloadId,
    },
    Unsupported {
        id: DownloadId,
        action: DownloadControlAction,
    },
    LockHeld {
        path: PathBuf,
    },
    StaleSession {
        path: PathBuf,
    },
    BadRequest {
        message: String,
    },
    Io {
        message: String,
    },
    Transport {
        message: String,
    },
    Lagged {
        skipped: u64,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => write!(f, "session is closed"),
            Self::NotFound { id } => write!(f, "download {} was not found", id.0),
            Self::Unsupported { id, action } => {
                write!(f, "download {} does not support {action:?}", id.0)
            }
            Self::LockHeld { path } => write!(f, "session lock is held at {}", path.display()),
            Self::StaleSession { path } => {
                write!(f, "stale session descriptor at {}", path.display())
            }
            Self::BadRequest { message } => write!(f, "bad request: {message}"),
            Self::Io { message } => write!(f, "io error: {message}"),
            Self::Transport { message } => write!(f, "transport error: {message}"),
            Self::Lagged { skipped } => write!(f, "session skipped {skipped} events"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<EngineError> for SessionError {
    fn from(error: EngineError) -> Self {
        match error {
            EngineError::Closed => Self::Closed,
            EngineError::NotFound { id } => Self::NotFound { id },
            EngineError::Unsupported { id, action } => Self::Unsupported { id, action },
        }
    }
}

impl From<io::Error> for SessionError {
    fn from(error: io::Error) -> Self {
        Self::Io {
            message: error.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDescriptor {
    pub protocol_version: u32,
    pub pid: u32,
    pub profile_database_path: PathBuf,
    pub socket_path: PathBuf,
    pub created_unix_ms: u128,
}

impl SessionDescriptor {
    pub fn for_paths(paths: &CorePaths) -> Self {
        Self {
            protocol_version: SESSION_PROTOCOL_VERSION,
            pid: std::process::id(),
            profile_database_path: paths.database_path.clone(),
            socket_path: session_socket_path(paths),
            created_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or_default(),
        }
    }
}

pub struct SessionSubscription {
    pub snapshot: SessionSnapshot,
    events: broadcast::Receiver<SessionEvent>,
}

impl SessionSubscription {
    pub async fn next_event(&mut self) -> Result<SessionEvent, SessionError> {
        self.events.recv().await.map_err(|error| match error {
            broadcast::error::RecvError::Closed => SessionError::Closed,
            broadcast::error::RecvError::Lagged(skipped) => SessionError::Lagged { skipped },
        })
    }
}

#[derive(Clone)]
pub struct SessionClient {
    tx: mpsc::Sender<SessionRequest>,
}

impl SessionClient {
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
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(SessionRequest::Subscribe { reply: reply_tx })
            .await
            .map_err(|_| SessionError::Closed)?;
        reply_rx.await.map_err(|_| SessionError::Closed)?
    }

    pub async fn shutdown(&self) -> Result<(), SessionError> {
        self.expect_ack(SessionCommand::Shutdown).await
    }

    async fn expect_ack(&self, command: SessionCommand) -> Result<(), SessionError> {
        match self.dispatch(command).await? {
            SessionResponse::Ack => Ok(()),
            _ => Err(unexpected_response("ack")),
        }
    }

    pub async fn dispatch(&self, command: SessionCommand) -> Result<SessionResponse, SessionError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(SessionRequest::Command {
                command,
                reply: reply_tx,
            })
            .await
            .map_err(|_| SessionError::Closed)?;
        reply_rx.await.map_err(|_| SessionError::Closed)?
    }
}

fn unexpected_response(command: &str) -> SessionError {
    SessionError::BadRequest {
        message: format!("unexpected response for {command}"),
    }
}

pub struct SessionHost {
    client: SessionClient,
    task: Option<JoinHandle<()>>,
    transport: Option<LocalSessionServer>,
}

impl SessionHost {
    pub fn start(
        runtime: &Handle,
        config: CoreConfig,
        paths: CorePaths,
    ) -> Result<Self, SessionError> {
        let lock = SessionLock::acquire(session_lock_path(&paths))?;
        let bootstrap = state::bootstrap(&paths).map_err(|error| SessionError::Io {
            message: error.to_string(),
        })?;
        let descriptor = SessionDescriptor::for_paths(&paths);
        let (tx, rx) = mpsc::channel(SESSION_COMMAND_CAPACITY);
        let (event_tx, _) = broadcast::channel(SESSION_EVENT_CAPACITY);
        let client = SessionClient { tx };
        let transport = LocalSessionServer::start(runtime, descriptor, client.clone())?;

        let engine = DownloadEngine::spawn_on(
            runtime,
            config.clone(),
            bootstrap.db_tx,
            bootstrap.next_download_id,
        );
        let task = runtime.spawn(
            SessionRuntime {
                config,
                engine,
                history_reader: bootstrap.history_reader,
                saved_downloads: bootstrap.saved_downloads,
                _db_worker: bootstrap.worker,
                _lock: lock,
                read_model: SessionReadModel::default(),
                event_tx,
            }
            .run(rx),
        );

        Ok(Self {
            client,
            task: Some(task),
            transport: Some(transport),
        })
    }

    pub fn client(&self) -> SessionClient {
        self.client.clone()
    }

    pub fn descriptor(&self) -> Option<&SessionDescriptor> {
        self.transport.as_ref().map(LocalSessionServer::descriptor)
    }

    pub async fn shutdown(mut self) -> Result<(), SessionError> {
        let result = self.client.shutdown().await;
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        drop(self.transport.take());
        result
    }
}

impl Drop for SessionHost {
    fn drop(&mut self) {
        let _ = self.client.tx.try_send(SessionRequest::Command {
            command: SessionCommand::Shutdown,
            reply: oneshot::channel().0,
        });
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

type SessionReply = oneshot::Sender<Result<SessionResponse, SessionError>>;
type SubscribeReply = oneshot::Sender<Result<SessionSubscription, SessionError>>;

enum SessionRequest {
    Command {
        command: SessionCommand,
        reply: SessionReply,
    },
    Subscribe {
        reply: SubscribeReply,
    },
}

struct SessionRuntime {
    config: CoreConfig,
    engine: DownloadEngine,
    history_reader: HistoryReader,
    saved_downloads: Vec<crate::engine::SavedDownload>,
    _db_worker: state::DbWorkerHandle,
    _lock: SessionLock,
    read_model: SessionReadModel,
    event_tx: broadcast::Sender<SessionEvent>,
}

impl SessionRuntime {
    async fn run(mut self, mut rx: mpsc::Receiver<SessionRequest>) {
        self.restore_saved_downloads().await;

        loop {
            tokio::select! {
                request = rx.recv() => {
                    let Some(request) = request else {
                        break;
                    };
                    self.drain_engine_events();
                    if self.handle_request(request).await {
                        break;
                    }
                    self.drain_engine_events();
                }
                event = self.engine.next_event() => {
                    let Some(event) = event else {
                        break;
                    };
                    self.apply_engine_event(event);
                }
            }
        }

        if let Err(error) = self.engine.shutdown().await {
            tracing::warn!("download engine shutdown failed: {error}");
        }
    }

    async fn restore_saved_downloads(&mut self) {
        for saved in std::mem::take(&mut self.saved_downloads) {
            let restored = RestoredDownload::from_saved(&saved, &self.config);
            if let Err(error) = self.engine.restore(restored).await {
                tracing::warn!(id = saved.id.0, "restore failed: {error}");
            }
            self.drain_engine_events();
        }
    }

    async fn handle_request(&mut self, request: SessionRequest) -> bool {
        match request {
            SessionRequest::Command { command, reply } => {
                let shutdown = matches!(command, SessionCommand::Shutdown);
                let response = self.handle_command(command).await;
                let _ = reply.send(response);
                shutdown
            }
            SessionRequest::Subscribe { reply } => {
                let receiver = self.event_tx.subscribe();
                let snapshot = self.read_model.snapshot();
                let _ = reply.send(Ok(SessionSubscription {
                    snapshot,
                    events: receiver,
                }));
                false
            }
        }
    }

    async fn handle_command(
        &mut self,
        command: SessionCommand,
    ) -> Result<SessionResponse, SessionError> {
        match command {
            SessionCommand::Add { request } => {
                let spec = request.into_spec(&self.config)?;
                let id = self.engine.add(spec).await?;
                Ok(SessionResponse::DownloadAdded { id })
            }
            SessionCommand::Pause { id } => {
                self.engine.pause(id).await?;
                Ok(SessionResponse::Ack)
            }
            SessionCommand::Resume { id } => {
                self.engine.resume(id).await?;
                Ok(SessionResponse::Ack)
            }
            SessionCommand::Cancel { id } => {
                self.engine.cancel(id).await?;
                Ok(SessionResponse::Ack)
            }
            SessionCommand::DeleteArtifact { id } => {
                self.engine.delete_artifact(id).await?;
                Ok(SessionResponse::Ack)
            }
            SessionCommand::UpdateConfig { config } => {
                self.engine.update_config(config.clone()).await?;
                self.config = config;
                Ok(SessionResponse::Ack)
            }
            SessionCommand::LoadHistory { filter, query } => {
                let rows =
                    self.history_reader
                        .load(filter, &query)
                        .map_err(|error| SessionError::Io {
                            message: error.to_string(),
                        })?;
                Ok(SessionResponse::History { rows })
            }
            SessionCommand::Snapshot => Ok(SessionResponse::Snapshot {
                snapshot: self.read_model.snapshot(),
            }),
            SessionCommand::Subscribe => Ok(SessionResponse::Snapshot {
                snapshot: self.read_model.snapshot(),
            }),
            SessionCommand::Shutdown => Ok(SessionResponse::Ack),
        }
    }

    fn drain_engine_events(&mut self) {
        while let Some(event) = self.engine.try_next_event() {
            self.apply_engine_event(event);
        }
    }

    fn apply_engine_event(&mut self, event: EngineEvent) {
        if let Some(event) = self.read_model.apply_engine_event(event) {
            let _ = self.event_tx.send(event);
        }
    }
}

#[derive(Default)]
struct SessionReadModel {
    transfers: HashMap<DownloadId, TransferSnapshot>,
    order: Vec<DownloadId>,
}

impl SessionReadModel {
    fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            transfers: self
                .order
                .iter()
                .filter_map(|id| self.transfers.get(id).cloned())
                .collect(),
        }
    }

    fn apply_engine_event(&mut self, event: EngineEvent) -> Option<SessionEvent> {
        match event {
            EngineEvent::TransferAdded { snapshot }
            | EngineEvent::TransferRestored { snapshot } => {
                let snapshot = self.upsert(snapshot);
                Some(SessionEvent::TransferChanged { snapshot })
            }
            EngineEvent::Progress(update) => self.apply_progress(update),
            EngineEvent::DownloadBytesWritten { id, bytes } => {
                Some(SessionEvent::DownloadBytesWritten { id, bytes })
            }
            EngineEvent::DestinationChanged { id, destination } => self
                .mutate(id, |snapshot| {
                    snapshot.destination = destination;
                })
                .map(|snapshot| SessionEvent::TransferChanged { snapshot }),
            EngineEvent::ControlSupportChanged { id, support } => self
                .mutate(id, |snapshot| {
                    snapshot.control_support = support;
                })
                .map(|snapshot| SessionEvent::TransferChanged { snapshot }),
            EngineEvent::ChunkMapChanged { id, state } => self
                .mutate(id, |snapshot| {
                    snapshot.chunk_map_state = state;
                })
                .map(|snapshot| SessionEvent::TransferChanged { snapshot }),
            EngineEvent::LiveTransferRemoved {
                id,
                action,
                artifact_state,
            } => {
                self.transfers.remove(&id);
                self.order.retain(|current| *current != id);
                Some(SessionEvent::TransferRemoved {
                    id,
                    action,
                    artifact_state,
                })
            }
            EngineEvent::ControlUnsupported { id, action } => {
                Some(SessionEvent::ControlUnsupported { id, action })
            }
        }
    }

    fn apply_progress(&mut self, update: ProgressUpdate) -> Option<SessionEvent> {
        self.mutate(update.id, |snapshot| {
            snapshot.status = update.status;
            snapshot.downloaded_bytes = update.downloaded_bytes;
            snapshot.total_bytes = update.total_bytes;
            snapshot.speed_bytes_per_sec = update.speed_bytes_per_sec;
        })
        .map(|snapshot| SessionEvent::TransferChanged { snapshot })
    }

    fn upsert(&mut self, snapshot: TransferSnapshot) -> TransferSnapshot {
        if !self.transfers.contains_key(&snapshot.id) {
            self.order.push(snapshot.id);
        }
        self.transfers.insert(snapshot.id, snapshot.clone());
        snapshot
    }

    fn mutate(
        &mut self,
        id: DownloadId,
        mutate: impl FnOnce(&mut TransferSnapshot),
    ) -> Option<TransferSnapshot> {
        let snapshot = self.transfers.get_mut(&id)?;
        mutate(snapshot);
        Some(snapshot.clone())
    }
}

struct SessionLock {
    path: PathBuf,
    _file: File,
}

impl SessionLock {
    fn acquire(path: PathBuf) -> Result<Self, SessionError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    SessionError::LockHeld { path: path.clone() }
                } else {
                    SessionError::Io {
                        message: error.to_string(),
                    }
                }
            })?;
        let _ = writeln!(file, "pid={}", std::process::id());
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

pub fn session_lock_path(paths: &CorePaths) -> PathBuf {
    session_dir(paths).join("downloads.session.lock")
}

pub fn session_descriptor_path(paths: &CorePaths) -> PathBuf {
    session_dir(paths).join("downloads.session.json")
}

pub fn session_socket_path(paths: &CorePaths) -> PathBuf {
    session_dir(paths).join("downloads.session.sock")
}

fn session_dir(paths: &CorePaths) -> PathBuf {
    paths
        .database_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

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
pub struct LocalSessionServer {
    descriptor: SessionDescriptor,
    descriptor_path: PathBuf,
    socket_path: PathBuf,
    task: JoinHandle<()>,
}

#[cfg(unix)]
impl LocalSessionServer {
    fn start(
        runtime: &Handle,
        descriptor: SessionDescriptor,
        client: SessionClient,
    ) -> Result<Self, SessionError> {
        use tokio::net::UnixListener;

        if let Some(parent) = descriptor.socket_path.parent() {
            fs::create_dir_all(parent)?;
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

    fn descriptor(&self) -> &SessionDescriptor {
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
                    id: 0,
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

#[cfg(unix)]
async fn write_wire_frame(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    frame: &SessionWireFrame,
) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;

    let mut line = serde_json::to_vec(frame)?;
    line.push(b'\n');
    writer.write_all(&line).await
}

#[cfg(not(unix))]
pub struct LocalSessionServer;

#[cfg(not(unix))]
impl LocalSessionServer {
    fn start(
        _runtime: &Handle,
        _descriptor: SessionDescriptor,
        _client: SessionClient,
    ) -> Result<Self, SessionError> {
        Err(SessionError::Transport {
            message: "native local transport is not implemented on this platform yet".into(),
        })
    }

    fn descriptor(&self) -> &SessionDescriptor {
        unreachable!("local session transport is not implemented on this platform yet")
    }
}

fn write_descriptor(path: &Path, descriptor: &SessionDescriptor) -> Result<(), SessionError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(descriptor).map_err(|error| SessionError::Io {
        message: error.to_string(),
    })?;
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    fn test_paths(dir: &TempDir) -> CorePaths {
        CorePaths::new(
            dir.path().join("downloads.db"),
            None,
            dir.path().join("downloads"),
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn session_host_rejects_second_owner_for_profile() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(&dir);
        let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths.clone())
            .expect("first session should start");

        let second = SessionHost::start(&Handle::current(), CoreConfig::default(), paths.clone());

        assert!(matches!(second, Err(SessionError::LockHeld { .. })));
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn new_subscriber_receives_snapshot_before_live_events() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(&dir);
        let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths)
            .expect("session should start");

        let subscription = host.client().subscribe().await.unwrap();

        assert!(subscription.snapshot.transfers.is_empty());
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn history_loads_through_session_client() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(&dir);
        let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths)
            .expect("session should start");

        let rows = host
            .client()
            .load_history(HistoryFilter::All, "")
            .await
            .unwrap();

        assert!(rows.is_empty());
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn download_request_resolves_explicit_destination() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("picked.bin");
        let config = CoreConfig::default_with_download_dir(dir.path().join("downloads"));
        let request = DownloadRequest {
            source: DownloadRequestSource::Http {
                url: "https://example.com/file.bin".into(),
            },
            destination: DownloadDestination::ExplicitPath(destination.clone()),
        };

        let spec = request.into_spec(&config).unwrap();

        assert_eq!(spec.destination(), destination);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn unix_socket_returns_snapshot_response() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(&dir);
        let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths)
            .expect("session should start");
        let socket_path = host.descriptor().unwrap().socket_path.clone();

        let stream = tokio::net::UnixStream::connect(socket_path).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let command = SessionWireCommand {
            id: 7,
            command: SessionCommand::Snapshot,
        };
        let mut body = serde_json::to_vec(&command).unwrap();
        body.push(b'\n');
        writer.write_all(&body).await.unwrap();

        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let frame: SessionWireFrame = serde_json::from_str(&line).unwrap();

        assert!(matches!(
            frame,
            SessionWireFrame::Response {
                id: 7,
                response: SessionResponse::Snapshot { .. }
            }
        ));
        host.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn unix_socket_reports_bad_json_as_bad_request() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(&dir);
        let host = SessionHost::start(&Handle::current(), CoreConfig::default(), paths)
            .expect("session should start");
        let socket_path = host.descriptor().unwrap().socket_path.clone();

        let stream = tokio::net::UnixStream::connect(socket_path).await.unwrap();
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        writer.write_all(b"not json\n").await.unwrap();

        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let frame: SessionWireFrame = serde_json::from_str(&line).unwrap();

        assert!(matches!(
            frame,
            SessionWireFrame::Error {
                error: SessionError::BadRequest { .. },
                ..
            }
        ));
        host.shutdown().await.unwrap();
    }
}
