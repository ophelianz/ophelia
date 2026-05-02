//! Service-owned transfer runtime
//!
//! Owns active, queued, and paused transfers for one OpheliaService
//! HTTP still does the protocol work; this module owns when transfers start and stop

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use bitvec::prelude::{BitVec, Lsb0};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::EngineConfig;
use crate::disk::DiskHandle;
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
const NO_INDEX: usize = usize::MAX;

struct TaskEntry {
    handle: JoinHandle<crate::engine::http::TaskFinalState>,
    pause_token: CancellationToken,
    pause_sink: TaskPauseSink,
    destination_sink: TaskDestinationSink,
    control_support: TransferControlSupport,
}

struct TransferStart {
    id: TransferId,
    spec: DownloadSpec,
    resume_data: Option<ProviderResumeData>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TransferRow(usize);

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransferRuntimeState {
    Active,
    Paused,
    Queued,
    Removed,
}

#[derive(Default)]
struct TransferTable {
    row_by_id: HashMap<TransferId, TransferRow>,
    ids: Vec<TransferId>,
    states: Vec<TransferRuntimeState>,
    specs: Vec<Option<DownloadSpec>>,
    active_entries: Vec<Option<TaskEntry>>,
    resume_data: Vec<Option<ProviderResumeData>>,
    flags: TransferFlags,
    active_rows: Vec<TransferRow>,
    paused_rows: Vec<TransferRow>,
    queued_rows: VecDeque<TransferRow>,
    active_positions: Vec<usize>,
    paused_positions: Vec<usize>,
}

#[derive(Default)]
struct TransferFlags {
    pause_requested: BitVec<usize, Lsb0>,
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

impl TransferTable {
    fn active_len(&self) -> usize {
        self.active_rows.len()
    }

    fn paused_len(&self) -> usize {
        self.paused_rows.len()
    }

    fn queued_len(&self) -> usize {
        self.queued_rows.len()
    }

    fn contains_active(&self, id: TransferId) -> bool {
        self.row_for(id)
            .is_some_and(|row| self.states[row.0] == TransferRuntimeState::Active)
    }

    fn contains_paused(&self, id: TransferId) -> bool {
        self.row_for(id)
            .is_some_and(|row| self.states[row.0] == TransferRuntimeState::Paused)
    }

    fn active_entry_mut(&mut self, id: TransferId) -> Option<&mut TaskEntry> {
        let row = self.row_for(id)?;
        (self.states[row.0] == TransferRuntimeState::Active).then_some(())?;
        self.active_entries[row.0].as_mut()
    }

    fn active_entry(&self, id: TransferId) -> Option<&TaskEntry> {
        let row = self.row_for(id)?;
        (self.states[row.0] == TransferRuntimeState::Active).then_some(())?;
        self.active_entries[row.0].as_ref()
    }

    fn spec(&self, id: TransferId) -> Option<&DownloadSpec> {
        let row = self.row_for(id)?;
        self.specs[row.0].as_ref()
    }

    fn spec_mut(&mut self, id: TransferId) -> Option<&mut DownloadSpec> {
        let row = self.row_for(id)?;
        self.specs[row.0].as_mut()
    }

    fn set_active(&mut self, id: TransferId, spec: DownloadSpec, entry: TaskEntry) {
        let row = self.ensure_row(id);
        self.leave_current_list(row);
        self.states[row.0] = TransferRuntimeState::Active;
        self.specs[row.0] = Some(spec);
        self.active_entries[row.0] = Some(entry);
        self.resume_data[row.0] = None;
        self.flags.pause_requested.set(row.0, false);
        self.active_positions[row.0] = self.active_rows.len();
        self.active_rows.push(row);
    }

    fn set_queued_back(
        &mut self,
        id: TransferId,
        spec: DownloadSpec,
        resume_data: Option<ProviderResumeData>,
    ) {
        let row = self.prepare_queued(id, spec, resume_data);
        self.queued_rows.push_back(row);
    }

    fn set_queued_front(
        &mut self,
        id: TransferId,
        spec: DownloadSpec,
        resume_data: Option<ProviderResumeData>,
    ) {
        let row = self.prepare_queued(id, spec, resume_data);
        self.queued_rows.push_front(row);
    }

    fn set_paused(
        &mut self,
        id: TransferId,
        spec: DownloadSpec,
        resume_data: Option<ProviderResumeData>,
    ) {
        let row = self.ensure_row(id);
        self.leave_current_list(row);
        self.states[row.0] = TransferRuntimeState::Paused;
        self.specs[row.0] = Some(spec);
        self.active_entries[row.0] = None;
        self.resume_data[row.0] = resume_data;
        self.flags.pause_requested.set(row.0, false);
        self.paused_positions[row.0] = self.paused_rows.len();
        self.paused_rows.push(row);
    }

