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

//! Download engine actor
//!
//! Sits between frontends and download tasks
//! Commands come in through a bounded channel
//! Frontends read one ordered event stream from the actor
//!
//! Queue flow:
//!   Add    → if tasks < max_concurrent, spawn immediately; else push to queue
//!   Done   → done_rx receives the task's final state; remove from tasks,
//!            save final state, advance queue
//!   Pause  → cancel the task's CancellationToken and await the handle
//!            If it returns paused with resume data, store it in `paused`
//!            If it is still queued, move it directly to `paused`
//!   Resume → if at capacity, push to front of queue; else spawn immediately
//!   Cancel → abort the handle (prevents done_tx from firing), drain from queue,
//!            then advance queue manually since done_rx won't fire

use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::runtime::Handle;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::CoreConfig;
use crate::engine::destination::part_path_for;
use crate::engine::http::TokenBucket;
use crate::engine::provider::{
    self, ProviderRuntimeContext, SchedulerKey, SpawnedTask, TaskDestinationSink, TaskDone,
    TaskPauseSink,
};
use crate::engine::{
    ArtifactState, DbEvent, DownloadControlAction, DownloadId, DownloadSpec, DownloadStatus,
    EngineError, EngineEvent, LiveTransferRemovalAction, ProgressUpdate, ProviderResumeData,
    RestoredDownload, TaskRuntimeUpdate, TransferChunkMapState, TransferControlSupport,
};

const ENGINE_COMMAND_CAPACITY: usize = 64;
const ENGINE_EVENT_CAPACITY: usize = 512;
const TASK_DONE_CAPACITY: usize = 64;
const TASK_RUNTIME_UPDATE_CAPACITY: usize = 256;

#[allow(dead_code)]
enum EngineCommand {
    Add {
        id: DownloadId,
        spec: DownloadSpec,
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
        destination: PathBuf,
    },
    Restore {
        download: RestoredDownload,
    },
    UpdateConfig {
        config: CoreConfig,
    },
    Shutdown,
}

// --- per-task bookkeeping ------------------------------------------------

/// Everything needed to pause or cancel an active task
struct TaskEntry {
    handle: JoinHandle<crate::engine::http::TaskFinalState>,
    /// Fired on soft pause
    /// Hard cancel uses handle.abort()
    pause_token: CancellationToken,
    /// Written by the task on pause, read by the engine after awaiting the handle
    pause_sink: TaskPauseSink,
    /// Updated if the server suggests a better filename
    destination_sink: TaskDestinationSink,
    /// May narrow after the task starts
    control_support: TransferControlSupport,
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
    id: DownloadId,
    spec: DownloadSpec,
    resume_data: Option<ProviderResumeData>,
}

// --- public engine handle ------------------------------------------------

pub struct DownloadEngine {
    actor: Option<JoinHandle<()>>,
    cmd_tx: mpsc::Sender<EngineCommand>,
    event_rx: mpsc::Receiver<EngineEvent>,
    next_id: u64,
}

impl DownloadEngine {
    pub fn spawn_on(
        runtime: &Handle,
        config: CoreConfig,
        db_tx: std::sync::mpsc::Sender<DbEvent>,
        initial_next_id: u64,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(ENGINE_COMMAND_CAPACITY);
        let (event_tx, event_rx) = mpsc::channel(ENGINE_EVENT_CAPACITY);
        let (done_tx, done_rx) = mpsc::channel::<TaskDone>(TASK_DONE_CAPACITY);
        let (runtime_update_tx, runtime_update_rx) =
            mpsc::channel::<TaskRuntimeUpdate>(TASK_RUNTIME_UPDATE_CAPACITY);

        let actor = runtime.spawn(
            EngineActor::new(event_tx, config, db_tx, done_tx, runtime_update_tx).run(
                cmd_rx,
                done_rx,
                runtime_update_rx,
            ),
        );

        Self {
            actor: Some(actor),
            cmd_tx,
            event_rx,
            next_id: initial_next_id,
        }
    }

    /// Add a saved download to the paused map
    /// The user must resume it explicitly
    pub async fn restore(&self, download: RestoredDownload) -> Result<(), EngineError> {
        self.send_command(EngineCommand::Restore { download }).await
    }

