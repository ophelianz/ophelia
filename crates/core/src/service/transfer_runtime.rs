//! Service-owned transfer runtime
//!
//! Owns active, queued, and paused transfers for one OpheliaService
//! HTTP still does the protocol work; this module owns when transfers start and stop

use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::EngineConfig;
use crate::engine::destination::part_path_for;
use crate::engine::http::TokenBucket;
use crate::engine::provider::{
    self, ProviderRuntimeContext, SchedulerKey, SpawnedTask, TaskDestinationSink, TaskPauseSink,
};
use crate::engine::{
    ArtifactState, DbEvent, DownloadSpec, EngineError, LiveTransferRemovalAction, ProgressUpdate,
    ProviderResumeData, RestoredDownload, TaskRuntimeUpdate, TransferChunkMapState,
    TransferControlAction, TransferControlSupport, TransferId, TransferStatus, TransferSummary,
};

const TASK_RUNTIME_UPDATE_CAPACITY: usize = 256;

/// Everything needed to pause or cancel an active task
struct TaskEntry {
    handle: JoinHandle<crate::engine::http::TaskFinalState>,
    /// Fired on soft pause
    /// Hard cancel uses handle.abort()
    pause_token: CancellationToken,
    /// Written by the task on pause, read when TaskRuntimeUpdate::Done arrives
    pause_sink: TaskPauseSink,
    /// Updated if the server suggests a better filename
    destination_sink: TaskDestinationSink,
    /// May narrow after the task starts
    control_support: TransferControlSupport,
    /// Pause finishes through TaskRuntimeUpdate::Done so the service keeps draining task updates
    pause_requested: bool,
    /// Kept for resume
    spec: DownloadSpec,
}

/// Paused download state
struct PausedTask {
    spec: DownloadSpec,
    resume_data: Option<ProviderResumeData>,
}

/// A download waiting in the queue
/// `resume_data` is set when a resumed download cannot start yet
struct QueuedTask {
    id: TransferId,
    spec: DownloadSpec,
    resume_data: Option<ProviderResumeData>,
}

#[derive(Debug, Clone)]
pub(super) enum TransferRuntimeEvent {
    TransferAdded {
        snapshot: TransferSummary,
    },
    TransferRestored {
        snapshot: TransferSummary,
    },
    Progress(ProgressUpdate),
    TransferBytesWritten {
        id: TransferId,
        bytes: u64,
    },
    DestinationChanged {
        id: TransferId,
        destination: PathBuf,
    },
    ControlSupportChanged {
        id: TransferId,
        support: TransferControlSupport,
    },
    ChunkMapChanged {
        id: TransferId,
        state: TransferChunkMapState,
    },
    TransferRemoved {
        id: TransferId,
        action: LiveTransferRemovalAction,
        artifact_state: ArtifactState,
    },
    ControlUnsupported {
        id: TransferId,
        action: TransferControlAction,
    },
}

pub(super) struct TransferRuntime {
    tasks: HashMap<TransferId, TaskEntry>,
    paused: HashMap<TransferId, PausedTask>,
    queue: VecDeque<QueuedTask>,
    max_concurrent: usize,
    next_id: u64,
    config: EngineConfig,
    /// Shared semaphores for source-wide limits
    /// HTTP uses this for per-host limits
    shared_schedulers: HashMap<SchedulerKey, Arc<Semaphore>>,
    db_tx: std::sync::mpsc::Sender<DbEvent>,
    /// Global speed cap shared across active downloads
    global_throttle: Arc<TokenBucket>,
    runtime_update_tx: mpsc::Sender<TaskRuntimeUpdate>,
    runtime_update_rx: mpsc::Receiver<TaskRuntimeUpdate>,
    events: Vec<TransferRuntimeEvent>,
}

impl TransferRuntime {
    pub(super) fn new(
        config: EngineConfig,
        db_tx: std::sync::mpsc::Sender<DbEvent>,
        initial_next_id: u64,
    ) -> Self {
        let (runtime_update_tx, runtime_update_rx) =
            mpsc::channel::<TaskRuntimeUpdate>(TASK_RUNTIME_UPDATE_CAPACITY);
        let max_concurrent = config.max_concurrent_downloads;
        let global_throttle = Arc::new(TokenBucket::new(config.global_speed_limit_bps));
        Self {
            tasks: HashMap::new(),
            paused: HashMap::new(),
            queue: VecDeque::new(),
            max_concurrent,
            next_id: initial_next_id,
            config,
            shared_schedulers: HashMap::new(),
            db_tx,
            global_throttle,
            runtime_update_tx,
            runtime_update_rx,
            events: Vec::new(),
        }
    }

