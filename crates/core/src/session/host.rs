use super::lock::{SessionLock, session_descriptor_path, session_lock_path};
use super::read_model::{SessionEventCoalescer, SessionReadModel};
use super::wire::LocalSessionServer;
use super::*;

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
        let descriptor = SessionDescriptor::for_paths(&paths);
        let lock = SessionLock::acquire(
            session_lock_path(&paths),
            session_descriptor_path(&paths),
            descriptor.socket_path.clone(),
        )?;
        let bootstrap = state::bootstrap(&paths).map_err(|error| SessionError::Io {
            message: error.to_string(),
        })?;
        let (tx, rx) = mpsc::channel(SESSION_COMMAND_CAPACITY);
        let (event_tx, _) = broadcast::channel(SESSION_EVENT_CAPACITY);
        let client = SessionClient::in_process(tx);
        let transport = LocalSessionServer::start(runtime, descriptor, client.clone())?;
        let db_tx = bootstrap.db_tx.clone();

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
                db_tx,
                saved_downloads: bootstrap.saved_downloads,
                _db_worker: bootstrap.worker,
                _lock: lock,
                read_model: SessionReadModel::default(),
                coalescer: SessionEventCoalescer::default(),
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
        let result = self.client.shutdown_embedded().await;
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        drop(self.transport.take());
        result
    }
}

impl Drop for SessionHost {
    fn drop(&mut self) {
        self.client.try_shutdown_embedded();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

type SessionReply = oneshot::Sender<Result<SessionResponse, SessionError>>;
type SubscribeReply = oneshot::Sender<Result<SessionSubscription, SessionError>>;

pub(super) enum SessionRequest {
    Command {
        command: SessionCommand,
        reply: SessionReply,
    },
    Subscribe {
        reply: SubscribeReply,
    },
    Shutdown {
        reply: SessionReply,
    },
}

struct SessionRuntime {
    config: CoreConfig,
    engine: DownloadEngine,
    history_reader: HistoryReader,
    db_tx: std::sync::mpsc::Sender<DbEvent>,
    saved_downloads: Vec<crate::engine::SavedDownload>,
    _db_worker: state::DbWorkerHandle,
    _lock: SessionLock,
    read_model: SessionReadModel,
    coalescer: SessionEventCoalescer,
    event_tx: broadcast::Sender<SessionEvent>,
}

impl SessionRuntime {
    async fn run(mut self, mut rx: mpsc::Receiver<SessionRequest>) {
        self.restore_saved_downloads().await;
        let mut hot_event_flush =
            tokio::time::interval(Duration::from_millis(HOT_SESSION_EVENT_FLUSH_MS));
        hot_event_flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                request = rx.recv() => {
                    let Some(request) = request else {
                        break;
                    };
                    self.drain_engine_events();
                    self.flush_coalesced_events();
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
                _ = hot_event_flush.tick() => {
                    self.flush_coalesced_events();
                }
            }
        }

        self.flush_coalesced_events();
        if let Err(error) = self.engine.shutdown().await {
            tracing::warn!("download engine shutdown failed: {error}");
        }
    }

    async fn restore_saved_downloads(&mut self) {
        for saved in std::mem::take(&mut self.saved_downloads) {
            let restored = RestoredDownload::from_saved(&saved, &self.config);
            let (result, events) = self.engine.restore_collecting_events(restored).await;
            self.apply_engine_events(events);
            self.flush_coalesced_events();
            if let Err(error) = result {
                tracing::warn!(id = saved.id.0, "restore failed: {error}");
            }
            self.drain_engine_events();
        }
    }

    async fn handle_request(&mut self, request: SessionRequest) -> bool {
        match request {
            SessionRequest::Command { command, reply } => {
                let response = self.handle_command(command).await;
                self.flush_coalesced_events();
                let _ = reply.send(response);
                false
            }
            SessionRequest::Subscribe { reply } => {
                let receiver = self.event_tx.subscribe();
                let snapshot = self.read_model.snapshot();
                let _ = reply.send(Ok(SessionSubscription::in_process(snapshot, receiver)));
                false
            }
            SessionRequest::Shutdown { reply } => {
                self.flush_coalesced_events();
                let _ = reply.send(Ok(SessionResponse::Ack));
                true
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
                let (result, events) = self.engine.add_collecting_events(spec).await;
                self.apply_engine_events(events);
                let id = result?;
                Ok(SessionResponse::DownloadAdded { id })
            }
            SessionCommand::Pause { id } => {
                let (result, events) = self.engine.pause_collecting_events(id).await;
                self.apply_engine_events(events);
                result?;
                Ok(SessionResponse::Ack)
            }
            SessionCommand::Resume { id } => {
                let (result, events) = self.engine.resume_collecting_events(id).await;
                self.apply_engine_events(events);
                result?;
                Ok(SessionResponse::Ack)
            }
            SessionCommand::Cancel { id } => {
                let (result, events) = self.engine.cancel_collecting_events(id).await;
                self.apply_engine_events(events);
                result?;
                Ok(SessionResponse::Ack)
            }
            SessionCommand::DeleteArtifact { id } => {
                let (result, events) = self.engine.delete_artifact_collecting_events(id).await;
                self.apply_engine_events(events);
                match result {
                    Ok(()) => {}
                    Err(EngineError::NotFound { id }) => {
                        self.delete_artifact_from_session_state(id)?;
                    }
                    Err(error) => return Err(error.into()),
                }
                Ok(SessionResponse::Ack)
            }
            SessionCommand::UpdateConfig { config } => {
                let (result, events) = self
                    .engine
                    .update_config_collecting_events(config.clone())
                    .await;
                self.apply_engine_events(events);
                result?;
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
        }
    }

    fn drain_engine_events(&mut self) {
        while let Some(event) = self.engine.try_next_event() {
            self.apply_engine_event(event);
        }
    }

    fn apply_engine_events(&mut self, events: Vec<EngineEvent>) {
        for event in events {
            self.apply_engine_event(event);
        }
    }

    fn apply_engine_event(&mut self, event: EngineEvent) {
        if should_flush_before_immediate(&event) {
            self.flush_coalesced_events();
        }
        if let Some(event) = self
            .read_model
            .apply_engine_event(event, &mut self.coalescer)
        {
            let _ = self.event_tx.send(event);
        }
    }

    fn flush_coalesced_events(&mut self) {
        for event in self.coalescer.drain_events() {
            let _ = self.event_tx.send(event);
        }
    }

    fn delete_artifact_from_session_state(&mut self, id: DownloadId) -> Result<(), SessionError> {
        let destination = if let Some(destination) = self.read_model.destination(id) {
            Some(destination.to_path_buf())
        } else {
            self.history_reader
                .load_by_id(id)
                .map_err(|error| SessionError::Io {
                    message: error.to_string(),
                })?
                .map(|row| PathBuf::from(row.destination))
        }
        .ok_or(SessionError::NotFound { id })?;

        let artifact_state = delete_artifact_files(&destination);
        let _ = self
            .db_tx
            .send(DbEvent::ArtifactStateChanged { id, artifact_state });
        self.read_model.remove(id);
        let _ = self.event_tx.send(SessionEvent::TransferRemoved {
            id,
            action: LiveTransferRemovalAction::DeleteArtifact,
            artifact_state,
        });
        Ok(())
    }
}

pub(super) fn should_flush_before_immediate(event: &EngineEvent) -> bool {
    match event {
        EngineEvent::Progress(update) => update.status != DownloadStatus::Downloading,
        EngineEvent::LiveTransferRemoved { .. } => true,
        _ => false,
    }
}
