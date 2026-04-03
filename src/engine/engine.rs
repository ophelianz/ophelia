//! The download engine actor.
//!
//! Owns the tokio runtime and sits between the UI thread and download tasks.
//! Commands arrive over a cmd channel; progress updates flow back the other way.
//!
//! Queue lifecycle:
//!   Add    → if tasks < max_concurrent, spawn immediately; else push to queue.
//!   Done   → done_rx fires when a task returns naturally (finish or error);
//!            remove from tasks, advance queue.
//!   Pause  → cancel the task's CancellationToken, await the handle, read chunk
//!            offsets from the pause_sink, store in `paused` map. If the id is
//!            in the queue (not yet started), move it directly to `paused`.
//!   Resume → if at capacity, push to front of queue; else spawn immediately.
//!   Cancel → abort the handle (prevents done_tx from firing), drain from queue,
//!            then advance queue manually since done_rx won't fire.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::runtime::Runtime;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

fn host_from_url(url: &str) -> String {
    let after_scheme = url.splitn(2, "://").nth(1).unwrap_or(url);
    let after_auth = after_scheme.splitn(2, '@').last().unwrap_or(after_scheme);
    after_auth
        .splitn(2, '/')
        .next()
        .unwrap_or(after_auth)
        .to_lowercase()
}

use crate::engine::http::{HttpDownloadConfig, TokenBucket, download_task};
use crate::engine::types::*;
use crate::settings::Settings;

// --- per-task bookkeeping ------------------------------------------------

/// Everything the engine needs to pause or cancel an active task.
struct TaskEntry {
    handle: JoinHandle<()>,
    /// Fired on soft pause. Hard cancel uses handle.abort() instead.
    pause_token: CancellationToken,
    /// Written by the task on pause; read by the engine after awaiting the handle.
    pause_sink: Arc<Mutex<Option<Vec<ChunkSnapshot>>>>,
    /// Kept for re-spawning on resume.
    url: String,
    destination: PathBuf,
    config: HttpDownloadConfig,
}

/// State stored when a download is paused, used to re-spawn on resume.
struct PausedTask {
    url: String,
    destination: PathBuf,
    config: HttpDownloadConfig,
    snapshots: Vec<ChunkSnapshot>,
}

/// A download waiting in the queue. `resume_from` is Some when the user
/// resumed a paused download that couldn't start immediately.
struct QueuedTask {
    id: DownloadId,
    url: String,
    destination: PathBuf,
    config: HttpDownloadConfig,
    resume_from: Option<Vec<ChunkSnapshot>>,
}

// --- public engine handle ------------------------------------------------

pub struct DownloadEngine {
    #[allow(dead_code)] // must be held to keep the tokio runtime alive
    runtime: Runtime,
    cmd_tx: mpsc::UnboundedSender<EngineCommand>,
    progress_rx: mpsc::UnboundedReceiver<ProgressUpdate>,
    ipc_rx: mpsc::UnboundedReceiver<crate::ipc::DownloadRequest>,
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
        let (ipc_tx, ipc_rx) = mpsc::unbounded_channel();
        let (done_tx, done_rx) = mpsc::unbounded_channel::<DownloadId>();

        runtime.spawn(EngineActor::new(progress_tx, settings, db_tx, done_tx).run(cmd_rx, done_rx));
        runtime.spawn(crate::ipc::serve(ipc_tx));

        Self {
            runtime,
            cmd_tx,
            progress_rx,
            ipc_rx,
            next_id: initial_next_id,
        }
    }

    /// Pre-populate the paused map with a download restored from SQLite.
    /// Does not start a task, user must resume explicitly.
    pub fn restore(
        &self,
        id: DownloadId,
        url: String,
        destination: PathBuf,
        config: HttpDownloadConfig,
        chunks: Vec<ChunkSnapshot>,
    ) {
        let _ = self.cmd_tx.send(EngineCommand::Restore {
            id,
            url,
            destination,
            config,
            chunks,
        });
    }

    /// Non-blocking drain of one pending IPC download request from the browser extension.
    pub fn poll_ipc(&mut self) -> Option<crate::ipc::DownloadRequest> {
        self.ipc_rx.try_recv().ok()
    }

    pub fn add(
        &mut self,
        url: String,
        destination: PathBuf,
        config: HttpDownloadConfig,
    ) -> DownloadId {
        let id = DownloadId(self.next_id);
        self.next_id += 1;
        let _ = self.cmd_tx.send(EngineCommand::AddHttp {
            id,
            url,
            destination,
            config,
        });
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

    pub fn poll_progress(&mut self) -> Option<ProgressUpdate> {
        self.progress_rx.try_recv().ok()
    }
}

// --- actor ---------------------------------------------------------------

