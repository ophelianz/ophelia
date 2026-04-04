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
use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::engine::http::{TaskFinalState, TokenBucket, download_task};
use crate::engine::{
    ChunkSnapshot, DbEvent, DownloadId, DownloadSource, DownloadSpec, DownloadStatus,
    EngineNotification, HttpResumeData, PersistedDownloadSource, ProgressUpdate,
    ProviderResumeData, RestoredDownload,
};
use crate::settings::Settings;

fn host_from_url(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let after_auth = after_scheme.rsplit('@').next().unwrap_or(after_scheme);
    after_auth
        .split('/')
        .next()
        .unwrap_or(after_auth)
        .to_lowercase()
}

#[allow(dead_code)]
enum EngineCommand {
    Add { id: DownloadId, spec: DownloadSpec },
    Pause { id: DownloadId },
    Resume { id: DownloadId },
    Cancel { id: DownloadId },
    Restore { download: RestoredDownload },
    UpdateSettings { settings: Settings },
    Shutdown,
}

struct TaskDone {
    id: DownloadId,
    final_state: TaskFinalState,
}

// --- per-task bookkeeping ------------------------------------------------

/// Everything the engine needs to pause or cancel an active task.
struct TaskEntry {
    handle: JoinHandle<()>,
    /// Fired on soft pause. Hard cancel uses handle.abort() instead.
    pause_token: CancellationToken,
    /// Written by the task on pause; read by the engine after awaiting the handle.
    pause_sink: TaskPauseSink,
    /// Kept for re-spawning on resume.
    spec: DownloadSpec,
}

enum TaskPauseSink {
    Http(Arc<Mutex<Option<Vec<ChunkSnapshot>>>>),
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

    pub fn cancel(&self, id: DownloadId) {
        let _ = self.cmd_tx.send(EngineCommand::Cancel { id });
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
    /// One semaphore per hostname; permits = settings.max_connections_per_server.
    /// Shared across all downloads targeting the same host.
    server_semaphores: HashMap<String, Arc<Semaphore>>,
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
            server_semaphores: HashMap::new(),
            db_tx,
            global_throttle,
        }
    }

    fn server_semaphore(&mut self, url: &str) -> Arc<Semaphore> {
        let host = host_from_url(url);
        let limit = self.settings.max_connections_per_server;
        self.server_semaphores
            .entry(host)
            .or_insert_with(|| Arc::new(Semaphore::new(limit)))
            .clone()
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
                        EngineCommand::Restore { download } => {
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
        let tx = self.progress_tx.clone();
        let done_tx = self.done_tx.clone();

        let (handle, pause_sink) = match &spec.source {
            DownloadSource::Http { url, config } => {
                let pause_sink: Arc<Mutex<Option<Vec<ChunkSnapshot>>>> = Arc::new(Mutex::new(None));
                let server_semaphore = self.server_semaphore(url);
                let resume_from = resume_data
                    .as_ref()
                    .and_then(ProviderResumeData::as_http)
                    .map(|data| data.chunks.clone());
                let handle = tokio::spawn({
                    let url_ = url.clone();
                    let dest_ = spec.destination.clone();
                    let cfg_ = config.clone();
                    let pt_ = pause_token.clone();
                    let ps_ = Arc::clone(&pause_sink);
                    let gt_ = Arc::clone(&self.global_throttle);
                    async move {
                        let final_state = download_task(
                            id,
                            url_,
                            dest_,
                            cfg_,
                            tx,
                            pt_,
                            ps_,
                            resume_from,
                            server_semaphore,
                            gt_,
                        )
                        .await;
                        let _ = done_tx.send(TaskDone { id, final_state });
                    }
                });
                (handle, TaskPauseSink::Http(pause_sink))
            }
        };

        self.tasks.insert(
            id,
            TaskEntry {
                handle,
                pause_token,
                pause_sink,
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
            let _ = self.db_tx.send(self.started_event(next.id, &next.spec));
            self.spawn_task(next.id, next.spec, next.resume_data);
        }
    }

    fn handle_add(&mut self, id: DownloadId, spec: DownloadSpec) {
        if self.tasks.len() < self.max_concurrent {
            tracing::info!(id = id.0, url = spec.url(), "download starting");
            let _ = self.db_tx.send(self.started_event(id, &spec));
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
        if let Some(entry) = self.tasks.remove(&id) {
            tracing::info!(id = id.0, "pausing download");
            entry.pause_token.cancel();
            // Task exits quickly: the biased select! in download_chunk fires on the
            // next loop iteration, flushes its write buffer, and returns.
            let _ = entry.handle.await;
            let resume_data = match entry.pause_sink {
                TaskPauseSink::Http(sink) => sink
                    .lock()
                    .unwrap()
                    .take()
                    .map(HttpResumeData::new)
                    .map(ProviderResumeData::Http),
            };
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
                        spec: entry.spec,
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
        if let Some(pt) = self.paused.remove(&id) {
            tracing::info!(id = id.0, "resuming download");
            let _ = self.db_tx.send(DbEvent::Resumed { id });
            let (downloaded_bytes, total_bytes) = snapshot_totals(pt.resume_data.as_ref());
            if self.tasks.len() < self.max_concurrent {
                let _ = self.notification_tx.send(self.status_notification(
                    id,
                    DownloadStatus::Downloading,
                    downloaded_bytes,
                    total_bytes,
                ));
                self.spawn_task(id, pt.spec, pt.resume_data);
            } else {
                // At capacity -> put at front of queue so it's next to start.
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
        if let Some(entry) = self.tasks.remove(&id) {
            tracing::info!(id = id.0, "download cancelled");
            // abort() prevents done_tx from firing, so we advance the queue manually.
            entry.handle.abort();
            self.try_start_next();
        }
        // Also remove from queue or paused if it hadn't started yet.
        self.queue.retain(|t| t.id != id);
        self.paused.remove(&id);
        let _ = self.db_tx.send(DbEvent::Removed { id });
        let _ = self
            .notification_tx
            .send(EngineNotification::Removed { id });
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
        let removed_active = self.tasks.remove(&done.id).is_some();
        if !removed_active && !self.paused.contains_key(&done.id) {
            return;
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
            DownloadStatus::Pending | DownloadStatus::Downloading => {
                self.try_start_next();
            }
        }
    }

    fn handle_update_settings(&mut self, settings: Settings) {
        let old_max_concurrent = self.max_concurrent;
        let old_per_server = self.settings.max_connections_per_server;

        self.max_concurrent = settings.max_concurrent_downloads;
        self.global_throttle
            .set_limit(settings.global_speed_limit_bps);
        self.adjust_server_semaphores(old_per_server, settings.max_connections_per_server);
        self.settings = settings;

        if self.max_concurrent > old_max_concurrent {
            self.try_start_next();
        }
    }

    fn adjust_server_semaphores(&mut self, old_limit: usize, new_limit: usize) {
        if old_limit == new_limit {
            return;
        }

        for semaphore in self.server_semaphores.values() {
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

    fn started_event(&self, id: DownloadId, spec: &DownloadSpec) -> DbEvent {
        DbEvent::Started {
            id,
            source: match &spec.source {
                DownloadSource::Http { url, .. } => {
                    PersistedDownloadSource::Http { url: url.clone() }
                }
            },
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
}

fn snapshot_totals(resume_data: Option<&ProviderResumeData>) -> (u64, Option<u64>) {
    match resume_data {
        Some(data) => (data.downloaded_bytes(), data.total_bytes()),
        None => (0, None),
    }
}