    fn shared_scheduler_semaphore(&mut self, spec: &DownloadSpec) -> Option<Arc<Semaphore>> {
        let requirement = provider::capabilities(spec, &self.config).shared_scheduler?;
        Some(
            self.shared_schedulers
                .entry(requirement.key)
                .or_insert_with(|| Arc::new(Semaphore::new(requirement.limit)))
                .clone(),
        )
    }

    pub(super) async fn restore(
        &mut self,
        download: RestoredDownload,
    ) -> (Result<(), EngineError>, Vec<TransferRuntimeEvent>) {
        let result = self.handle_restore(download).await;
        (result, self.take_events())
    }

    pub(super) async fn add(
        &mut self,
        spec: DownloadSpec,
    ) -> (Result<TransferId, EngineError>, Vec<TransferRuntimeEvent>) {
        let id = TransferId(self.next_id);
        self.next_id += 1;
        let result = self.handle_add(id, spec).await;
        if result.is_err() {
            self.next_id -= 1;
        }
        (result, self.take_events())
    }

    pub(super) async fn pause(
        &mut self,
        id: TransferId,
    ) -> (Result<(), EngineError>, Vec<TransferRuntimeEvent>) {
        let result = self.handle_pause(id).await;
        (result, self.take_events())
    }

    pub(super) async fn resume(
        &mut self,
        id: TransferId,
    ) -> (Result<(), EngineError>, Vec<TransferRuntimeEvent>) {
        let result = self.handle_resume(id).await;
        (result, self.take_events())
    }

    pub(super) async fn cancel(
        &mut self,
        id: TransferId,
    ) -> (Result<(), EngineError>, Vec<TransferRuntimeEvent>) {
        let result = self.handle_cancel(id).await;
        (result, self.take_events())
    }

    pub(super) async fn delete_artifact(
        &mut self,
        id: TransferId,
    ) -> (Result<(), EngineError>, Vec<TransferRuntimeEvent>) {
        let result = self.handle_delete_artifact(id).await;
        (result, self.take_events())
    }

    pub(super) async fn update_config(
        &mut self,
        config: EngineConfig,
    ) -> (Result<(), EngineError>, Vec<TransferRuntimeEvent>) {
        let result = self.handle_update_config(config).await;
        (result, self.take_events())
    }

    pub(super) async fn next_update(&mut self) -> Option<Vec<TransferRuntimeEvent>> {
        let update = self.runtime_update_rx.recv().await?;
        self.handle_runtime_update(update).await;
        Some(self.take_events())
    }

    pub(super) async fn drain_updates(&mut self, limit: usize) -> Vec<TransferRuntimeEvent> {
        for _ in 0..limit {
            let Ok(update) = self.runtime_update_rx.try_recv() else {
                break;
            };
            self.handle_runtime_update(update).await;
        }
        self.take_events()
    }

    pub(super) fn shutdown(&mut self) {
        self.handle_shutdown();
    }

    fn take_events(&mut self) -> Vec<TransferRuntimeEvent> {
        std::mem::take(&mut self.events)
    }

    async fn spawn_task(
        &mut self,
        id: TransferId,
        spec: DownloadSpec,
        resume_data: Option<ProviderResumeData>,
    ) {
        let pause_token = CancellationToken::new();
        let SpawnedTask {
            handle,
            pause_sink,
            destination_sink,
        } = provider::spawn_task(
            id,
            &spec,
            pause_token.clone(),
            resume_data,
            ProviderRuntimeContext {
                shared_scheduler_semaphore: self.shared_scheduler_semaphore(&spec),
                global_throttle: Arc::clone(&self.global_throttle),
                runtime_update_tx: self.runtime_update_tx.clone(),
            },
        );

        let chunk_map_state = spec.active_chunk_map_state();
        self.tasks.insert(
            id,
            TaskEntry {
                handle,
                pause_token,
                pause_sink,
                destination_sink,
                control_support: spec.control_support(),
                pause_requested: false,
                spec,
            },
        );
        self.emit(TransferRuntimeEvent::ChunkMapChanged {
            id,
            state: chunk_map_state,
        })
        .await;
    }

