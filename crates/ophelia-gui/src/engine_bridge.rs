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

//! Tokio bridge between GPUI state and the backend engine.
//!
//! This copies Zed's shape, not its whole boot sequence: the GPUI app owns one
//! Tokio runtime, and long-running app services sit behind lightweight handles.

use futures::channel::mpsc;
use tokio::runtime::Handle;
use tokio::sync::mpsc as tokio_mpsc;
use tokio::task::JoinHandle;

use crate::engine::{
    DownloadEngine, DownloadId, DownloadSpec, EngineError, EngineEvent, RestoredDownload,
    SavedDownload,
};
use crate::settings::Settings;

#[derive(Debug)]
pub enum EngineBridgeEvent {
    Engine(EngineEvent),
    CommandFailed {
        id: Option<DownloadId>,
        action: EngineCommandKind,
        error: EngineError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineCommandKind {
    Add,
    Pause,
    Resume,
    Cancel,
    DeleteArtifact,
    Restore,
}

#[derive(Clone)]
pub struct EngineClient {
    tx: tokio_mpsc::UnboundedSender<EngineCommand>,
}

impl EngineClient {
    pub fn add(&self, spec: DownloadSpec) {
        self.send(EngineCommand::Add { spec });
    }

    pub fn pause(&self, id: DownloadId) {
        self.send(EngineCommand::Pause { id });
    }

    pub fn resume(&self, id: DownloadId) {
        self.send(EngineCommand::Resume { id });
    }

    #[allow(dead_code)] // reserved for a future UI with a distinct cancel-transfer action.
    pub fn cancel(&self, id: DownloadId) {
        self.send(EngineCommand::Cancel { id });
    }

    pub fn delete_artifact(&self, id: DownloadId) {
        self.send(EngineCommand::DeleteArtifact { id });
    }

    pub fn update_settings(&self, settings: &Settings) {
        self.send(EngineCommand::UpdateConfig {
            config: settings.core_config(),
        });
    }

    fn send(&self, command: EngineCommand) {
        if let Err(error) = self.tx.send(command) {
            tracing::warn!("download engine command channel is closed: {error}");
        }
    }
}

pub struct EngineBridge {
    client: EngineClient,
    events: Option<mpsc::UnboundedReceiver<EngineBridgeEvent>>,
    task: JoinHandle<()>,
}

impl EngineBridge {
    pub fn spawn(
        handle: &Handle,
        settings: &Settings,
        db_tx: std::sync::mpsc::Sender<crate::engine::types::DbEvent>,
        saved_downloads: Vec<SavedDownload>,
        initial_next_id: u64,
    ) -> Self {
        let (command_tx, command_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, events) = mpsc::unbounded();
        let config = settings.core_config();
        let engine = DownloadEngine::spawn_on(handle, config.clone(), db_tx, initial_next_id);

        let task = handle.spawn(run_engine_bridge(
            engine,
            command_rx,
            event_tx,
            config,
            saved_downloads,
        ));

        Self {
            client: EngineClient { tx: command_tx },
            events: Some(events),
            task,
        }
    }

    pub fn client(&self) -> EngineClient {
        self.client.clone()
    }

    pub fn take_events(&mut self) -> mpsc::UnboundedReceiver<EngineBridgeEvent> {
        self.events
            .take()
            .expect("engine bridge events were already taken")
    }
}

impl Drop for EngineBridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug)]
enum EngineCommand {
    Add { spec: DownloadSpec },
    Pause { id: DownloadId },
    Resume { id: DownloadId },
    Cancel { id: DownloadId },
    DeleteArtifact { id: DownloadId },
    UpdateConfig { config: crate::engine::CoreConfig },
}

async fn run_engine_bridge(
    mut engine: DownloadEngine,
    mut command_rx: tokio_mpsc::UnboundedReceiver<EngineCommand>,
    event_tx: mpsc::UnboundedSender<EngineBridgeEvent>,
    config: crate::engine::CoreConfig,
    saved_downloads: Vec<SavedDownload>,
) {
    for saved in saved_downloads {
        let restored = RestoredDownload::from_saved(&saved, &config);
        if let Err(error) = engine.restore(restored).await {
            send_event(
                &event_tx,
                EngineBridgeEvent::CommandFailed {
                    id: Some(saved.id),
                    action: EngineCommandKind::Restore,
                    error,
                },
            );
        }
    }

    loop {
        tokio::select! {
            Some(command) = command_rx.recv() => {
                handle_command(command, &mut engine, &event_tx).await;
            }
            event = engine.next_event() => {
                let Some(event) = event else {
                    break;
                };
                send_event(&event_tx, EngineBridgeEvent::Engine(event));
            }
            else => break,
        }
    }

    if let Err(error) = engine.shutdown().await {
        tracing::warn!("download engine shutdown failed: {error}");
    }
}

async fn handle_command(
    command: EngineCommand,
    engine: &mut DownloadEngine,
    event_tx: &mpsc::UnboundedSender<EngineBridgeEvent>,
) {
    match command {
        EngineCommand::Add { spec } => {
            if let Err(error) = engine.add(spec).await {
                send_command_failed(event_tx, None, EngineCommandKind::Add, error);
            }
        }
        EngineCommand::Pause { id } => {
            if let Err(error) = engine.pause(id).await {
                send_command_failed(event_tx, Some(id), EngineCommandKind::Pause, error);
            }
        }
        EngineCommand::Resume { id } => {
            if let Err(error) = engine.resume(id).await {
                send_command_failed(event_tx, Some(id), EngineCommandKind::Resume, error);
            }
        }
        EngineCommand::Cancel { id } => {
            if let Err(error) = engine.cancel(id).await {
                send_command_failed(event_tx, Some(id), EngineCommandKind::Cancel, error);
            }
        }
        EngineCommand::DeleteArtifact { id } => {
            if let Err(error) = engine.delete_artifact(id).await {
                send_command_failed(event_tx, Some(id), EngineCommandKind::DeleteArtifact, error);
            }
        }
        EngineCommand::UpdateConfig { config } => {
            if let Err(error) = engine.update_config(config).await {
                tracing::warn!("download engine settings update failed: {error}");
            }
        }
    }
}

fn send_command_failed(
    event_tx: &mpsc::UnboundedSender<EngineBridgeEvent>,
    id: Option<DownloadId>,
    action: EngineCommandKind,
    error: EngineError,
) {
    send_event(
        event_tx,
        EngineBridgeEvent::CommandFailed { id, action, error },
    );
}

fn send_event(event_tx: &mpsc::UnboundedSender<EngineBridgeEvent>, event: EngineBridgeEvent) {
    if let Err(error) = event_tx.unbounded_send(event) {
        tracing::warn!("download engine event receiver is closed: {error}");
    }
}