    pub async fn add(&mut self, spec: DownloadSpec) -> Result<DownloadId, EngineError> {
        let id = DownloadId(self.next_id);
        self.next_id += 1;
        if let Err(error) = self.send_command(EngineCommand::Add { id, spec }).await {
            self.next_id -= 1;
            return Err(error);
        }
        Ok(id)
    }

    pub async fn pause(&self, id: DownloadId) -> Result<(), EngineError> {
        self.send_command(EngineCommand::Pause { id }).await
    }

    pub async fn resume(&self, id: DownloadId) -> Result<(), EngineError> {
        self.send_command(EngineCommand::Resume { id }).await
    }

    #[allow(dead_code)] // kept as an explicit backend control even though the current UI deletes artifacts instead.
    pub async fn cancel(&self, id: DownloadId) -> Result<(), EngineError> {
        self.send_command(EngineCommand::Cancel { id }).await
    }

    pub async fn delete_artifact(
        &self,
        id: DownloadId,
        destination: PathBuf,
    ) -> Result<(), EngineError> {
        self.send_command(EngineCommand::DeleteArtifact { id, destination })
            .await
    }

    pub async fn update_config(&self, config: CoreConfig) -> Result<(), EngineError> {
        self.send_command(EngineCommand::UpdateConfig { config })
            .await
    }

    pub async fn next_event(&mut self) -> Option<EngineEvent> {
        self.event_rx.recv().await
    }

    pub async fn shutdown(mut self) -> Result<(), EngineError> {
        let send_result = self.send_command(EngineCommand::Shutdown).await;
        if let Some(actor) = self.actor.take() {
            let _ = actor.await;
        }
        send_result
    }

    async fn send_command(&self, command: EngineCommand) -> Result<(), EngineError> {
        self.cmd_tx
            .send(command)
            .await
            .map_err(|_| EngineError::Closed)
    }
}

impl Drop for DownloadEngine {
    fn drop(&mut self) {
        let _ = self.cmd_tx.try_send(EngineCommand::Shutdown);
    }
}

// --- actor ---------------------------------------------------------------

/// Owns engine state and handles commands on the tokio runtime
/// New engine-wide state goes here as fields
/// New source kinds add new spec variants and spawn paths
struct EngineActor {
    tasks: HashMap<DownloadId, TaskEntry>,
    paused: HashMap<DownloadId, PausedTask>,
    queue: VecDeque<QueuedTask>,
    max_concurrent: usize,
    done_tx: mpsc::Sender<TaskDone>,
    event_tx: mpsc::Sender<EngineEvent>,
    config: CoreConfig,
    /// Shared semaphores for source-wide limits
    /// HTTP uses this for per-host limits
    shared_schedulers: HashMap<SchedulerKey, Arc<Semaphore>>,
    db_tx: std::sync::mpsc::Sender<DbEvent>,
    /// Global speed cap shared across active downloads
    global_throttle: Arc<TokenBucket>,
    runtime_update_tx: mpsc::Sender<TaskRuntimeUpdate>,
}

impl EngineActor {
    fn new(
        event_tx: mpsc::Sender<EngineEvent>,
        config: CoreConfig,
        db_tx: std::sync::mpsc::Sender<DbEvent>,
        done_tx: mpsc::Sender<TaskDone>,
        runtime_update_tx: mpsc::Sender<TaskRuntimeUpdate>,
    ) -> Self {
        let max_concurrent = config.max_concurrent_downloads;
        let global_throttle = Arc::new(TokenBucket::new(config.global_speed_limit_bps));
        Self {
            tasks: HashMap::new(),
            paused: HashMap::new(),
            queue: VecDeque::new(),
            max_concurrent,
            done_tx,
            event_tx,
            config,
            shared_schedulers: HashMap::new(),
            db_tx,
            global_throttle,
            runtime_update_tx,
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

    async fn run(
        mut self,
        mut cmd_rx: mpsc::Receiver<EngineCommand>,
        mut done_rx: mpsc::Receiver<TaskDone>,
        mut runtime_update_rx: mpsc::Receiver<TaskRuntimeUpdate>,
    ) {
        loop {
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else {
                        self.handle_shutdown();
                        break;
                    };
                    match cmd {
                        EngineCommand::Add { id, spec } =>
                            self.handle_add(id, spec).await,
                        EngineCommand::Pause { id } =>
                            self.handle_pause(id).await,
                        EngineCommand::Resume { id } =>
                            self.handle_resume(id).await,
                        EngineCommand::Cancel { id } =>
                            self.handle_cancel(id).await,
                        EngineCommand::DeleteArtifact { id, destination } =>
                            self.handle_delete_artifact(id, &destination).await,
                        EngineCommand::Restore { download } => {
                            if !provider::supports_control_action(
                                &download.spec,
                                DownloadControlAction::Restore,
                            ) {
                                self.notify_unsupported_control(
                                    download.id,
                                    DownloadControlAction::Restore,
                                ).await;
                                tracing::warn!(
                                    id = download.id.0,
                                    "provider does not support restart restore for this download"
                                );
                                continue;
                            }
                            tracing::info!(id = download.id.0, "restoring paused download from database");
                            self.paused.insert(download.id, PausedTask {
                                spec: download.spec,
                                resume_data: download.resume_data,
                            });
                        }
                        EngineCommand::UpdateConfig { config } => {
                            self.handle_update_config(config).await;
                        }
                        EngineCommand::Shutdown => {
                            self.handle_shutdown();
                            break;
                        }
                    }
                }
                Some(done) = done_rx.recv() => {
                    self.handle_task_done(done).await;
                }
                Some(update) = runtime_update_rx.recv() => {
                    self.handle_runtime_update(update).await;
                }
            }
        }
    }