    /// Start queued downloads until capacity is full
    async fn try_start_next(&mut self) {
        while self.tasks.len() < self.max_concurrent {
            let Some(next) = self.queue.pop_front() else {
                break;
            };
            tracing::info!(
                id = next.id.0,
                queued_remaining = self.queue.len(),
                "starting queued download"
            );
            let _ = self.db_tx.send(DbEvent::Started { id: next.id });
            self.spawn_task(next.id, next.spec, next.resume_data).await;
        }
    }

    async fn handle_add(
        &mut self,
        id: TransferId,
        spec: DownloadSpec,
    ) -> Result<TransferId, EngineError> {
        let _ = self.db_tx.send(self.added_event(id, &spec));
        self.emit(TransferRuntimeEvent::TransferAdded {
            snapshot: self.transfer_snapshot(
                id,
                &spec,
                TransferStatus::Pending,
                0,
                None,
                TransferChunkMapState::Unsupported,
            ),
        })
        .await;
        if self.tasks.len() < self.max_concurrent {
            tracing::info!(id = id.0, url = spec.url(), "download starting");
            let _ = self.db_tx.send(DbEvent::Started { id });
            self.spawn_task(id, spec, None).await;
        } else {
            tracing::info!(
                id = id.0,
                url = spec.url(),
                queued = self.queue.len() + 1,
                "download queued (at capacity)"
            );
            self.queue.push_back(QueuedTask {
                id,
                spec,
                resume_data: None,
            });
        }
        Ok(id)
    }

    /// Soft pause: fire the CancellationToken, then wait for TaskRuntimeUpdate::Done
    /// If the download is still queued, move it to paused directly
    async fn handle_pause(&mut self, id: TransferId) -> Result<(), EngineError> {
        let Some((supports_pause, _)) = self.pause_target(id) else {
            return Err(EngineError::NotFound { id });
        };
        if !supports_pause {
            self.notify_unsupported_control(id, TransferControlAction::Pause)
                .await;
            return Err(EngineError::Unsupported {
                id,
                action: TransferControlAction::Pause,
            });
        }

        if let Some(entry) = self.tasks.get_mut(&id) {
            tracing::info!(id = id.0, "pausing download");
            entry.pause_token.cancel();
            entry.pause_requested = true;
        } else if let Some(pos) = self.queue.iter().position(|t| t.id == id) {
            // Not started yet -> pull from queue and park in paused with no progress
            let task = self.queue.remove(pos).unwrap();
            tracing::info!(id = id.0, "pausing queued (unstarted) download");
            let _ = self.db_tx.send(DbEvent::Paused {
                id,
                downloaded_bytes: 0,
                resume_data: None,
            });
            self.emit(TransferRuntimeEvent::ChunkMapChanged {
                id,
                state: TransferChunkMapState::Unsupported,
            })
            .await;
            self.emit(self.status_event(id, TransferStatus::Paused, 0, None))
                .await;
            self.paused.insert(
                id,
                PausedTask {
                    spec: task.spec,
                    resume_data: task.resume_data,
                },
            );
        }
        Ok(())
    }

    async fn handle_restore(&mut self, download: RestoredDownload) -> Result<(), EngineError> {
        if !provider::supports_control_action(&download.spec, TransferControlAction::Restore) {
            self.notify_unsupported_control(download.id, TransferControlAction::Restore)
                .await;
            tracing::warn!(
                id = download.id.0,
                "provider does not support restart restore for this download"
            );
            return Err(EngineError::Unsupported {
                id: download.id,
                action: TransferControlAction::Restore,
            });
        }

        let (downloaded_bytes, total_bytes) = snapshot_totals(download.resume_data.as_ref());
        self.emit(TransferRuntimeEvent::TransferRestored {
            snapshot: self.transfer_snapshot(
                download.id,
                &download.spec,
                TransferStatus::Paused,
                downloaded_bytes,
                total_bytes,
                TransferChunkMapState::Unsupported,
            ),
        })
        .await;
        tracing::info!(
            id = download.id.0,
            "restoring paused download from database"
        );
        self.paused.insert(
            download.id,
            PausedTask {
                spec: download.spec,
                resume_data: download.resume_data,
            },
        );
        Ok(())
    }