    fn mark_pause_requested(&mut self, id: TransferId) -> bool {
        let Some(row) = self.row_for(id) else {
            return false;
        };
        if self.states[row.0] != TransferRuntimeState::Active {
            return false;
        }
        self.flags.pause_requested.set(row.0, true);
        true
    }

    fn pause_requested(&self, id: TransferId) -> bool {
        self.row_for(id).is_some_and(|row| {
            self.flags
                .pause_requested
                .get(row.0)
                .is_some_and(|bit| *bit)
        })
    }

    fn take_queued(&mut self, id: TransferId) -> Option<TransferStart> {
        let row = self.row_for(id)?;
        if self.states[row.0] != TransferRuntimeState::Queued {
            return None;
        }
        let pos = self.queued_rows.iter().position(|queued| *queued == row)?;
        self.queued_rows.remove(pos);
        self.take_start(row)
    }

    fn take_paused(&mut self, id: TransferId) -> Option<TransferStart> {
        let row = self.row_for(id)?;
        if self.states[row.0] != TransferRuntimeState::Paused {
            return None;
        }
        self.remove_paused(row);
        self.take_start(row)
    }

    fn pop_next_queued(&mut self) -> Option<TransferStart> {
        let row = self.queued_rows.pop_front()?;
        self.take_start(row)
    }

    fn remove_active(&mut self, id: TransferId) -> Option<(TaskEntry, DownloadSpec)> {
        let row = self.row_for(id)?;
        if self.states[row.0] != TransferRuntimeState::Active {
            return None;
        }
        self.remove_active_row(row);
        self.states[row.0] = TransferRuntimeState::Removed;
        self.flags.pause_requested.set(row.0, false);
        let entry = self.active_entries[row.0].take()?;
        let spec = self.specs[row.0].take()?;
        self.resume_data[row.0] = None;
        Some((entry, spec))
    }

    fn remove_any(&mut self, id: TransferId) -> Option<RemovedTransfer> {
        let row = self.row_for(id)?;
        let state = self.states[row.0];
        let active_entry = if state == TransferRuntimeState::Active {
            self.remove_active_row(row);
            self.active_entries[row.0].take()
        } else {
            None
        };
        if state == TransferRuntimeState::Paused {
            self.remove_paused(row);
        }
        if state == TransferRuntimeState::Queued {
            self.remove_queued(row);
        }
        if state == TransferRuntimeState::Removed {
            return None;
        }

        self.states[row.0] = TransferRuntimeState::Removed;
        self.flags.pause_requested.set(row.0, false);
        self.specs[row.0] = None;
        self.resume_data[row.0] = None;
        Some(RemovedTransfer { active_entry })
    }

    fn clear(&mut self) {
        for entry in self.active_entries.iter_mut().filter_map(Option::take) {
            entry.handle.abort();
        }
        self.active_rows.clear();
        self.paused_rows.clear();
        self.queued_rows.clear();
        for state in &mut self.states {
            *state = TransferRuntimeState::Removed;
        }
        for spec in &mut self.specs {
            *spec = None;
        }
        for resume_data in &mut self.resume_data {
            *resume_data = None;
        }
    }

    fn prepare_queued(
        &mut self,
        id: TransferId,
        spec: DownloadSpec,
        resume_data: Option<ProviderResumeData>,
    ) -> TransferRow {
        let row = self.ensure_row(id);
        self.leave_current_list(row);
        self.states[row.0] = TransferRuntimeState::Queued;
        self.specs[row.0] = Some(spec);
        self.active_entries[row.0] = None;
        self.resume_data[row.0] = resume_data;
        self.flags.pause_requested.set(row.0, false);
        row
    }

    fn take_start(&mut self, row: TransferRow) -> Option<TransferStart> {
        let id = self.ids[row.0];
        self.states[row.0] = TransferRuntimeState::Removed;
        self.flags.pause_requested.set(row.0, false);
        Some(TransferStart {
            id,
            spec: self.specs[row.0].take()?,
            resume_data: self.resume_data[row.0].take(),
        })
    }

    fn ensure_row(&mut self, id: TransferId) -> TransferRow {
        if let Some(row) = self.row_for(id) {
            return row;
        }

        let row = TransferRow(self.ids.len());
        self.row_by_id.insert(id, row);
        self.ids.push(id);
        self.states.push(TransferRuntimeState::Removed);
        self.specs.push(None);
        self.active_entries.push(None);
        self.resume_data.push(None);
        self.flags.pause_requested.push(false);
        self.active_positions.push(NO_INDEX);
        self.paused_positions.push(NO_INDEX);
        row
    }