/// Owns all mutable engine state and handles commands on the tokio runtime.
/// New state (speed limits, scheduler) goes here as fields.
/// New protocols get a handle_* method; the dispatch loop never changes shape.
struct EngineActor {
    tasks: HashMap<DownloadId, TaskEntry>,
    paused: HashMap<DownloadId, PausedTask>,
    queue: VecDeque<QueuedTask>,
    max_concurrent: usize,
    done_tx: mpsc::UnboundedSender<DownloadId>,
    progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
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
        settings: Settings,
        db_tx: std::sync::mpsc::Sender<DbEvent>,
        done_tx: mpsc::UnboundedSender<DownloadId>,
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
        mut done_rx: mpsc::UnboundedReceiver<DownloadId>,
    ) {
        loop {
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        EngineCommand::AddHttp { id, url, destination, config } =>
                            self.handle_add_http(id, url, destination, config),
                        EngineCommand::Pause { id } =>
                            self.handle_pause(id).await,
                        EngineCommand::Resume { id } =>
                            self.handle_resume(id),
                        EngineCommand::Cancel { id } =>
                            self.handle_cancel(id),
                        EngineCommand::Restore { id, url, destination, config, chunks } => {
                            tracing::info!(id = id.0, "restoring paused download from database");
                            self.paused.insert(id, PausedTask { url, destination, config, snapshots: chunks });
                        }
                        EngineCommand::Shutdown => {
                            self.handle_shutdown();
                            break;
                        }
                    }
                }
                Some(id) = done_rx.recv() => {
                    self.tasks.remove(&id);
                    // A paused task also fires done_rx (task returned normally after
                    // soft-cancel). In that case the id is already in self.paused and
                    // the slot isn't freed so don't advance the queue.
                    if !self.paused.contains_key(&id) {
                        self.try_start_next();
                    }
                }
            }
        }
    }

    fn spawn_task(
        &mut self,
        id: DownloadId,
        url: String,
        destination: PathBuf,
        config: HttpDownloadConfig,
        resume_from: Option<Vec<ChunkSnapshot>>,
    ) {
        let pause_token = CancellationToken::new();
        let pause_sink: Arc<Mutex<Option<Vec<ChunkSnapshot>>>> = Arc::new(Mutex::new(None));
        let tx = self.progress_tx.clone();
        let server_semaphore = self.server_semaphore(&url);
        let done_tx = self.done_tx.clone();

        let handle = tokio::spawn({
            let url_ = url.clone();
            let dest_ = destination.clone();
            let cfg_ = config.clone();
            let pt_ = pause_token.clone();
            let ps_ = Arc::clone(&pause_sink);
            let gt_ = Arc::clone(&self.global_throttle);
            async move {
                download_task(
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
                let _ = done_tx.send(id);
            }
        });

        self.tasks.insert(
            id,
            TaskEntry {
                handle,
                pause_token,
                pause_sink,
                url,
                destination,
                config,
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
            let _ = self.db_tx.send(DbEvent::Started {
                id: next.id,
                url: next.url.clone(),
                destination: next.destination.clone(),
            });
            self.spawn_task(
                next.id,
                next.url,
                next.destination,
                next.config,
                next.resume_from,
            );
        }
    }

    fn handle_add_http(
        &mut self,
        id: DownloadId,
        url: String,
        destination: PathBuf,
        config: HttpDownloadConfig,
    ) {
        if self.tasks.len() < self.max_concurrent {
            tracing::info!(id = id.0, %url, "download starting");
            let _ = self.db_tx.send(DbEvent::Started {
                id,
                url: url.clone(),
                destination: destination.clone(),
            });
            self.spawn_task(id, url, destination, config, None);
        } else {
            tracing::info!(id = id.0, %url, queued = self.queue.len() + 1, "download queued (at capacity)");
            self.queue.push_back(QueuedTask {
                id,
                url,
                destination,
                config,
                resume_from: None,
            });
        }
    }

    /// Soft pause: fire the CancellationToken, wait for the task to drain, then
    /// read the chunk snapshots it left in the pause_sink.
    /// If the download is in the queue (not yet started), move it to paused directly.
    async fn handle_pause(&mut self, id: DownloadId) {
        if let Some(entry) = self.tasks.remove(&id) {
            tracing::info!(id = id.0, "pausing download");
            entry.pause_token.cancel();
            // Task exits quickly: the biased select! in download_chunk fires on the
            // next loop iteration, flushes its write buffer, and returns.
            let _ = entry.handle.await;
            let snapshots = entry.pause_sink.lock().unwrap().take();
            if let Some(snaps) = snapshots {
                let downloaded_bytes: u64 = snaps.iter().map(|c| c.downloaded).sum();
                let _ = self.db_tx.send(DbEvent::Paused {
                    id,
                    downloaded_bytes,
                    chunks: snaps.clone(),
                });
                self.paused.insert(
                    id,
                    PausedTask {
                        url: entry.url,
                        destination: entry.destination,
                        config: entry.config,
                        snapshots: snaps,
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
                chunks: Vec::new(),
            });
            self.paused.insert(
                id,
                PausedTask {
                    url: task.url,
                    destination: task.destination,
                    config: task.config,
                    snapshots: Vec::new(),
                },
            );
        }
    }

    fn handle_resume(&mut self, id: DownloadId) {
        if let Some(pt) = self.paused.remove(&id) {
            tracing::info!(id = id.0, "resuming download");
            let _ = self.db_tx.send(DbEvent::Resumed { id });
            if self.tasks.len() < self.max_concurrent {
                self.spawn_task(id, pt.url, pt.destination, pt.config, Some(pt.snapshots));
            } else {
                // At capacity -> put at front of queue so it's next to start.
                self.queue.push_front(QueuedTask {
                    id,
                    url: pt.url,
                    destination: pt.destination,
                    config: pt.config,
                    resume_from: Some(pt.snapshots),
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
}