    async fn handle_resume(&mut self, id: TransferId) -> Result<(), EngineError> {
        let Some(spec) = self.resume_target_spec(id) else {
            return Err(EngineError::NotFound { id });
        };
        if !provider::supports_control_action(spec, TransferControlAction::Resume) {
            self.notify_unsupported_control(id, TransferControlAction::Resume)
                .await;
            return Err(EngineError::Unsupported {
                id,
                action: TransferControlAction::Resume,
            });
        }

        if let Some(pt) = self.paused.remove(&id) {
            tracing::info!(id = id.0, "resuming download");
            if pt.resume_data.is_none() {
                remove_stale_part_file_for_fresh_resume(pt.spec.destination());
            }
            let (downloaded_bytes, total_bytes) = snapshot_totals(pt.resume_data.as_ref());
            if self.tasks.len() < self.max_concurrent {
                let _ = self.db_tx.send(DbEvent::Resumed { id });
                self.emit(self.status_event(
                    id,
                    TransferStatus::Downloading,
                    downloaded_bytes,
                    total_bytes,
                ))
                .await;
                self.spawn_task(id, pt.spec, pt.resume_data).await;
            } else {
                // At capacity -> put at front of queue so it starts next
                let _ = self.db_tx.send(DbEvent::Queued { id });
                self.emit(self.status_event(
                    id,
                    TransferStatus::Pending,
                    downloaded_bytes,
                    total_bytes,
                ))
                .await;
                self.queue.push_front(QueuedTask {
                    id,
                    spec: pt.spec,
                    resume_data: pt.resume_data,
                });
            }
        }
        Ok(())
    }

    async fn handle_cancel(&mut self, id: TransferId) -> Result<(), EngineError> {
        let Some((supports_cancel, destination)) = self.cancel_target(id) else {
            return Err(EngineError::NotFound { id });
        };
        if !supports_cancel {
            self.notify_unsupported_control(id, TransferControlAction::Cancel)
                .await;
            return Err(EngineError::Unsupported {
                id,
                action: TransferControlAction::Cancel,
            });
        }

        let mut removed = false;
        if let Some(entry) = self.tasks.remove(&id) {
            tracing::info!(id = id.0, "download cancelled");
            // abort() prevents TaskRuntimeUpdate::Done, so advance the queue here
            entry.handle.abort();
            self.try_start_next().await;
            removed = true;
        }
        let queued_before = self.queue.len();
        self.queue.retain(|t| t.id != id);
        removed |= self.queue.len() != queued_before;
        removed |= self.paused.remove(&id).is_some();

        if removed {
            let artifact_state = current_artifact_state(&destination);
            let _ = self.db_tx.send(DbEvent::Cancelled { id });
            let _ = self
                .db_tx
                .send(DbEvent::ArtifactStateChanged { id, artifact_state });
            self.emit(TransferRuntimeEvent::ChunkMapChanged {
                id,
                state: TransferChunkMapState::Unsupported,
            })
            .await;
            self.emit(TransferRuntimeEvent::TransferRemoved {
                id,
                action: LiveTransferRemovalAction::Cancelled,
                artifact_state,
            })
            .await;
        }
        Ok(())
    }

    async fn handle_delete_artifact(&mut self, id: TransferId) -> Result<(), EngineError> {
        let Some(resolved_destination) = self.known_destination(id) else {
            return Err(EngineError::NotFound { id });
        };
        let was_active = self.tasks.remove(&id).map(|entry| {
            entry.handle.abort();
        });
        if was_active.is_some() {
            self.try_start_next().await;
        }

        let queued_before = self.queue.len();
        self.queue.retain(|task| task.id != id);
        let removed_queued = self.queue.len() != queued_before;
        let removed_paused = self.paused.remove(&id).is_some();
        let removed_runtime_state = was_active.is_some() || removed_queued || removed_paused;

        if removed_runtime_state {
            let _ = self.db_tx.send(DbEvent::Cancelled { id });
        }
        let artifact_state = delete_artifact_files(&resolved_destination);
        let _ = self
            .db_tx
            .send(DbEvent::ArtifactStateChanged { id, artifact_state });
        self.emit(TransferRuntimeEvent::ChunkMapChanged {
            id,
            state: TransferChunkMapState::Unsupported,
        })
        .await;
        self.emit(TransferRuntimeEvent::TransferRemoved {
            id,
            action: LiveTransferRemovalAction::DeleteArtifact,
            artifact_state,
        })
        .await;
        Ok(())
    }