    fn row_for(&self, id: TransferId) -> Option<TransferRow> {
        self.row_by_id.get(&id).copied()
    }

    fn leave_current_list(&mut self, row: TransferRow) {
        match self.states[row.0] {
            TransferRuntimeState::Active => {
                self.remove_active_row(row);
                self.active_entries[row.0] = None;
            }
            TransferRuntimeState::Paused => self.remove_paused(row),
            TransferRuntimeState::Queued => self.remove_queued(row),
            TransferRuntimeState::Removed => {}
        }
    }

    fn remove_active_row(&mut self, row: TransferRow) {
        let pos = self.active_positions[row.0];
        if pos == NO_INDEX {
            return;
        }
        self.active_rows.swap_remove(pos);
        if let Some(&moved) = self.active_rows.get(pos) {
            self.active_positions[moved.0] = pos;
        }
        self.active_positions[row.0] = NO_INDEX;
    }

    fn remove_paused(&mut self, row: TransferRow) {
        let pos = self.paused_positions[row.0];
        if pos == NO_INDEX {
            return;
        }
        self.paused_rows.swap_remove(pos);
        if let Some(&moved) = self.paused_rows.get(pos) {
            self.paused_positions[moved.0] = pos;
        }
        self.paused_positions[row.0] = NO_INDEX;
    }

    fn remove_queued(&mut self, row: TransferRow) {
        if let Some(pos) = self.queued_rows.iter().position(|queued| *queued == row) {
            self.queued_rows.remove(pos);
        }
    }
}

struct RemovedTransfer {
    active_entry: Option<TaskEntry>,
}

pub(super) struct TransferRuntime {
    transfers: TransferTable,
    disk: DiskHandle,
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
        disk: DiskHandle,
    ) -> Self {
        let (runtime_update_tx, runtime_update_rx) =
            mpsc::channel::<TaskRuntimeUpdate>(TASK_RUNTIME_UPDATE_CAPACITY);
        let max_concurrent = config.max_concurrent_downloads;
        let global_throttle = Arc::new(TokenBucket::new(config.global_speed_limit_bps));
        Self {
            transfers: TransferTable::default(),
            disk,
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
                disk: self.disk.clone(),
                runtime_update_tx: self.runtime_update_tx.clone(),
            },
        );