    async fn spawn_task(
        &mut self,
        id: DownloadId,
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
            self.done_tx.clone(),
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
                spec,
            },
        );
        self.emit(EngineEvent::ChunkMapChanged {
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

    async fn handle_add(&mut self, id: DownloadId, spec: DownloadSpec) {
        let _ = self.db_tx.send(self.added_event(id, &spec));
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
    }

    /// Soft pause: fire the CancellationToken, wait for the task to drain, then
    /// read the resume state it left in the pause sink
    /// If the download is still queued, move it to paused directly
    async fn handle_pause(&mut self, id: DownloadId) {
        let Some(spec) = self.pause_target_spec(id) else {
            return;
        };
        if !provider::supports_control_action(spec, DownloadControlAction::Pause) {
            self.notify_unsupported_control(id, DownloadControlAction::Pause)
                .await;
            return;
        }

        if let Some(entry) = self.tasks.remove(&id) {
            tracing::info!(id = id.0, "pausing download");
            entry.pause_token.cancel();
            // Range workers flush buffered bytes and report saved ranges for
            // resume. Single-stream fallback just stops because it cannot save
            // resume data
            let final_state = match entry.handle.await {
                Ok(final_state) => final_state,
                Err(error) => {
                    tracing::warn!(id = id.0, ?error, "download task failed while pausing");
                    let _ = self.db_tx.send(DbEvent::Error { id });
                    self.emit(EngineEvent::ChunkMapChanged {
                        id,
                        state: TransferChunkMapState::Unsupported,
                    })
                    .await;
                    self.try_start_next().await;
                    return;
                }
            };
            let mut spec = entry.spec;
            self.sync_runtime_destination(id, &mut spec, &entry.destination_sink)
                .await;
            let resume_data = provider::take_resume_data(entry.pause_sink);

            match final_state.status {
                DownloadStatus::Paused => {
                    if let Some(resume_data) = resume_data {
                        let downloaded_bytes = resume_data.downloaded_bytes();
                        let _ = self.db_tx.send(DbEvent::Paused {
                            id,
                            downloaded_bytes,
                            resume_data: Some(resume_data.clone()),
                        });
                        self.emit(EngineEvent::ChunkMapChanged {
                            id,
                            state: TransferChunkMapState::Unsupported,
                        })
                        .await;
                        self.paused.insert(
                            id,
                            PausedTask {
                                spec,
                                resume_data: Some(resume_data),
                            },
                        );
                    } else {
                        tracing::warn!(id = id.0, "download reported paused without resume data");
                        let _ = self.db_tx.send(DbEvent::Error { id });
                        self.emit(EngineEvent::ChunkMapChanged {
                            id,
                            state: TransferChunkMapState::Unsupported,
                        })
                        .await;
                    }
                    self.try_start_next().await;
                }
                DownloadStatus::Finished => {
                    self.emit(EngineEvent::ChunkMapChanged {
                        id,
                        state: TransferChunkMapState::Unsupported,
                    })
                    .await;
                    let _ = self.db_tx.send(DbEvent::Finished {
                        id,
                        total_bytes: final_state
                            .total_bytes
                            .unwrap_or(final_state.downloaded_bytes),
                    });
                    self.try_start_next().await;
                }
                DownloadStatus::Error => {
                    self.emit(EngineEvent::ChunkMapChanged {
                        id,
                        state: TransferChunkMapState::Unsupported,
                    })
                    .await;
                    let _ = self.db_tx.send(DbEvent::Error { id });
                    self.try_start_next().await;
                }
                DownloadStatus::Pending
                | DownloadStatus::Downloading
                | DownloadStatus::Cancelled => {
                    self.try_start_next().await;
                }
            }
        } else if let Some(pos) = self.queue.iter().position(|t| t.id == id) {
            // Not started yet -> pull from queue and park in paused with no progress
            let task = self.queue.remove(pos).unwrap();
            tracing::info!(id = id.0, "pausing queued (unstarted) download");
            let _ = self.db_tx.send(DbEvent::Paused {
                id,
                downloaded_bytes: 0,
                resume_data: None,
            });
            self.emit(EngineEvent::ChunkMapChanged {
                id,
                state: TransferChunkMapState::Unsupported,
            })
            .await;
            self.emit(self.status_event(id, DownloadStatus::Paused, 0, None))
                .await;
            self.paused.insert(
                id,
                PausedTask {
                    spec: task.spec,
                    resume_data: task.resume_data,
                },
            );
        }
    }

    async fn handle_resume(&mut self, id: DownloadId) {
        let Some(spec) = self.resume_target_spec(id) else {
            return;
        };
        if !provider::supports_control_action(spec, DownloadControlAction::Resume) {
            self.notify_unsupported_control(id, DownloadControlAction::Resume)
                .await;
            return;
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
                    DownloadStatus::Downloading,
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
                    DownloadStatus::Pending,
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
    }

    async fn handle_cancel(&mut self, id: DownloadId) {
        let Some((supports_cancel, destination)) = self.cancel_target(id) else {
            return;
        };
        if !supports_cancel {
            self.notify_unsupported_control(id, DownloadControlAction::Cancel)
                .await;
            return;
        }

        let mut removed = false;
        if let Some(entry) = self.tasks.remove(&id) {
            tracing::info!(id = id.0, "download cancelled");
            // abort() prevents done_tx from firing, so advance the queue here
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
            self.emit(EngineEvent::ChunkMapChanged {
                id,
                state: TransferChunkMapState::Unsupported,
            })
            .await;
            self.emit(EngineEvent::LiveTransferRemoved {
                id,
                action: LiveTransferRemovalAction::Cancelled,
                artifact_state,
            })
            .await;
        }
    }

    async fn handle_delete_artifact(&mut self, id: DownloadId, destination: &Path) {
        let resolved_destination = self
            .known_destination(id)
            .unwrap_or_else(|| destination.to_path_buf());
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
        self.emit(EngineEvent::ChunkMapChanged {
            id,
            state: TransferChunkMapState::Unsupported,
        })
        .await;
        self.emit(EngineEvent::LiveTransferRemoved {
            id,
            action: LiveTransferRemovalAction::DeleteArtifact,
            artifact_state,
        })
        .await;
    }

    fn handle_shutdown(&mut self) {
        tracing::info!(
            active = self.tasks.len(),
            paused = self.paused.len(),
            queued = self.queue.len(),
            "engine shutting down, aborting active tasks"
        );
        for (_, entry) in self.tasks.drain() {
            entry.handle.abort();
        }
        self.paused.clear();
        self.queue.clear();
    }

    async fn handle_task_done(&mut self, done: TaskDone) {
        let active_entry = self.tasks.remove(&done.id);
        if active_entry.is_none() && !self.paused.contains_key(&done.id) {
            return;
        }
        if let Some(mut entry) = active_entry {
            self.sync_runtime_destination(done.id, &mut entry.spec, &entry.destination_sink)
                .await;
        }

        match done.final_state.status {
            DownloadStatus::Finished => {
                self.emit(EngineEvent::ChunkMapChanged {
                    id: done.id,
                    state: TransferChunkMapState::Unsupported,
                })
                .await;
                let _ = self.db_tx.send(DbEvent::Finished {
                    id: done.id,
                    total_bytes: done
                        .final_state
                        .total_bytes
                        .unwrap_or(done.final_state.downloaded_bytes),
                });
                self.try_start_next().await;
            }
            DownloadStatus::Error => {
                self.emit(EngineEvent::ChunkMapChanged {
                    id: done.id,
                    state: TransferChunkMapState::Unsupported,
                })
                .await;
                let _ = self.db_tx.send(DbEvent::Error { id: done.id });
                self.try_start_next().await;
            }
            DownloadStatus::Paused => {
                if !self.paused.contains_key(&done.id) {
                    self.try_start_next().await;
                }
            }
            DownloadStatus::Pending | DownloadStatus::Downloading | DownloadStatus::Cancelled => {
                self.try_start_next().await;
            }
        }
    }

    async fn handle_update_config(&mut self, config: CoreConfig) {
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
    }

    async fn handle_runtime_update(&mut self, update: TaskRuntimeUpdate) {
        match update {
            TaskRuntimeUpdate::Progress(update) => {
                if self.tasks.contains_key(&update.id)
                    || matches!(
                        update.status,
                        DownloadStatus::Finished | DownloadStatus::Paused | DownloadStatus::Error
                    )
                {
                    self.emit(EngineEvent::Progress(update)).await;
                }
            }
            TaskRuntimeUpdate::DownloadBytesWritten { id, bytes } => {
                if !self.tasks.contains_key(&id) {
                    return;
                }
                self.emit(EngineEvent::DownloadBytesWritten { id, bytes })
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
                self.emit(EngineEvent::DestinationChanged { id, destination })
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
                self.emit(EngineEvent::ControlSupportChanged { id, support })
                    .await;
            }
            TaskRuntimeUpdate::ChunkMapChanged { id, state } => {
                if !self.tasks.contains_key(&id) {
                    return;
                }
                self.emit(EngineEvent::ChunkMapChanged { id, state }).await;
            }
        }
    }

    fn adjust_shared_scheduler_limits(&mut self, old_config: &CoreConfig, new_config: &CoreConfig) {
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

    fn added_event(&self, id: DownloadId, spec: &DownloadSpec) -> DbEvent {
        DbEvent::Added {
            id,
            source: provider::persisted_source(spec),
            destination: spec.destination().to_path_buf(),
        }
    }

    fn status_event(
        &self,
        id: DownloadId,
        status: DownloadStatus,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    ) -> EngineEvent {
        EngineEvent::Progress(ProgressUpdate {
            id,
            status,
            downloaded_bytes,
            total_bytes,
            speed_bytes_per_sec: 0,
        })
    }

    async fn notify_unsupported_control(&self, id: DownloadId, action: DownloadControlAction) {
        self.emit(EngineEvent::ControlUnsupported { id, action })
            .await;
    }

    async fn emit(&self, event: EngineEvent) {
        let _ = self.event_tx.send(event).await;
    }

    fn pause_target_spec(&self, id: DownloadId) -> Option<&DownloadSpec> {
        self.tasks
            .get(&id)
            .and_then(|entry| {
                entry
                    .control_support
                    .supports(DownloadControlAction::Pause)
                    .then_some(&entry.spec)
            })
            .or_else(|| {
                self.queue
                    .iter()
                    .find(|task| task.id == id)
                    .map(|task| &task.spec)
            })
    }

    fn resume_target_spec(&self, id: DownloadId) -> Option<&DownloadSpec> {
        self.paused.get(&id).map(|task| &task.spec)
    }

    fn cancel_target(&self, id: DownloadId) -> Option<(bool, PathBuf)> {
        if let Some(entry) = self.tasks.get(&id) {
            return Some((
                entry.control_support.can_cancel,
                self.runtime_destination(entry),
            ));
        }
        if let Some(task) = self.paused.get(&id) {
            return Some((
                provider::supports_control_action(&task.spec, DownloadControlAction::Cancel),
                task.spec.destination().to_path_buf(),
            ));
        }
        self.queue.iter().find(|task| task.id == id).map(|task| {
            (
                provider::supports_control_action(&task.spec, DownloadControlAction::Cancel),
                task.spec.destination().to_path_buf(),
            )
        })
    }

    fn known_destination(&self, id: DownloadId) -> Option<PathBuf> {
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
        &self,
        id: DownloadId,
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
        self.emit(EngineEvent::DestinationChanged { id, destination })
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

fn delete_artifact_files(destination: &Path) -> ArtifactState {
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