    fn handle_shutdown(&mut self) {
        tracing::info!(
            active = self.tasks.len(),
            paused = self.paused.len(),
            queued = self.queue.len(),
            "transfer runtime shutting down, aborting active tasks"
        );
        for (_, entry) in self.tasks.drain() {
            entry.handle.abort();
        }
        self.paused.clear();
        self.queue.clear();
    }

    async fn handle_task_done(
        &mut self,
        id: TransferId,
        status: TransferStatus,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    ) {
        let active_entry = self.tasks.remove(&id);
        if active_entry.is_none() && !self.paused.contains_key(&id) {
            return;
        }
        let mut active_entry = active_entry;
        if let Some(entry) = active_entry.as_mut() {
            let destination_sink = &entry.destination_sink;
            self.sync_runtime_destination(id, &mut entry.spec, destination_sink)
                .await;
        }

        match status {
            TransferStatus::Finished => {
                self.emit(TransferRuntimeEvent::ChunkMapChanged {
                    id,
                    state: TransferChunkMapState::Unsupported,
                })
                .await;
                let _ = self.db_tx.send(DbEvent::Finished {
                    id,
                    total_bytes: total_bytes.unwrap_or(downloaded_bytes),
                });
                self.try_start_next().await;
            }
            TransferStatus::Error => {
                self.emit(TransferRuntimeEvent::ChunkMapChanged {
                    id,
                    state: TransferChunkMapState::Unsupported,
                })
                .await;
                let _ = self.db_tx.send(DbEvent::Error { id });
                self.try_start_next().await;
            }
            TransferStatus::Paused => {
                if let Some(entry) = active_entry {
                    self.finish_active_pause(id, entry).await;
                }
                self.try_start_next().await;
            }
            TransferStatus::Pending | TransferStatus::Downloading | TransferStatus::Cancelled => {
                self.try_start_next().await;
            }
        }
    }

    async fn handle_update_config(&mut self, config: EngineConfig) -> Result<(), EngineError> {
        let old_max_concurrent = self.max_concurrent;
        let old_config = self.config.clone();

        self.max_concurrent = config.max_concurrent_downloads;
        self.global_throttle
            .set_limit(config.global_speed_limit_bps);
        self.adjust_shared_scheduler_limits(&old_config, &config);
        self.config = config;

        if self.max_concurrent > old_max_concurrent {
            self.try_start_next().await;
        }
        Ok(())
    }

    async fn handle_runtime_update(&mut self, update: TaskRuntimeUpdate) {
        match update {
            TaskRuntimeUpdate::Progress(update) => {
                if self.tasks.contains_key(&update.id)
                    || matches!(
                        update.status,
                        TransferStatus::Finished | TransferStatus::Paused | TransferStatus::Error
                    )
                {
                    self.emit(TransferRuntimeEvent::Progress(update)).await;
                }
            }
            TaskRuntimeUpdate::Done {
                id,
                status,
                downloaded_bytes,
                total_bytes,
            } => {
                self.handle_task_done(id, status, downloaded_bytes, total_bytes)
                    .await;
            }
            TaskRuntimeUpdate::TransferBytesWritten { id, bytes } => {
                if !self.tasks.contains_key(&id) {
                    return;
                }
                self.emit(TransferRuntimeEvent::TransferBytesWritten { id, bytes })
                    .await;
            }
            TaskRuntimeUpdate::DestinationChanged { id, destination } => {
                let Some(entry) = self.tasks.get_mut(&id) else {
                    return;
                };
                if destination == entry.spec.destination() {
                    return;
                }
                entry.spec.update_destination(destination.clone());
                let _ = self.db_tx.send(DbEvent::DestinationChanged {
                    id,
                    destination: destination.clone(),
                });
                self.emit(TransferRuntimeEvent::DestinationChanged { id, destination })
                    .await;
            }
            TaskRuntimeUpdate::ControlSupportChanged { id, support } => {
                let Some(entry) = self.tasks.get_mut(&id) else {
                    return;
                };
                if entry.control_support == support {
                    return;
                }
                entry.control_support = support;
                self.emit(TransferRuntimeEvent::ControlSupportChanged { id, support })
                    .await;
            }
            TaskRuntimeUpdate::ChunkMapChanged { id, state } => {
                if !self.tasks.contains_key(&id) {
                    return;
                }
                self.emit(TransferRuntimeEvent::ChunkMapChanged { id, state })
                    .await;
            }
        }
    }

