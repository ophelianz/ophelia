//! The download engine actor.
//!
//! Owns the tokio runtime and sits between the UI thread and download tasks.
//! Commands arrive over an mpsc channel; progress updates flow back the other way.
//! The task map and ID counter live inside the async run() loop so no mutexes needed.

use std::collections::HashMap;
use std::path::PathBuf;

use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::engine::http::download_task;
use crate::engine::http::HttpDownloadConfig;
use crate::engine::types::*;

pub struct DownloadEngine {
    runtime: Runtime,
    cmd_tx: mpsc::UnboundedSender<EngineCommand>,
    progress_rx: mpsc::UnboundedReceiver<ProgressUpdate>,
    next_id: u64,
}

impl DownloadEngine {
    pub fn new() -> Self {
        let runtime = Runtime::new().expect("failed to create tokio runtime");
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();

        runtime.spawn(EngineActor::new(progress_tx).run(cmd_rx));

        Self {
            runtime,
            cmd_tx,
            progress_rx,
            next_id: 0,
        }
    }

    pub fn add(&mut self, url: String, destination: PathBuf, config: HttpDownloadConfig) -> DownloadId {
        let id = DownloadId(self.next_id);
        self.next_id += 1;
        let _ = self.cmd_tx.send(EngineCommand::AddHttp { id, url, destination, config });
        id
    }

    pub fn pause(&self, id: DownloadId) {
        let _ = self.cmd_tx.send(EngineCommand::Pause { id });
    }

    pub fn cancel(&self, id: DownloadId) {
        let _ = self.cmd_tx.send(EngineCommand::Cancel { id });
    }

    pub fn poll_progress(&mut self) -> Option<ProgressUpdate> {
        self.progress_rx.try_recv().ok()
    }
}

/// Owns all mutable engine state and handles commands on the tokio runtime.
/// New state (cancellation tokens, queue, speed limits) goes here as fields.
/// New protocols get a handle_* method; the dispatch loop never changes shape.
struct EngineActor {
    tasks: HashMap<DownloadId, JoinHandle<()>>,
    progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
}

impl EngineActor {
    fn new(progress_tx: mpsc::UnboundedSender<ProgressUpdate>) -> Self {
        Self {
            tasks: HashMap::new(),
            progress_tx,
        }
    }

    async fn run(mut self, mut cmd_rx: mpsc::UnboundedReceiver<EngineCommand>) {
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                EngineCommand::AddHttp { id, url, destination, config } =>
                    self.handle_add_http(id, url, destination, config),
                EngineCommand::Pause { id: _ } => {
                    // TODO: cancellation token
                }
                EngineCommand::Resume { id: _ } => {
                    // TODO: re-spawn with byte offset
                }
                EngineCommand::Cancel { id } =>
                    self.handle_cancel(id),
                EngineCommand::Shutdown =>  {
                    self.handle_shutdown();
                    break;
                }
            }
        }
    }

    fn handle_add_http(&mut self, id: DownloadId, url: String, destination: PathBuf, config: HttpDownloadConfig) {
        let tx = self.progress_tx.clone();
        let handle = tokio::spawn(download_task(id, url, destination, config, tx));
        self.tasks.insert(id, handle);
    }

    fn handle_cancel(&mut self, id: DownloadId) {
        if let Some(handle) = self.tasks.remove(&id) {
            handle.abort();
        }
    }

    fn handle_shutdown(&mut self) {
        for (_, handle) in self.tasks.drain() {
            handle.abort();
        }
    }
}
