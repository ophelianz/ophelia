use super::lock::{ServiceLock, service_lock_path};
use super::read_model::{OpheliaEventCoalescer, OpheliaReadModel};
use super::transfer_runtime::{TransferRuntime, TransferRuntimeEvent};
use super::*;
use crate::disk::DiskHandle;

const SERVICE_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const TASK_UPDATE_DRAIN_AFTER_COMMAND: usize = 64;

pub struct OpheliaService {
    client: OpheliaClient,
    task: Option<JoinHandle<()>>,
}

impl OpheliaService {
    pub fn start(runtime: &Handle, paths: ProfilePaths) -> Result<Self, OpheliaError> {
        let settings = ServiceSettings::load(&paths);
        Self::start_with_settings(runtime, paths, settings)
    }

    pub fn start_with_settings(
        runtime: &Handle,
        paths: ProfilePaths,
        settings: ServiceSettings,
    ) -> Result<Self, OpheliaError> {
        let config = settings.to_engine_config(&paths);
        Self::start_inner(runtime, paths, settings, config)
    }

    #[cfg(test)]
    pub(crate) fn start_with_engine_config(
        runtime: &Handle,
        config: EngineConfig,
        paths: ProfilePaths,
    ) -> Result<Self, OpheliaError> {
        let settings = ServiceSettings::default_for_paths(&paths);
        Self::start_inner(runtime, paths, settings, config)
    }

    #[cfg(test)]
    pub(crate) fn start_with_engine_config_and_idle_timeout(
        runtime: &Handle,
        config: EngineConfig,
        paths: ProfilePaths,
        idle_timeout: Duration,
    ) -> Result<Self, OpheliaError> {
        let settings = ServiceSettings::default_for_paths(&paths);
        Self::start_inner_with_idle_timeout(runtime, paths, settings, config, idle_timeout)
    }

    fn start_inner(
        runtime: &Handle,
        paths: ProfilePaths,
        settings: ServiceSettings,
        config: EngineConfig,
    ) -> Result<Self, OpheliaError> {
        Self::start_inner_with_idle_timeout(runtime, paths, settings, config, SERVICE_IDLE_TIMEOUT)
    }

    fn start_inner_with_idle_timeout(
        runtime: &Handle,
        paths: ProfilePaths,
        settings: ServiceSettings,
        config: EngineConfig,
        idle_timeout: Duration,
    ) -> Result<Self, OpheliaError> {
        let lock = ServiceLock::acquire(service_lock_path(&paths))?;
        let bootstrap = state::bootstrap(&paths).map_err(|error| OpheliaError::Io {
            message: error.to_string(),
        })?;
        let service_info = OpheliaServiceInfo::current(&paths);
        let (tx, rx) = mpsc::channel(SERVICE_COMMAND_CAPACITY);
        let (event_tx, _) = broadcast::channel(SERVICE_EVENT_CAPACITY);
        let client = OpheliaClient::in_process(tx);
        let db_tx = bootstrap.db_tx.clone();
        let disk = DiskHandle::new();
        let transfers = TransferRuntime::new(
            config.clone(),
            db_tx.clone(),
            bootstrap.next_download_id,
            disk.clone(),
        );
        let task = runtime.spawn(
            OpheliaServiceRuntime {
                config,
                settings,
                paths,
                service_info,
                transfers,
                disk,
                db_tx,
                saved_downloads: bootstrap.saved_downloads,
                _db_worker: bootstrap.worker,
                _lock: lock,
                read_model: OpheliaReadModel::default(),
                coalescer: OpheliaEventCoalescer::default(),
                event_tx,
                idle_timeout,
            }
            .run(rx),
        );

        Ok(Self {
            client,
            task: Some(task),
        })
    }

    pub fn client(&self) -> OpheliaClient {
        self.client.clone()
    }

    pub async fn shutdown(mut self) -> Result<(), OpheliaError> {
        let result = self.client.shutdown_embedded().await;
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        result
    }