    fn adjust_shared_scheduler_limits(
        &mut self,
        old_config: &EngineConfig,
        new_config: &EngineConfig,
    ) {
        for (key, semaphore) in &self.shared_schedulers {
            let Some(old_limit) = provider::shared_scheduler_limit(key, old_config) else {
                continue;
            };
            let Some(new_limit) = provider::shared_scheduler_limit(key, new_config) else {
                continue;
            };
            if old_limit == new_limit {
                continue;
            }
            let available = semaphore.available_permits();
            let in_use = old_limit.saturating_sub(available);
            let desired_available = new_limit.saturating_sub(in_use);

            if desired_available > available {
                semaphore.add_permits(desired_available - available);
            } else if desired_available < available {
                let _ = semaphore.forget_permits(available - desired_available);
            }
        }
    }

    fn added_event(&self, id: TransferId, spec: &DownloadSpec) -> DbEvent {
        DbEvent::Added {
            id,
            source: provider::persisted_source(spec),
            destination: spec.destination().to_path_buf(),
        }
    }

    fn status_event(
        &self,
        id: TransferId,
        status: TransferStatus,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    ) -> TransferRuntimeEvent {
        TransferRuntimeEvent::Progress(ProgressUpdate {
            id,
            status,
            downloaded_bytes,
            total_bytes,
            speed_bytes_per_sec: 0,
        })
    }

    fn transfer_snapshot(
        &self,
        id: TransferId,
        spec: &DownloadSpec,
        status: TransferStatus,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        chunk_map_state: TransferChunkMapState,
    ) -> TransferSummary {
        TransferSummary {
            id,
            provider_kind: spec.provider_kind().to_string(),
            source_label: spec.source_label().to_string(),
            destination: spec.destination().to_path_buf(),
            status,
            downloaded_bytes,
            total_bytes,
            speed_bytes_per_sec: 0,
            control_support: spec.control_support(),
            chunk_map_state,
        }
    }

    async fn finish_active_pause(&mut self, id: TransferId, entry: TaskEntry) {
        if !entry.pause_requested {
            tracing::warn!(id = id.0, "download paused without a service pause request");
        }
        let resume_data = provider::take_resume_data(entry.pause_sink);
        if let Some(resume_data) = resume_data {
            let downloaded_bytes = resume_data.downloaded_bytes();
            let total_bytes = resume_data.total_bytes();
            let _ = self.db_tx.send(DbEvent::Paused {
                id,
                downloaded_bytes,
                resume_data: Some(resume_data.clone()),
            });
            self.emit(TransferRuntimeEvent::ChunkMapChanged {
                id,
                state: TransferChunkMapState::Unsupported,
            })
            .await;
            self.emit(self.status_event(id, TransferStatus::Paused, downloaded_bytes, total_bytes))
                .await;
            self.paused.insert(
                id,
                PausedTask {
                    spec: entry.spec,
                    resume_data: Some(resume_data),
                },
            );
        } else {
            tracing::warn!(id = id.0, "download reported paused without resume data");
            let _ = self.db_tx.send(DbEvent::Error { id });
            self.emit(TransferRuntimeEvent::ChunkMapChanged {
                id,
                state: TransferChunkMapState::Unsupported,
            })
            .await;
            self.emit(self.status_event(id, TransferStatus::Error, 0, None))
                .await;
        }
    }

    async fn notify_unsupported_control(&mut self, id: TransferId, action: TransferControlAction) {
        self.emit(TransferRuntimeEvent::ControlUnsupported { id, action })
            .await;
    }

    async fn emit(&mut self, event: TransferRuntimeEvent) {
        self.events.push(event);
    }

