use std::collections::HashMap;
use std::path::PathBuf;

use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::engine::task::download_task;
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

        runtime.spawn(Self::run(cmd_rx, progress_tx));

        Self {
            runtime,
            cmd_tx,
            progress_rx,
            next_id: 0,
        }
    }

    pub fn add(&mut self, url: String, destination: PathBuf) -> DownloadId {
        let id = DownloadId(self.next_id);
        self.next_id += 1;
        let _ = self.cmd_tx.send(EngineCommand::Add { id, url, destination });
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

    async fn run(
        mut cmd_rx: mpsc::UnboundedReceiver<EngineCommand>,
        progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
    ) {
        let mut tasks: HashMap<DownloadId, JoinHandle<()>> = HashMap::new();

        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                EngineCommand::Add { id, url, destination } => {
                    let tx = progress_tx.clone();
                    let handle = tokio::spawn(download_task(id, url, destination, tx));
                    tasks.insert(id, handle);
                }
                EngineCommand::Pause { id } => {
                    // TODO: cancellation token
                }
                EngineCommand::Resume { id } => {
                    // TODO: re-spawn with byte offset
                }
                EngineCommand::Cancel { id } => {
                    if let Some(handle) = tasks.remove(&id) {
                        handle.abort();
                    }
                }
                EngineCommand::Shutdown => {
                    for (_, handle) in tasks.drain() {
                        handle.abort();
                    }
                    break;
                }
            }
        }
    }
}