    pub async fn wait(mut self) {
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for OpheliaService {
    fn drop(&mut self) {
        self.client.try_shutdown_embedded();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

type OpheliaReply = oneshot::Sender<Result<OpheliaResponse, OpheliaError>>;
type SubscribeReply = oneshot::Sender<Result<OpheliaSubscription, OpheliaError>>;

pub(super) enum OpheliaRequest {
    Command {
        command: OpheliaCommand,
        reply: OpheliaReply,
    },
    Subscribe {
        reply: SubscribeReply,
    },
    Shutdown {
        reply: OpheliaReply,
    },
}

struct OpheliaServiceRuntime {
    config: EngineConfig,
    settings: ServiceSettings,
    paths: ProfilePaths,
    service_info: OpheliaServiceInfo,
    transfers: TransferRuntime,
    disk: DiskHandle,
    db_tx: std::sync::mpsc::Sender<DbEvent>,
    saved_downloads: Vec<crate::engine::SavedDownload>,
    _db_worker: state::DbWorkerHandle,
    _lock: ServiceLock,
    read_model: OpheliaReadModel,
    coalescer: OpheliaEventCoalescer,
    event_tx: broadcast::Sender<OpheliaEvent>,
    idle_timeout: Duration,
}

impl OpheliaServiceRuntime {
    async fn run(mut self, mut rx: mpsc::Receiver<OpheliaRequest>) {
        self.restore_saved_downloads().await;
        let mut hot_event_flush =
            tokio::time::interval(Duration::from_millis(HOT_SERVICE_EVENT_FLUSH_MS));
        hot_event_flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            if self.should_idle_exit() {
                tokio::select! {
                    _ = tokio::time::sleep(self.idle_timeout) => {
                        if self.should_idle_exit() {
                            break;
                        }
                    }
                    should_stop = self.next_runtime_step(&mut rx, &mut hot_event_flush) => {
                        if should_stop {
                            break;
                        }
                    }
                }
            } else if self.next_runtime_step(&mut rx, &mut hot_event_flush).await {
                break;
            }
        }

        self.flush_coalesced_events();
        self.log_coalescer_stats();
        self.transfers.shutdown();
    }

    async fn restore_saved_downloads(&mut self) {
        for saved in std::mem::take(&mut self.saved_downloads) {
            let restored = RestoredDownload::from_saved(&saved, &self.config);
            let (result, events) = self.transfers.restore(restored).await;
            self.apply_transfer_events(events);
            self.flush_coalesced_events();
            if let Err(error) = result {
                tracing::warn!(id = saved.id.0, "restore failed: {error}");
            }
            self.drain_transfer_updates(TASK_UPDATE_DRAIN_AFTER_COMMAND)
                .await;
        }
    }

    async fn next_runtime_step(
        &mut self,
        rx: &mut mpsc::Receiver<OpheliaRequest>,
        hot_event_flush: &mut tokio::time::Interval,
    ) -> bool {
        tokio::select! {
            request = rx.recv() => {
                let Some(request) = request else {
                    return true;
                };
                self.drain_transfer_updates(TASK_UPDATE_DRAIN_AFTER_COMMAND).await;
                self.flush_coalesced_events();
                if self.handle_request(request).await {
                    return true;
                }
                self.drain_transfer_updates(TASK_UPDATE_DRAIN_AFTER_COMMAND).await;
            }
            events = self.transfers.next_update() => {
                let Some(events) = events else {
                    return true;
                };
                self.apply_transfer_events(events);
            }
            _ = hot_event_flush.tick() => {
                self.flush_coalesced_events();
            }
        }
        false
    }

    async fn handle_request(&mut self, request: OpheliaRequest) -> bool {
        match request {
            OpheliaRequest::Command { command, reply } => {
                let response = self.handle_command(command).await;
                self.flush_coalesced_events();
                let _ = reply.send(response);
                false
            }
            OpheliaRequest::Subscribe { reply } => {
                let receiver = self.event_tx.subscribe();
                let snapshot = self.read_model.snapshot(&self.settings);
                let _ = reply.send(Ok(OpheliaSubscription::in_process(snapshot, receiver)));
                false
            }
            OpheliaRequest::Shutdown { reply } => {
                self.flush_coalesced_events();
                let _ = reply.send(Ok(OpheliaResponse::Ack));
                true
            }
        }
    }

    async fn handle_command(
        &mut self,
        command: OpheliaCommand,
    ) -> Result<OpheliaResponse, OpheliaError> {
        match command {
            OpheliaCommand::Add { request } => {
                let spec = request.into_spec(&self.config)?;
                let (result, events) = self.transfers.add(spec).await;
                self.apply_transfer_events(events);
                let id = result?;
                Ok(OpheliaResponse::TransferAdded { id })
            }
            OpheliaCommand::Pause { id } => {
                let (result, events) = self.transfers.pause(id).await;
                self.apply_transfer_events(events);
                result?;
                Ok(OpheliaResponse::Ack)
            }
            OpheliaCommand::Resume { id } => {
                let (result, events) = self.transfers.resume(id).await;
                self.apply_transfer_events(events);
                result?;
                Ok(OpheliaResponse::Ack)
            }
            OpheliaCommand::Cancel { id } => {
                let (result, events) = self.transfers.cancel(id).await;
                self.apply_transfer_events(events);
                result?;
                Ok(OpheliaResponse::Ack)
            }
            OpheliaCommand::DeleteArtifact { id } => {
                let (result, events) = self.transfers.delete_artifact(id).await;
                self.apply_transfer_events(events);
                match result {
                    Ok(()) => {}
                    Err(EngineError::NotFound { id }) => {
                        self.delete_artifact_from_service_state(id).await?;
                    }
                    Err(error) => return Err(error.into()),
                }
                Ok(OpheliaResponse::Ack)
            }
            OpheliaCommand::UpdateSettings { settings } => {
                settings.save(&self.paths)?;
                let config = settings.to_engine_config(&self.paths);
                let (result, events) = self.transfers.update_config(config.clone()).await;
                self.apply_transfer_events(events);
                result?;
                self.config = config;
                self.settings = settings.clone();
                let _ = self
                    .event_tx
                    .send(OpheliaEvent::SettingsChanged { settings });
                Ok(OpheliaResponse::Ack)
            }
            OpheliaCommand::LoadHistory { filter, query } => {
                let rows = load_history_rows(self.paths.clone(), filter, query).await?;
                Ok(OpheliaResponse::History { rows })
            }
            OpheliaCommand::ServiceInfo => Ok(OpheliaResponse::ServiceInfo {
                info: self.service_info.clone(),
            }),
            OpheliaCommand::Snapshot => Ok(OpheliaResponse::Snapshot {
                snapshot: self.read_model.snapshot(&self.settings),
            }),
            OpheliaCommand::Subscribe => Ok(OpheliaResponse::Snapshot {
                snapshot: self.read_model.snapshot(&self.settings),
            }),
        }
    }

    async fn drain_transfer_updates(&mut self, limit: usize) {
        let events = self.transfers.drain_updates(limit).await;
        self.apply_transfer_events(events);
    }

    fn apply_transfer_events(&mut self, events: Vec<TransferRuntimeEvent>) {
        for event in events {
            self.apply_transfer_event(event);
        }
    }

    fn apply_transfer_event(&mut self, event: TransferRuntimeEvent) {
        if should_flush_before_immediate(&event) {
            self.flush_coalesced_events();
        }
        if let Some(event) = self
            .read_model
            .apply_transfer_event(event, &mut self.coalescer)
        {
            let _ = self.event_tx.send(event);
        }
    }

    fn flush_coalesced_events(&mut self) {
        for event in self.coalescer.drain_events() {
            let _ = self.event_tx.send(event);
        }
    }

    fn log_coalescer_stats(&self) {
        let stats = self.coalescer.stats();
        if stats.raw_transfer_updates == 0 && stats.raw_write_updates == 0 {
            return;
        }
        tracing::debug!(
            raw_transfer_updates = stats.raw_transfer_updates,
            emitted_transfer_updates = stats.emitted_transfer_updates,
            coalesced_transfer_updates = stats.coalesced_transfer_updates(),
            raw_write_updates = stats.raw_write_updates,
            emitted_write_updates = stats.emitted_write_updates,
            coalesced_write_updates = stats.coalesced_write_updates(),
            "service event coalescing summary"
        );
    }

    fn should_idle_exit(&self) -> bool {
        self.event_tx.receiver_count() == 0 && !self.read_model.has_running_transfers()
    }

    async fn delete_artifact_from_service_state(
        &mut self,
        id: TransferId,
    ) -> Result<(), OpheliaError> {
        let destination = if let Some(destination) = self.read_model.destination(id) {
            Some(destination.to_path_buf())
        } else {
            load_history_row_by_id(self.paths.clone(), id)
                .await?
                .map(|row| PathBuf::from(row.destination))
        }
        .ok_or(OpheliaError::NotFound { id })?;

        let artifact_state = self.disk.delete_artifacts(&destination);
        let _ = self
            .db_tx
            .send(DbEvent::ArtifactStateChanged { id, artifact_state });
        self.read_model.remove(id);
        let _ = self.event_tx.send(OpheliaEvent::TransferRemoved {
            id,
            action: LiveTransferRemovalAction::DeleteArtifact,
            artifact_state,
        });
        Ok(())
    }
}

async fn load_history_rows(
    paths: ProfilePaths,
    filter: HistoryFilter,
    query: String,
) -> Result<Vec<HistoryRow>, OpheliaError> {
    tokio::task::spawn_blocking(move || {
        let reader = HistoryReader::open(&paths).map_err(history_error)?;
        reader.load(filter, &query).map_err(history_error)
    })
    .await
    .map_err(history_join_error)?
}

async fn load_history_row_by_id(
    paths: ProfilePaths,
    id: TransferId,
) -> Result<Option<HistoryRow>, OpheliaError> {
    tokio::task::spawn_blocking(move || {
        let reader = HistoryReader::open(&paths).map_err(history_error)?;
        reader.load_by_id(id).map_err(history_error)
    })
    .await
    .map_err(history_join_error)?
}

fn history_error(error: rusqlite::Error) -> OpheliaError {
    OpheliaError::Io {
        message: error.to_string(),
    }
}

fn history_join_error(error: tokio::task::JoinError) -> OpheliaError {
    OpheliaError::Io {
        message: format!("history query worker failed: {error}"),
    }
}

pub(super) fn should_flush_before_immediate(event: &TransferRuntimeEvent) -> bool {
    match event {
        TransferRuntimeEvent::Progress(update) => update.status != TransferStatus::Downloading,
        TransferRuntimeEvent::TransferRemoved { .. } => true,
        _ => false,
    }
}