    fn pause_target(&self, id: TransferId) -> Option<(bool, &DownloadSpec)> {
        if let Some(entry) = self.tasks.get(&id) {
            return Some((entry.control_support.can_pause, &entry.spec));
        }
        self.queue.iter().find(|task| task.id == id).map(|task| {
            (
                provider::supports_control_action(&task.spec, TransferControlAction::Pause),
                &task.spec,
            )
        })
    }

    fn resume_target_spec(&self, id: TransferId) -> Option<&DownloadSpec> {
        self.paused.get(&id).map(|task| &task.spec)
    }

    fn cancel_target(&self, id: TransferId) -> Option<(bool, PathBuf)> {
        if let Some(entry) = self.tasks.get(&id) {
            return Some((
                entry.control_support.can_cancel,
                self.runtime_destination(entry),
            ));
        }
        if let Some(task) = self.paused.get(&id) {
            return Some((
                provider::supports_control_action(&task.spec, TransferControlAction::Cancel),
                task.spec.destination().to_path_buf(),
            ));
        }
        self.queue.iter().find(|task| task.id == id).map(|task| {
            (
                provider::supports_control_action(&task.spec, TransferControlAction::Cancel),
                task.spec.destination().to_path_buf(),
            )
        })
    }

    fn known_destination(&self, id: TransferId) -> Option<PathBuf> {
        self.tasks
            .get(&id)
            .map(|entry| self.runtime_destination(entry))
            .or_else(|| {
                self.paused
                    .get(&id)
                    .map(|task| task.spec.destination().to_path_buf())
            })
            .or_else(|| {
                self.queue
                    .iter()
                    .find(|task| task.id == id)
                    .map(|task| task.spec.destination().to_path_buf())
            })
    }

    fn runtime_destination(&self, entry: &TaskEntry) -> PathBuf {
        provider::current_destination(&entry.destination_sink)
            .unwrap_or_else(|| entry.spec.destination().to_path_buf())
    }

    async fn sync_runtime_destination(
        &mut self,
        id: TransferId,
        spec: &mut DownloadSpec,
        destination_sink: &TaskDestinationSink,
    ) {
        let Some(destination) = provider::current_destination(destination_sink) else {
            return;
        };
        if destination == spec.destination() {
            return;
        }
        spec.update_destination(destination.clone());
        let _ = self.db_tx.send(DbEvent::DestinationChanged {
            id,
            destination: destination.clone(),
        });
        self.emit(TransferRuntimeEvent::DestinationChanged { id, destination })
            .await;
    }
}

fn snapshot_totals(resume_data: Option<&ProviderResumeData>) -> (u64, Option<u64>) {
    match resume_data {
        Some(data) => (data.downloaded_bytes(), data.total_bytes()),
        None => (0, None),
    }
}

fn remove_stale_part_file_for_fresh_resume(destination: &Path) {
    let part_path = part_path_for(destination);
    match std::fs::remove_file(&part_path) {
        Ok(()) => {
            tracing::info!(
                path = %part_path.display(),
                "removed stale part file before restarting restored download"
            );
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(
                ?error,
                path = %part_path.display(),
                "failed to remove stale part file before restarting restored download"
            );
        }
    }
}

fn artifact_paths(destination: &Path) -> [PathBuf; 2] {
    [destination.to_path_buf(), part_path_for(destination)]
}

fn current_artifact_state(destination: &Path) -> ArtifactState {
    if artifact_paths(destination).iter().any(|path| path.exists()) {
        ArtifactState::Present
    } else {
        ArtifactState::Missing
    }
}

pub(crate) fn delete_artifact_files(destination: &Path) -> ArtifactState {
    let mut removed_any = false;
    for path in artifact_paths(destination) {
        match std::fs::remove_file(&path) {
            Ok(()) => removed_any = true,
            Err(error) => match error.kind() {
                ErrorKind::NotFound => {}
                _ => tracing::warn!(path = %path.display(), "failed to delete artifact: {error}"),
            },
        }
    }

    if artifact_paths(destination).iter().any(|path| path.exists()) {
        ArtifactState::Present
    } else if removed_any {
        ArtifactState::Deleted
    } else {
        ArtifactState::Missing
    }
}
