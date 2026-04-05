/***************************************************
** This file is part of Ophelia, distributed under the
** terms of the GPL License, version 3 or later.
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs, do no evil and behave plz )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

//! The download engine actor.
//!
//! Owns the tokio runtime and sits between the UI thread and download tasks.
//! Commands arrive over a cmd channel; progress updates flow back the other way.
//!
//! Queue lifecycle:
//!   Add    → if tasks < max_concurrent, spawn immediately; else push to queue.
//!   Done   → done_rx fires when a task returns naturally (finish or error);
//!            remove from tasks, persist terminal state, advance queue.
//!   Pause  → cancel the task's CancellationToken, await the handle, read chunk
//!            offsets from the pause_sink, store in `paused` map. If the id is
//!            in the queue (not yet started), move it directly to `paused`.
//!   Resume → if at capacity, push to front of queue; else spawn immediately.
//!   Cancel → abort the handle (prevents done_tx from firing), drain from queue,
//!            then advance queue manually since done_rx won't fire.

use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::runtime::Runtime;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::engine::http::TokenBucket;
use crate::engine::provider::{
    self, ProviderRuntimeContext, SchedulerKey, SpawnedTask, TaskDestinationSink, TaskDone,
    TaskPauseSink,
};
use crate::engine::{
    ArtifactState, DbEvent, DownloadControlAction, DownloadId, DownloadSpec, DownloadStatus,
    EngineNotification, LiveTransferRemovalAction, ProgressUpdate, ProviderResumeData,
    RestoredDownload,
};
use crate::settings::Settings;

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
    UpdateSettings {
        settings: Settings,
    },
    Shutdown,
}

// --- per-task bookkeeping ------------------------------------------------

/// Everything the engine needs to pause or cancel an active task.
struct TaskEntry {
    handle: JoinHandle<()>,
    /// Fired on soft pause. Hard cancel uses handle.abort() instead.
    pause_token: CancellationToken,
    /// Written by the task on pause; read by the engine after awaiting the handle.
    pause_sink: TaskPauseSink,
    /// Updated by the task if a probe refines the destination/filename at runtime.
    destination_sink: TaskDestinationSink,
    /// Kept for re-spawning on resume.
    spec: DownloadSpec,
}

/// State stored when a download is paused, used to re-spawn on resume.
struct PausedTask {
    spec: DownloadSpec,
    resume_data: Option<ProviderResumeData>,
}

/// A download waiting in the queue. `resume_data` is populated when the user
/// resumed a paused download that could not start immediately.
struct QueuedTask {
    id: DownloadId,
    spec: DownloadSpec,
    resume_data: Option<ProviderResumeData>,
}

// --- public engine handle ------------------------------------------------

pub struct DownloadEngine {
    #[allow(dead_code)] // must be held to keep the tokio runtime alive
    runtime: Runtime,
    cmd_tx: mpsc::UnboundedSender<EngineCommand>,
    progress_rx: mpsc::UnboundedReceiver<ProgressUpdate>,
    notification_rx: mpsc::UnboundedReceiver<EngineNotification>,
    next_id: u64,
}

impl DownloadEngine {
    pub fn new(
        settings: Settings,
        db_tx: std::sync::mpsc::Sender<DbEvent>,
        initial_next_id: u64,
    ) -> Self {
        let runtime = Runtime::new().expect("failed to create tokio runtime");
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();
        let (notification_tx, notification_rx) = mpsc::unbounded_channel();
        let (done_tx, done_rx) = mpsc::unbounded_channel::<TaskDone>();

        runtime.spawn(
            EngineActor::new(progress_tx, notification_tx, settings, db_tx, done_tx)
                .run(cmd_rx, done_rx),
        );

        Self {
            runtime,
            cmd_tx,
            progress_rx,
            notification_rx,
            next_id: initial_next_id,
        }
    }

    /// Pre-populate the paused map with a download restored from SQLite.
    /// Does not start a task, user must resume explicitly.
    pub fn restore(&self, download: RestoredDownload) {
        let _ = self.cmd_tx.send(EngineCommand::Restore { download });
    }

    pub fn add(&mut self, spec: DownloadSpec) -> DownloadId {
        let id = DownloadId(self.next_id);
        self.next_id += 1;
        let _ = self.cmd_tx.send(EngineCommand::Add { id, spec });
        id
    }

    pub fn pause(&self, id: DownloadId) {
        let _ = self.cmd_tx.send(EngineCommand::Pause { id });
    }

    pub fn resume(&self, id: DownloadId) {
        let _ = self.cmd_tx.send(EngineCommand::Resume { id });
    }

    #[allow(dead_code)] // kept as an explicit backend control even though the current UI deletes artifacts instead.
    pub fn cancel(&self, id: DownloadId) {
        let _ = self.cmd_tx.send(EngineCommand::Cancel { id });
    }

    pub fn delete_artifact(&self, id: DownloadId, destination: PathBuf) {
        let _ = self
            .cmd_tx
            .send(EngineCommand::DeleteArtifact { id, destination });
    }

    pub fn update_settings(&self, settings: Settings) {
        let _ = self.cmd_tx.send(EngineCommand::UpdateSettings { settings });
    }

    pub fn poll_progress(&mut self) -> Option<ProgressUpdate> {
        self.progress_rx.try_recv().ok()
    }

    pub fn poll_notification(&mut self) -> Option<EngineNotification> {
        self.notification_rx.try_recv().ok()
    }
}

// --- actor ---------------------------------------------------------------

/// Owns all mutable engine state and handles commands on the tokio runtime.
/// New state (speed limits, scheduler) goes here as fields.
/// New providers add new spec variants and spawn paths; the dispatch loop stays flat.
struct EngineActor {
    tasks: HashMap<DownloadId, TaskEntry>,
    paused: HashMap<DownloadId, PausedTask>,
    queue: VecDeque<QueuedTask>,
    max_concurrent: usize,
    done_tx: mpsc::UnboundedSender<TaskDone>,
    progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
    notification_tx: mpsc::UnboundedSender<EngineNotification>,
    settings: Settings,
    /// Shared scheduler semaphores keyed by provider-defined scheduling keys.
    /// HTTP currently uses per-hostname limits, but that detail now lives in
    /// `provider.rs` rather than in the engine actor itself.
    shared_schedulers: HashMap<SchedulerKey, Arc<Semaphore>>,
    db_tx: std::sync::mpsc::Sender<DbEvent>,
    /// Global bandwidth cap shared across all active download tasks.
    global_throttle: Arc<TokenBucket>,
}

impl EngineActor {
    fn new(
        progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
        notification_tx: mpsc::UnboundedSender<EngineNotification>,
        settings: Settings,
        db_tx: std::sync::mpsc::Sender<DbEvent>,
        done_tx: mpsc::UnboundedSender<TaskDone>,
    ) -> Self {
        let max_concurrent = settings.max_concurrent_downloads;
        let global_throttle = Arc::new(TokenBucket::new(settings.global_speed_limit_bps));
        Self {
            tasks: HashMap::new(),
            paused: HashMap::new(),
            queue: VecDeque::new(),
            max_concurrent,
            done_tx,
            progress_tx,
            notification_tx,
            settings,
            shared_schedulers: HashMap::new(),
            db_tx,
            global_throttle,
        }
    }

    fn shared_scheduler_semaphore(&mut self, spec: &DownloadSpec) -> Option<Arc<Semaphore>> {
        let requirement = provider::capabilities(spec, &self.settings).shared_scheduler?;
        Some(
            self.shared_schedulers
                .entry(requirement.key)
                .or_insert_with(|| Arc::new(Semaphore::new(requirement.limit)))
                .clone(),
        )
    }

    async fn run(
        mut self,
        mut cmd_rx: mpsc::UnboundedReceiver<EngineCommand>,
        mut done_rx: mpsc::UnboundedReceiver<TaskDone>,
    ) {
        loop {
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        EngineCommand::Add { id, spec } =>
                            self.handle_add(id, spec),
                        EngineCommand::Pause { id } =>
                            self.handle_pause(id).await,
                        EngineCommand::Resume { id } =>
                            self.handle_resume(id),
                        EngineCommand::Cancel { id } =>
                            self.handle_cancel(id),
                        EngineCommand::DeleteArtifact { id, destination } =>
                            self.handle_delete_artifact(id, &destination),
                        EngineCommand::Restore { download } => {
                            if !provider::supports_control_action(
                                &download.spec,
                                DownloadControlAction::Restore,
                            ) {
                                self.notify_unsupported_control(
                                    download.id,
                                    DownloadControlAction::Restore,
                                );
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
                        EngineCommand::UpdateSettings { settings } => {
                            self.handle_update_settings(settings);
                        }
                        EngineCommand::Shutdown => {
                            self.handle_shutdown();
                            break;
                        }
                    }
                }
                Some(done) = done_rx.recv() => {
                    self.handle_task_done(done);
                }
            }
        }
    }

    fn spawn_task(
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
            self.progress_tx.clone(),
            self.done_tx.clone(),
            pause_token.clone(),
            resume_data,
            ProviderRuntimeContext {
                shared_scheduler_semaphore: self.shared_scheduler_semaphore(&spec),
                global_throttle: Arc::clone(&self.global_throttle),
            },
        );

        self.tasks.insert(
            id,
            TaskEntry {
                handle,
                pause_token,
                pause_sink,
                destination_sink,
                spec,
            },
        );
    }

    /// Pop queued tasks and spawn them until we hit max_concurrent or the queue is empty.
    fn try_start_next(&mut self) {
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
            self.spawn_task(next.id, next.spec, next.resume_data);
        }
    }

    fn handle_add(&mut self, id: DownloadId, spec: DownloadSpec) {
        let _ = self.db_tx.send(self.added_event(id, &spec));
        if self.tasks.len() < self.max_concurrent {
            tracing::info!(id = id.0, url = spec.url(), "download starting");
            let _ = self.db_tx.send(DbEvent::Started { id });
            self.spawn_task(id, spec, None);
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
    /// read the provider-specific resume state it left in the pause sink.
    /// If the download is in the queue (not yet started), move it to paused directly.
    async fn handle_pause(&mut self, id: DownloadId) {
        let Some(spec) = self.pause_target_spec(id) else {
            return;
        };
        if !provider::supports_control_action(spec, DownloadControlAction::Pause) {
            self.notify_unsupported_control(id, DownloadControlAction::Pause);
            return;
        }

        if let Some(entry) = self.tasks.remove(&id) {
            tracing::info!(id = id.0, "pausing download");
            entry.pause_token.cancel();
            // Task exits quickly: the biased select! in download_chunk fires on the
            // next loop iteration, flushes its write buffer, and returns.
            let _ = entry.handle.await;
            let mut spec = entry.spec;
            self.sync_runtime_destination(id, &mut spec, &entry.destination_sink);
            let resume_data = provider::take_resume_data(entry.pause_sink);
            if let Some(resume_data) = resume_data {
                let downloaded_bytes = resume_data.downloaded_bytes();
                let _ = self.db_tx.send(DbEvent::Paused {
                    id,
                    downloaded_bytes,
                    resume_data: Some(resume_data.clone()),
                });
                self.paused.insert(
                    id,
                    PausedTask {
                        spec,
                        resume_data: Some(resume_data),
                    },
                );
            }
        } else if let Some(pos) = self.queue.iter().position(|t| t.id == id) {
            // Not started yet -> pull from queue and park in paused with no progress.
            let task = self.queue.remove(pos).unwrap();
            tracing::info!(id = id.0, "pausing queued (unstarted) download");
            let _ = self.db_tx.send(DbEvent::Paused {
                id,
                downloaded_bytes: 0,
                resume_data: None,
            });
            let _ = self.notification_tx.send(self.status_notification(
                id,
                DownloadStatus::Paused,
                0,
                None,
            ));
            self.paused.insert(
                id,
                PausedTask {
                    spec: task.spec,
                    resume_data: task.resume_data,
                },
            );
        }
    }

    fn handle_resume(&mut self, id: DownloadId) {
        let Some(spec) = self.resume_target_spec(id) else {
            return;
        };
        if !provider::supports_control_action(spec, DownloadControlAction::Resume) {
            self.notify_unsupported_control(id, DownloadControlAction::Resume);
            return;
        }

        if let Some(pt) = self.paused.remove(&id) {
            tracing::info!(id = id.0, "resuming download");
            let (downloaded_bytes, total_bytes) = snapshot_totals(pt.resume_data.as_ref());
            if self.tasks.len() < self.max_concurrent {
                let _ = self.db_tx.send(DbEvent::Resumed { id });
                let _ = self.notification_tx.send(self.status_notification(
                    id,
                    DownloadStatus::Downloading,
                    downloaded_bytes,
                    total_bytes,
                ));
                self.spawn_task(id, pt.spec, pt.resume_data);
            } else {
                // At capacity -> put at front of queue so it's next to start.
                let _ = self.db_tx.send(DbEvent::Queued { id });
                let _ = self.notification_tx.send(self.status_notification(
                    id,
                    DownloadStatus::Pending,
                    downloaded_bytes,
                    total_bytes,
                ));
                self.queue.push_front(QueuedTask {
                    id,
                    spec: pt.spec,
                    resume_data: pt.resume_data,
                });
            }
        }
    }

    fn handle_cancel(&mut self, id: DownloadId) {
        let Some((supports_cancel, destination)) = self.cancel_target(id) else {
            return;
        };
        if !supports_cancel {
            self.notify_unsupported_control(id, DownloadControlAction::Cancel);
            return;
        }

        let mut removed = false;
        if let Some(entry) = self.tasks.remove(&id) {
            tracing::info!(id = id.0, "download cancelled");
            // abort() prevents done_tx from firing, so we advance the queue manually.
            entry.handle.abort();
            self.try_start_next();
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
            let _ = self
                .notification_tx
                .send(EngineNotification::LiveTransferRemoved {
                    id,
                    action: LiveTransferRemovalAction::Cancelled,
                    artifact_state,
                });
        }
    }

    fn handle_delete_artifact(&mut self, id: DownloadId, destination: &Path) {
        let resolved_destination = self
            .known_destination(id)
            .unwrap_or_else(|| destination.to_path_buf());
        let was_active = self.tasks.remove(&id).map(|entry| {
            entry.handle.abort();
        });
        if was_active.is_some() {
            self.try_start_next();
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
        let _ = self
            .notification_tx
            .send(EngineNotification::LiveTransferRemoved {
                id,
                action: LiveTransferRemovalAction::DeleteArtifact,
                artifact_state,
            });
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

    fn handle_task_done(&mut self, done: TaskDone) {
        let active_entry = self.tasks.remove(&done.id);
        if active_entry.is_none() && !self.paused.contains_key(&done.id) {
            return;
        }
        if let Some(mut entry) = active_entry {
            self.sync_runtime_destination(done.id, &mut entry.spec, &entry.destination_sink);
        }

        match done.final_state.status {
            DownloadStatus::Finished => {
                let _ = self.db_tx.send(DbEvent::Finished {
                    id: done.id,
                    total_bytes: done
                        .final_state
                        .total_bytes
                        .unwrap_or(done.final_state.downloaded_bytes),
                });
                self.try_start_next();
            }
            DownloadStatus::Error => {
                let _ = self.db_tx.send(DbEvent::Error { id: done.id });
                self.try_start_next();
            }
            DownloadStatus::Paused => {
                if !self.paused.contains_key(&done.id) {
                    self.try_start_next();
                }
            }
            DownloadStatus::Pending | DownloadStatus::Downloading | DownloadStatus::Cancelled => {
                self.try_start_next();
            }
        }
    }

    fn handle_update_settings(&mut self, settings: Settings) {
        let old_max_concurrent = self.max_concurrent;
        let old_settings = self.settings.clone();

        self.max_concurrent = settings.max_concurrent_downloads;
        self.global_throttle
            .set_limit(settings.global_speed_limit_bps);
        self.adjust_shared_scheduler_limits(&old_settings, &settings);
        self.settings = settings;

        if self.max_concurrent > old_max_concurrent {
            self.try_start_next();
        }
    }

    fn adjust_shared_scheduler_limits(&mut self, old_settings: &Settings, new_settings: &Settings) {
        for (key, semaphore) in &self.shared_schedulers {
            let Some(old_limit) = provider::shared_scheduler_limit(key, old_settings) else {
                continue;
            };
            let Some(new_limit) = provider::shared_scheduler_limit(key, new_settings) else {
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

    fn status_notification(
        &self,
        id: DownloadId,
        status: DownloadStatus,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    ) -> EngineNotification {
        EngineNotification::Update(ProgressUpdate {
            id,
            status,
            downloaded_bytes,
            total_bytes,
            speed_bytes_per_sec: 0,
        })
    }

    fn notify_unsupported_control(&self, id: DownloadId, action: DownloadControlAction) {
        let _ = self
            .notification_tx
            .send(EngineNotification::ControlUnsupported { id, action });
    }

    fn pause_target_spec(&self, id: DownloadId) -> Option<&DownloadSpec> {
        self.tasks.get(&id).map(|entry| &entry.spec).or_else(|| {
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
                provider::supports_control_action(&entry.spec, DownloadControlAction::Cancel),
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

    fn sync_runtime_destination(
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
        let _ = self
            .notification_tx
            .send(EngineNotification::DestinationChanged { id, destination });
    }
}

fn snapshot_totals(resume_data: Option<&ProviderResumeData>) -> (u64, Option<u64>) {
    match resume_data {
        Some(data) => (data.downloaded_bytes(), data.total_bytes()),
        None => (0, None),
    }
}

fn artifact_paths(destination: &Path) -> [PathBuf; 2] {
    [
        destination.to_path_buf(),
        PathBuf::from(format!("{}.ophelia_part", destination.display())),
    ]
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
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(path = %path.display(), "failed to delete artifact: {error}")
            }
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