        let chunk_map_state = spec.active_chunk_map_state();
        let control_support = spec.control_support();
        self.transfers.set_active(
            id,
            spec,
            TaskEntry {
                handle,
                pause_token,
                pause_sink,
                destination_sink,
                control_support,
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
        while self.transfers.active_len() < self.max_concurrent {
            let Some(next) = self.transfers.pop_next_queued() else {
                break;
            };
            tracing::info!(
                id = next.id.0,
                queued_remaining = self.transfers.queued_len(),
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
        if self.transfers.active_len() < self.max_concurrent {
            tracing::info!(id = id.0, url = spec.url(), "download starting");
            let _ = self.db_tx.send(DbEvent::Started { id });
            self.spawn_task(id, spec, None).await;
        } else {
            tracing::info!(
                id = id.0,
                url = spec.url(),
                queued = self.transfers.queued_len() + 1,
                "download queued (at capacity)"
            );
            self.transfers.set_queued_back(id, spec, None);
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

        if let Some(entry) = self.transfers.active_entry_mut(id) {
            tracing::info!(id = id.0, "pausing download");
            entry.pause_token.cancel();
            self.transfers.mark_pause_requested(id);
        } else if let Some(task) = self.transfers.take_queued(id) {
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
            self.transfers.set_paused(id, task.spec, task.resume_data);
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
        self.transfers
            .set_paused(download.id, download.spec, download.resume_data);
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

        if let Some(pt) = self.transfers.take_paused(id) {
            tracing::info!(id = id.0, "resuming download");
            if pt.resume_data.is_none() {
                self.disk
                    .remove_stale_part_for_fresh_resume(pt.spec.destination());
            }
            let (downloaded_bytes, total_bytes) = snapshot_totals(pt.resume_data.as_ref());
            if self.transfers.active_len() < self.max_concurrent {
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
                let _ = self.db_tx.send(DbEvent::Queued { id });
                self.emit(self.status_event(
                    id,
                    TransferStatus::Pending,
                    downloaded_bytes,
                    total_bytes,
                ))
                .await;
                self.transfers.set_queued_front(id, pt.spec, pt.resume_data);
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

        let removed = self.transfers.remove_any(id);
        if let Some(entry) = removed
            .as_ref()
            .and_then(|removed| removed.active_entry.as_ref())
        {
            tracing::info!(id = id.0, "download cancelled");
            entry.handle.abort();
            self.try_start_next().await;
        }

        if removed.is_some() {
            let artifact_state = self.disk.artifact_state(&destination);
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
        let removed = self.transfers.remove_any(id);
        let was_active = removed
            .as_ref()
            .and_then(|removed| removed.active_entry.as_ref())
            .map(|entry| {
                entry.handle.abort();
            });
        if was_active.is_some() {
            self.try_start_next().await;
        }

        if removed.is_some() {
            let _ = self.db_tx.send(DbEvent::Cancelled { id });
        }
        let artifact_state = self.disk.delete_artifacts(&resolved_destination);
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
            active = self.transfers.active_len(),
            paused = self.transfers.paused_len(),
            queued = self.transfers.queued_len(),
            "transfer runtime shutting down, aborting active tasks"
        );
        self.transfers.clear();
    }

    async fn handle_task_done(
        &mut self,
        id: TransferId,
        status: TransferStatus,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    ) {
        let active_entry = self.transfers.remove_active(id);
        if active_entry.is_none() && !self.transfers.contains_paused(id) {
            return;
        }
        let mut active_entry = active_entry;
        if let Some((entry, spec)) = active_entry.as_mut() {
            sync_runtime_destination(
                &self.db_tx,
                &mut self.events,
                id,
                spec,
                &entry.destination_sink,
            );
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
                if let Some((entry, spec)) = active_entry {
                    self.finish_active_pause(id, entry, spec).await;
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
                if self.transfers.contains_active(update.id)
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
                if !self.transfers.contains_active(id) {
                    return;
                }
                self.emit(TransferRuntimeEvent::TransferBytesWritten { id, bytes })
                    .await;
            }
            TaskRuntimeUpdate::DestinationChanged { id, destination } => {
                if !self.transfers.contains_active(id) {
                    return;
                }
                let Some(spec) = self.transfers.spec_mut(id) else {
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
            TaskRuntimeUpdate::ControlSupportChanged { id, support } => {
                let Some(entry) = self.transfers.active_entry_mut(id) else {
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
                if !self.transfers.contains_active(id) {
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

    async fn finish_active_pause(&mut self, id: TransferId, entry: TaskEntry, spec: DownloadSpec) {
        if !self.transfers.pause_requested(id) {
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
            self.transfers.set_paused(id, spec, Some(resume_data));
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
        if let Some(entry) = self.transfers.active_entry(id) {
            return Some((entry.control_support.can_pause, self.transfers.spec(id)?));
        }
        let spec = self.transfers.spec(id)?;
        Some((
            provider::supports_control_action(spec, TransferControlAction::Pause),
            spec,
        ))
    }

    fn resume_target_spec(&self, id: TransferId) -> Option<&DownloadSpec> {
        self.transfers
            .contains_paused(id)
            .then(|| self.transfers.spec(id))
            .flatten()
    }

    fn cancel_target(&self, id: TransferId) -> Option<(bool, PathBuf)> {
        if let Some(entry) = self.transfers.active_entry(id) {
            let spec = self.transfers.spec(id)?;
            return Some((
                entry.control_support.can_cancel,
                runtime_destination(spec, entry),
            ));
        }
        let spec = self.transfers.spec(id)?;
        Some((
            provider::supports_control_action(spec, TransferControlAction::Cancel),
            spec.destination().to_path_buf(),
        ))
    }

    fn known_destination(&self, id: TransferId) -> Option<PathBuf> {
        if let Some(entry) = self.transfers.active_entry(id) {
            let spec = self.transfers.spec(id)?;
            return Some(runtime_destination(spec, entry));
        }
        self.transfers
            .spec(id)
            .map(|spec| spec.destination().to_path_buf())
    }
}

fn runtime_destination(spec: &DownloadSpec, entry: &TaskEntry) -> PathBuf {
    provider::current_destination(&entry.destination_sink)
        .unwrap_or_else(|| spec.destination().to_path_buf())
}

fn sync_runtime_destination(
    db_tx: &std::sync::mpsc::Sender<DbEvent>,
    events: &mut Vec<TransferRuntimeEvent>,
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
    let _ = db_tx.send(DbEvent::DestinationChanged {
        id,
        destination: destination.clone(),
    });
    events.push(TransferRuntimeEvent::DestinationChanged { id, destination });
}

fn snapshot_totals(resume_data: Option<&ProviderResumeData>) -> (u64, Option<u64>) {
    match resume_data {
        Some(data) => (data.downloaded_bytes(), data.total_bytes()),
        None => (0, None),
    }
}
