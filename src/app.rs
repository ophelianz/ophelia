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

//! App download state
//!
//! Downloads owns the engine and the live transfer lists
//! A background task drains engine updates every 100ms and asks GPUI to redraw
//!
//! Startup sequence:
//!   1. Load saved settings
//!   2. Start SQLite state, DB worker, and history reader
//!   3. Start the local IPC server
//!   4. Create DownloadEngine with the next free download id
//!   5. Restore saved downloads into the engine and app lists

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use gpui::{Context, SharedString};

use crate::engine::state::{self, HistoryReader};
use crate::engine::{
    AddDownloadRequest, ArtifactState, DownloadControlAction, DownloadEngine, DownloadId,
    DownloadSpec, DownloadStatus, EngineNotification, HistoryFilter, HistoryRow, ProgressUpdate,
    RestoredDownload, SavedDownload, TransferChunkMapState, TransferControlSupport,
};
use crate::ipc::IpcServer;
use crate::settings::Settings;
use crate::views::overlays::notification::NotificationKind;

/// Live downloads stored as parallel vecs
/// Every vec uses the same row index
pub struct Downloads {
    engine: DownloadEngine,
    _db_worker: state::DbWorkerHandle,
    ipc: IpcServer,
    pub settings: Settings,

    pub ids: Vec<DownloadId>,
    row_by_id: HashMap<DownloadId, usize>,
    /// Source kind per live transfer
    /// Kept app-side so future views do not need engine access
    pub provider_kinds: Vec<SharedString>,
    /// Source label per live transfer
    pub source_labels: Vec<SharedString>,
    pub filenames: Vec<SharedString>,
    pub destinations: Vec<SharedString>,
    pub statuses: Vec<DownloadStatus>,
    /// Controls this transfer supports
    pub control_supports: Vec<TransferControlSupport>,
    pub transfer_chunk_maps: Vec<TransferChunkMapState>,
    pub downloaded_bytes: Vec<u64>,
    pub total_bytes: Vec<Option<u64>>,
    pub speeds: Vec<u64>,

    /// Rolling ~60-second download speed history
    pub speed_history: VecDeque<u64>,
    write_sampler: DownloadWriteSampler,
    poll_ticks: u8,

    history_reader: HistoryReader,
    pub history: Vec<HistoryRow>,
    pub history_filter: HistoryFilter,

    #[cfg(test)]
    _test_db_dir: Option<tempfile::TempDir>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDisplayState {
    Active,
    Paused,
    Queued,
    Finished,
    Error,
}

impl TransferDisplayState {
    fn from_status(status: DownloadStatus) -> Self {
        match status {
            DownloadStatus::Downloading => Self::Active,
            DownloadStatus::Paused => Self::Paused,
            DownloadStatus::Finished => Self::Finished,
            DownloadStatus::Error | DownloadStatus::Cancelled => Self::Error,
            DownloadStatus::Pending => Self::Queued,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferAvailableActions {
    pub pause: bool,
    pub resume: bool,
    pub cancel: bool,
    pub delete_artifact: bool,
}

impl TransferAvailableActions {
    pub fn from_status_and_support(
        status: DownloadStatus,
        support: TransferControlSupport,
    ) -> Self {
        Self {
            pause: matches!(
                status,
                DownloadStatus::Pending | DownloadStatus::Downloading
            ) && support.can_pause,
            resume: matches!(status, DownloadStatus::Paused) && support.can_resume,
            cancel: matches!(
                status,
                DownloadStatus::Pending | DownloadStatus::Downloading | DownloadStatus::Paused
            ) && support.can_cancel,
            delete_artifact: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransferListRow {
    pub id: DownloadId,
    #[allow(dead_code)]
    // kept app-facing so future provider filters/badges do not need engine access
    pub provider_kind: SharedString,
    #[allow(dead_code)]
    // kept app-facing so future views can render source labels directly
    pub source_label: SharedString,
    pub filename: SharedString,
    pub destination: SharedString,
    pub status: DownloadStatus,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub progress: f32,
    #[allow(dead_code)] // retained for future richer transfer rows and stats drill-downs.
    pub speed_bps: u64,
    pub display_state: TransferDisplayState,
    pub available_actions: TransferAvailableActions,
}

impl TransferListRow {
    fn from_downloads(downloads: &Downloads, index: usize) -> Self {
        let status = downloads.statuses[index];
        let progress = match downloads.total_bytes[index] {
            Some(total) if total > 0 => downloads.downloaded_bytes[index] as f32 / total as f32,
            _ => 0.0,
        };

        Self {
            id: downloads.ids[index],
            provider_kind: downloads.provider_kinds[index].clone(),
            source_label: downloads.source_labels[index].clone(),
            filename: downloads.filenames[index].clone(),
            destination: downloads.destinations[index].clone(),
            status,
            downloaded_bytes: downloads.downloaded_bytes[index],
            total_bytes: downloads.total_bytes[index],
            progress,
            speed_bps: downloads.speeds[index],
            display_state: TransferDisplayState::from_status(status),
            available_actions: TransferAvailableActions::from_status_and_support(
                status,
                downloads.control_supports[index],
            ),
        }
    }

    #[allow(dead_code)] // the current Transfers row still prioritizes destination path as its subtitle
    pub fn source_summary(&self) -> SharedString {
        source_summary(&self.provider_kind, &self.source_label).into()
    }
}

#[derive(Debug, Clone)]
pub struct HistoryListRow {
    pub id: DownloadId,
    pub provider_kind: SharedString,
    pub source_label: SharedString,
    pub filename: SharedString,
    #[allow(dead_code)] // retained for future history row actions like reveal/copy destination
    pub destination: SharedString,
    pub status: DownloadStatus,
    pub artifact_state: ArtifactState,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub added_at: i64,
    pub finished_at: Option<i64>,
}

impl HistoryListRow {
    fn from_history_row(row: &HistoryRow) -> Self {
        Self {
            id: row.id,
            provider_kind: row.provider_kind.clone().into(),
            source_label: row.source_label.clone().into(),
            filename: row.filename().to_string().into(),
            destination: row.destination.clone().into(),
            status: row.status,
            artifact_state: row.artifact_state,
            downloaded_bytes: row.downloaded_bytes,
            total_bytes: row.total_bytes,
            added_at: row.added_at,
            finished_at: row.finished_at,
        }
    }

    pub fn source_summary(&self) -> SharedString {
        source_summary(&self.provider_kind, &self.source_label).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SidebarStorageSummary {
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub fraction: f32,
}

impl SidebarStorageSummary {
    fn from_usage(used_bytes: u64, total_bytes: u64) -> Self {
        Self {
            used_bytes,
            total_bytes,
            fraction: if total_bytes > 0 {
                (used_bytes as f32 / total_bytes as f32).clamp(0.0, 1.0)
            } else {
                0.0
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DownloadWriteSample {
    at: Instant,
    total_bytes: u64,
}

#[derive(Debug, Default)]
struct DownloadWriteSampler {
    last_sample: Option<DownloadWriteSample>,
    total_bytes: u64,
    write_speed_bps: u64,
}

impl DownloadWriteSampler {
    fn record(&mut self, bytes: u64) {
        self.total_bytes = self.total_bytes.saturating_add(bytes);
    }

    fn sample_now(&mut self, now: Instant) {
        let Some(previous) = self.last_sample else {
            self.last_sample = Some(DownloadWriteSample {
                at: now,
                total_bytes: self.total_bytes,
            });
            self.write_speed_bps = 0;
            return;
        };

        let elapsed = now.saturating_duration_since(previous.at).as_secs_f64();
        self.last_sample = Some(DownloadWriteSample {
            at: now,
            total_bytes: self.total_bytes,
        });
        if elapsed <= 0.0 {
            return;
        }

        self.write_speed_bps =
            (self.total_bytes.saturating_sub(previous.total_bytes) as f64 / elapsed).round() as u64;
    }

    fn write_speed_bps(&self) -> u64 {
        self.write_speed_bps
    }
}

impl Downloads {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let settings = Settings::load();
        let bootstrap = state::bootstrap().expect("failed to bootstrap backend state");
        let ipc = IpcServer::start(settings.ipc_port);
        Self::from_bootstrap(settings, bootstrap, ipc, cx)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(cx: &mut Context<Self>) -> Self {
        let db_dir = tempfile::tempdir().expect("failed to create test database directory");
        let db_path = db_dir.path().join("downloads.db");
        let bootstrap =
            state::bootstrap_at(&db_path).expect("failed to bootstrap test backend state");
        let mut model =
            Self::from_bootstrap(Settings::default(), bootstrap, IpcServer::disabled(), cx);
        model._test_db_dir = Some(db_dir);
        model
    }

    fn from_bootstrap(
        settings: Settings,
        bootstrap: state::StateBootstrap,
        ipc: IpcServer,
        cx: &mut Context<Self>,
    ) -> Self {
        let state::StateBootstrap {
            db_tx,
            history_reader,
            saved_downloads,
            next_download_id,
            worker,
        } = bootstrap;
        let engine = DownloadEngine::new(settings.clone(), db_tx, next_download_id);

        let mut model = Self {
            engine,
            _db_worker: worker,
            ipc,
            settings,
            ids: Vec::new(),
            row_by_id: HashMap::new(),
            provider_kinds: Vec::new(),
            source_labels: Vec::new(),
            filenames: Vec::new(),
            destinations: Vec::new(),
            statuses: Vec::new(),
            control_supports: Vec::new(),
            transfer_chunk_maps: Vec::new(),
            downloaded_bytes: Vec::new(),
            total_bytes: Vec::new(),
            speeds: Vec::new(),
            speed_history: VecDeque::new(),
            write_sampler: DownloadWriteSampler::default(),
            poll_ticks: 0,
            history_reader,
            history: Vec::new(),
            history_filter: HistoryFilter::All,
            #[cfg(test)]
            _test_db_dir: None,
        };

        for saved_dl in &saved_downloads {
            model
                .engine
                .restore(RestoredDownload::from_saved(saved_dl, &model.settings));
            model.push_saved(saved_dl);
        }

        model.refresh_history(cx);

        cx.spawn(async |this, cx: &mut gpui::AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                cx.update(|app| {
                    this.update(app, |model, cx| {
                        while let Some(update) = model.engine.poll_progress() {
                            model.apply_progress(update, cx);
                        }
                        while let Some(notification) = model.engine.poll_notification() {
                            model.apply_notification(notification, cx);
                        }
                        while let Some(req) = model.ipc.try_recv() {
                            model.add_request(req, cx);
                        }
                        model.tick_metrics();
                    })
                    .ok();
                });
            }
        })
        .detach();

        model
    }

    pub fn add(
        &mut self,
        url: String,
        destination: PathBuf,
        cx: &mut Context<Self>,
    ) -> Option<DownloadId> {
        let display_name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("download")
            .to_string();
        let spec = match DownloadSpec::from_user_input(url, destination, &self.settings) {
            Ok(spec) => spec,
            Err(error) => {
                self.report_add_failure(display_name, error, cx);
                return None;
            }
        };
        Some(self.add_spec(spec, AddOrigin::UserInput, cx))
    }

    pub fn add_request(
        &mut self,
        request: AddDownloadRequest,
        cx: &mut Context<Self>,
    ) -> Option<DownloadId> {
        let display_name = request.display_filename_hint();
        let spec = match request.into_spec(&self.settings) {
            Ok(spec) => spec,
            Err(error) => {
                self.report_add_failure(display_name, error, cx);
                return None;
            }
        };
        Some(self.add_spec(spec, AddOrigin::IpcRequest, cx))
    }

    fn add_spec(
        &mut self,
        spec: DownloadSpec,
        origin: AddOrigin,
        cx: &mut Context<Self>,
    ) -> DownloadId {
        let filename = notification_filename(spec.destination());
        let id = self.push_spec(spec, cx);
        if let Some(kind) = start_notification_kind(origin) {
            self.show_notification(cx, filename, kind);
        }
        id
    }

    fn push_spec(&mut self, spec: DownloadSpec, cx: &mut Context<Self>) -> DownloadId {
        let destination = spec.destination().to_path_buf();
        let filename = notification_filename(&destination);
        let dest_str: SharedString = destination.to_string_lossy().to_string().into();

        let provider_kind: SharedString = spec.provider_kind().to_string().into();
        let source_label: SharedString = spec.source_label().to_string().into();
        let control_support = spec.control_support();
        let id = self.engine.add(spec);
        self.ids.push(id);
        self.provider_kinds.push(provider_kind);
        self.source_labels.push(source_label);
        self.filenames.push(filename);
        self.destinations.push(dest_str);
        self.statuses.push(DownloadStatus::Pending);
        self.control_supports.push(control_support);
        self.transfer_chunk_maps
            .push(TransferChunkMapState::Unsupported);
        self.downloaded_bytes.push(0);
        self.total_bytes.push(None);
        self.speeds.push(0);
        self.row_by_id.insert(id, self.ids.len() - 1);
        cx.notify();
        id
    }

    fn report_add_failure(
        &self,
        display_name: impl Into<SharedString>,
        error: std::io::Error,
        cx: &mut Context<Self>,
    ) {
        tracing::warn!("failed to resolve download destination: {error}");
        self.show_notification(cx, display_name.into(), NotificationKind::Error);
    }

    pub fn pause(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        self.engine.pause(id);
        cx.notify();
    }

    pub fn apply_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        if settings.ipc_port != self.settings.ipc_port {
            self.ipc = IpcServer::start(settings.ipc_port);
        }
        self.engine.update_settings(settings.clone());
        self.settings = settings;
        cx.notify();
    }

    pub fn resume(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        self.engine.resume(id);
        cx.notify();
    }

    /// Cancel a live transfer without deleting bytes already on disk
    #[allow(dead_code)] // reserved for a future UI with a distinct cancel-transfer action.
    pub fn cancel_transfer(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        self.engine.cancel(id);
        cx.notify();
    }

    /// Delete the file for a live transfer
    /// History is kept separately
    pub fn delete_artifact(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        let Some(idx) = self.index_of(id) else {
            return;
        };
        let destination = PathBuf::from(self.destinations[idx].as_ref());
        self.engine.delete_artifact(id, destination);
        cx.notify();
    }

    /// Removing a live transfer deletes the file but keeps history
    pub fn remove(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        self.delete_artifact(id, cx);
    }

    /// Open the folder containing a transfer's destination
    pub fn open_destination_folder(&mut self, id: DownloadId, _cx: &mut Context<Self>) {
        let Some(idx) = self.index_of(id) else {
            return;
        };

        let destination = PathBuf::from(self.destinations[idx].as_ref());
        let target = if destination.is_dir() {
            destination
        } else {
            destination
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or(destination)
        };

        if let Err(error) = open_in_file_manager(&target) {
            tracing::warn!(
                path = %target.display(),
                "failed to open transfer destination in file manager: {error}"
            );
        }
    }

    fn tick_metrics(&mut self) {
        self.poll_ticks = self.poll_ticks.wrapping_add(1);
        if self.poll_ticks % 10 == 0 {
            let total: u64 = self.speeds.iter().sum();
            if self.speed_history.len() >= 60 {
                self.speed_history.pop_front();
            }
            self.speed_history.push_back(total);
            self.write_sampler.sample_now(Instant::now());
        }
    }

    /// Current total download speed across active tasks, in bytes/s
    pub fn download_speed_bps(&self) -> u64 {
        self.speeds.iter().sum()
    }

    /// Speed history as MB/s floats, oldest first, ready to hand to StatsBar
    pub fn speed_samples_mbs(&self) -> Vec<f32> {
        self.speed_history
            .iter()
            .map(|&s| s as f32 / 1_000_000.0)
            .collect()
    }

    /// Current download-file read rate
    /// Nothing reports file reads yet, so keep the UI slot visible but honest
    pub fn disk_read_speed_bps(&self) -> Option<u64> {
        Some(0)
    }

    /// Current rate of bytes successfully written by download tasks
    pub fn disk_write_speed_bps(&self) -> Option<u64> {
        Some(self.write_sampler.write_speed_bps())
    }

    /// active, finished, queued counts
    pub fn status_counts(&self) -> (usize, usize, usize) {
        let active = self
            .statuses
            .iter()
            .filter(|&&s| s == DownloadStatus::Downloading)
            .count();
        let finished = self
            .statuses
            .iter()
            .filter(|&&s| s == DownloadStatus::Finished)
            .count();
        let queued = self
            .statuses
            .iter()
            .filter(|&&s| matches!(s, DownloadStatus::Pending | DownloadStatus::Paused))
            .count();
        (active, finished, queued)
    }

    pub fn transfer_rows(&self) -> Vec<TransferListRow> {
        (0..self.len())
            .map(|index| TransferListRow::from_downloads(self, index))
            .collect()
    }

    pub fn history_rows(&self) -> Vec<HistoryListRow> {
        self.history
            .iter()
            .map(HistoryListRow::from_history_row)
            .collect()
    }

    #[allow(dead_code)] // reserved for the Transfers chunk bitmap card once the frontend consumes it.
    pub fn transfer_chunk_map_state(&self, id: DownloadId) -> TransferChunkMapState {
        transfer_chunk_map_state_or_unsupported(&self.transfer_chunk_maps, self.index_of(id))
    }

    pub fn storage_summary(&self) -> SidebarStorageSummary {
        storage_summary_for_path(&self.settings.download_dir())
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Reload history and redraw the UI
    pub fn refresh_history(&mut self, cx: &mut Context<Self>) {
        match self.history_reader.load(self.history_filter, "") {
            Ok(rows) => {
                self.history = rows;
                cx.notify();
            }
            Err(e) => tracing::warn!("history query failed: {e}"),
        }
    }

    pub fn set_history_filter(&mut self, filter: HistoryFilter, cx: &mut Context<Self>) {
        self.history_filter = filter;
        self.refresh_history(cx);
    }

    /// Push a restored download into the app lists without going through the engine
    fn push_saved(&mut self, saved: &SavedDownload) {
        let filename: SharedString = saved
            .destination
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string()
            .into();
        let dest_str: SharedString = saved.destination.to_string_lossy().to_string().into();
        let provider_kind: SharedString = saved.source.kind().to_string().into();
        let source_label: SharedString = saved.source.display_label().to_string().into();

        self.ids.push(saved.id);
        self.provider_kinds.push(provider_kind);
        self.source_labels.push(source_label);
        self.filenames.push(filename);
        self.destinations.push(dest_str);
        self.statuses.push(DownloadStatus::Paused);
        self.control_supports.push(saved.source.control_support());
        self.transfer_chunk_maps
            .push(TransferChunkMapState::Unsupported);
        self.downloaded_bytes.push(saved.downloaded_bytes);
        self.total_bytes.push(saved.total_bytes);
        self.speeds.push(0);
        self.row_by_id.insert(saved.id, self.ids.len() - 1);
    }

    fn apply_progress(&mut self, update: ProgressUpdate, cx: &mut Context<Self>) {
        if let Some(idx) = self.index_of(update.id) {
            let prev = self.statuses[idx];
            self.statuses[idx] = update.status;
            self.downloaded_bytes[idx] = update.downloaded_bytes;
            self.total_bytes[idx] = update.total_bytes;
            self.speeds[idx] = update.speed_bytes_per_sec;

            // Show notification on first final status
            if let Some(kind) = terminal_notification_kind(prev, update.status) {
                let filename = self.filenames[idx].clone();
                self.show_notification(cx, filename, kind);
                self.refresh_history(cx);
            }

            cx.notify();
        }
    }

    fn apply_notification(&mut self, notification: EngineNotification, cx: &mut Context<Self>) {
        match notification {
            EngineNotification::Update(update) => self.apply_progress(update, cx),
            EngineNotification::DownloadBytesWritten { id, bytes } => {
                if self.index_of(id).is_some() {
                    self.write_sampler.record(bytes);
                }
            }
            EngineNotification::DestinationChanged { id, destination } => {
                if let Some(idx) = self.index_of(id) {
                    self.filenames[idx] = destination
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("unknown")
                        .to_string()
                        .into();
                    self.destinations[idx] = destination.to_string_lossy().to_string().into();
                    cx.notify();
                }
            }
            EngineNotification::ControlSupportChanged { id, support } => {
                if let Some(idx) = self.index_of(id) {
                    self.control_supports[idx] = support;
                    cx.notify();
                }
            }
            EngineNotification::ChunkMapStateChanged { id, state } => {
                if let Some(idx) = self.index_of(id) {
                    self.transfer_chunk_maps[idx] = state;
                    cx.notify();
                }
            }
            EngineNotification::LiveTransferRemoved {
                id,
                action,
                artifact_state,
            } => {
                tracing::debug!(
                    id = id.0,
                    action = live_transfer_removal_action_name(action),
                    artifact_state = artifact_state_name(artifact_state),
                    "live transfer left the active surface"
                );
                self.remove_row(id, cx);
                self.refresh_history(cx);
            }
            EngineNotification::ControlUnsupported { id, action } => {
                tracing::warn!(
                    id = id.0,
                    action = control_action_name(action),
                    "provider does not support requested control action"
                );
            }
        }
    }

    fn remove_row(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        if let Some(idx) = self.index_of(id) {
            self.ids.remove(idx);
            self.row_by_id.remove(&id);
            self.provider_kinds.remove(idx);
            self.source_labels.remove(idx);
            self.filenames.remove(idx);
            self.destinations.remove(idx);
            self.statuses.remove(idx);
            self.control_supports.remove(idx);
            self.transfer_chunk_maps.remove(idx);
            self.downloaded_bytes.remove(idx);
            self.total_bytes.remove(idx);
            self.speeds.remove(idx);
            self.rebuild_row_index();
            cx.notify();
        }
    }

    fn index_of(&self, id: DownloadId) -> Option<usize> {
        self.row_by_id.get(&id).copied()
    }

    fn rebuild_row_index(&mut self) {
        self.row_by_id = build_row_index(&self.ids);
    }

    fn show_notification(
        &self,
        cx: &mut Context<Self>,
        filename: impl Into<SharedString>,
        kind: NotificationKind,
    ) {
        show_popup_notification(&self.settings, cx, filename.into(), kind);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddOrigin {
    UserInput,
    IpcRequest,
}

fn control_action_name(action: DownloadControlAction) -> &'static str {
    match action {
        DownloadControlAction::Pause => "pause",
        DownloadControlAction::Resume => "resume",
        DownloadControlAction::Cancel => "cancel",
        DownloadControlAction::Restore => "restore",
    }
}

fn live_transfer_removal_action_name(
    action: crate::engine::LiveTransferRemovalAction,
) -> &'static str {
    match action {
        crate::engine::LiveTransferRemovalAction::Cancelled => "cancelled",
        crate::engine::LiveTransferRemovalAction::DeleteArtifact => "delete_artifact",
    }
}

fn artifact_state_name(state: crate::engine::ArtifactState) -> &'static str {
    match state {
        crate::engine::ArtifactState::Present => "present",
        crate::engine::ArtifactState::Deleted => "deleted",
        crate::engine::ArtifactState::Missing => "missing",
    }
}

fn notification_filename(destination: &Path) -> SharedString {
    destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string()
        .into()
}

fn open_in_file_manager(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn().map(|_| ())
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer").arg(path).spawn().map(|_| ())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(path).spawn().map(|_| ())
    }
}

fn show_popup_notification(
    settings: &Settings,
    cx: &mut gpui::App,
    filename: SharedString,
    kind: NotificationKind,
) {
    if settings.notifications_enabled {
        crate::views::overlays::notification::show(cx, filename, kind);
    }
}

fn start_notification_kind(origin: AddOrigin) -> Option<NotificationKind> {
    match origin {
        AddOrigin::UserInput => None,
        AddOrigin::IpcRequest => Some(NotificationKind::Started),
    }
}

fn terminal_notification_kind(
    previous: DownloadStatus,
    next: DownloadStatus,
) -> Option<NotificationKind> {
    let was_terminal = matches!(previous, DownloadStatus::Finished | DownloadStatus::Error);
    if was_terminal {
        return None;
    }

    match next {
        DownloadStatus::Finished => Some(NotificationKind::Success),
        DownloadStatus::Error => Some(NotificationKind::Error),
        _ => None,
    }
}

fn source_summary(provider_kind: &str, source_label: &str) -> String {
    if provider_kind == "http" {
        source_label.to_string()
    } else {
        format!("{provider_kind}: {source_label}")
    }
}

fn storage_summary_for_path(path: &Path) -> SidebarStorageSummary {
    let (used_bytes, total_bytes) = query_disk(path);
    SidebarStorageSummary::from_usage(used_bytes, total_bytes)
}

fn build_row_index(ids: &[DownloadId]) -> HashMap<DownloadId, usize> {
    ids.iter()
        .copied()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect()
}

#[cfg_attr(not(test), allow(dead_code))]
fn transfer_chunk_map_state_or_unsupported(
    states: &[TransferChunkMapState],
    index: Option<usize>,
) -> TransferChunkMapState {
    index
        .and_then(|idx| states.get(idx).cloned())
        .unwrap_or(TransferChunkMapState::Unsupported)
}

fn query_disk(path: &Path) -> (u64, u64) {
    use std::ffi::CString;

    let Ok(cpath) = CString::new(path.to_string_lossy().as_bytes()) else {
        return (0, 0);
    };
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(cpath.as_ptr(), &mut stat) } != 0 {
        return (0, 0);
    }
    let block = stat.f_frsize as u64;
    let total = block * stat.f_blocks as u64;
    let avail = block * stat.f_bavail as u64;
    (total.saturating_sub(avail), total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestApp;

    #[test]
    fn transfer_available_actions_follow_status_and_capabilities() {
        let support = TransferControlSupport {
            can_pause: true,
            can_resume: true,
            can_cancel: true,
            can_restore: false,
        };

        assert_eq!(
            TransferAvailableActions::from_status_and_support(DownloadStatus::Downloading, support,),
            TransferAvailableActions {
                pause: true,
                resume: false,
                cancel: true,
                delete_artifact: true,
            }
        );

        assert_eq!(
            TransferAvailableActions::from_status_and_support(DownloadStatus::Paused, support),
            TransferAvailableActions {
                pause: false,
                resume: true,
                cancel: true,
                delete_artifact: true,
            }
        );

        assert_eq!(
            TransferAvailableActions::from_status_and_support(DownloadStatus::Finished, support),
            TransferAvailableActions {
                pause: false,
                resume: false,
                cancel: false,
                delete_artifact: true,
            }
        );
    }

    #[test]
    fn history_row_model_preserves_provider_and_artifact_semantics() {
        let row = HistoryRow {
            id: DownloadId(7),
            provider_kind: "soulseek".into(),
            source_label: "artist/track.flac".into(),
            destination: "/tmp/Music/track.flac".into(),
            status: DownloadStatus::Cancelled,
            artifact_state: ArtifactState::Missing,
            total_bytes: Some(1024),
            downloaded_bytes: 512,
            added_at: 10,
            finished_at: Some(20),
        };

        let model = HistoryListRow::from_history_row(&row);

        assert_eq!(model.id, DownloadId(7));
        assert_eq!(model.filename.as_ref(), "track.flac");
        assert_eq!(
            model.source_summary().as_ref(),
            "soulseek: artist/track.flac"
        );
        assert_eq!(model.artifact_state, ArtifactState::Missing);
        assert_eq!(model.status, DownloadStatus::Cancelled);
    }

    #[test]
    fn sidebar_storage_summary_clamps_fraction() {
        let empty = SidebarStorageSummary::from_usage(0, 0);
        assert_eq!(empty.fraction, 0.0);

        let summary = SidebarStorageSummary::from_usage(750, 1000);
        assert_eq!(summary.used_bytes, 750);
        assert_eq!(summary.total_bytes, 1000);
        assert_eq!(summary.fraction, 0.75);
    }

    #[test]
    fn source_summary_omits_http_prefix() {
        assert_eq!(
            source_summary("http", "https://example.com/file.zip"),
            "https://example.com/file.zip"
        );
        assert_eq!(
            source_summary("ftp", "ftp://example.com/file.zip"),
            "ftp: ftp://example.com/file.zip"
        );
    }

    #[test]
    fn ipc_requests_show_a_start_notification_but_manual_adds_do_not() {
        assert_eq!(start_notification_kind(AddOrigin::UserInput), None);
        assert_eq!(
            start_notification_kind(AddOrigin::IpcRequest),
            Some(NotificationKind::Started)
        );
    }

    #[test]
    fn terminal_notification_kind_only_emits_on_first_terminal_transition() {
        assert_eq!(
            terminal_notification_kind(DownloadStatus::Downloading, DownloadStatus::Finished),
            Some(NotificationKind::Success)
        );
        assert_eq!(
            terminal_notification_kind(DownloadStatus::Downloading, DownloadStatus::Error),
            Some(NotificationKind::Error)
        );
        assert_eq!(
            terminal_notification_kind(DownloadStatus::Finished, DownloadStatus::Finished),
            None
        );
        assert_eq!(
            terminal_notification_kind(DownloadStatus::Error, DownloadStatus::Error),
            None
        );
        assert_eq!(
            terminal_notification_kind(DownloadStatus::Paused, DownloadStatus::Paused),
            None
        );
    }

    #[test]
    fn notification_filename_uses_the_resolved_destination_name() {
        assert_eq!(
            notification_filename(Path::new("/tmp/downloads/browser-name.mp4")).as_ref(),
            "browser-name.mp4"
        );
    }

    #[test]
    fn popup_notifications_respect_the_global_settings_switch() {
        let mut app = TestApp::new();
        let mut enabled_settings = Settings::default();
        enabled_settings.notifications_enabled = true;

        app.update(|cx| {
            show_popup_notification(
                &enabled_settings,
                cx,
                "file.mp4".into(),
                NotificationKind::Started,
            );
        });
        assert_eq!(app.windows().len(), 1);

        let mut disabled_settings = Settings::default();
        disabled_settings.notifications_enabled = false;
        let mut app = TestApp::new();
        app.update(|cx| {
            show_popup_notification(
                &disabled_settings,
                cx,
                "file.mp4".into(),
                NotificationKind::Error,
            );
        });
        assert_eq!(app.windows().len(), 0);
    }

    #[test]
    fn build_row_index_tracks_current_positions() {
        let map = build_row_index(&[DownloadId(7), DownloadId(11), DownloadId(3)]);

        assert_eq!(map.get(&DownloadId(7)), Some(&0));
        assert_eq!(map.get(&DownloadId(11)), Some(&1));
        assert_eq!(map.get(&DownloadId(3)), Some(&2));
    }

    #[test]
    fn transfer_chunk_map_state_defaults_to_unsupported_for_missing_rows() {
        let states = vec![TransferChunkMapState::Loading];

        assert_eq!(
            transfer_chunk_map_state_or_unsupported(&states, None),
            TransferChunkMapState::Unsupported
        );
    }

    #[test]
    fn transfer_chunk_map_state_returns_stored_state_for_present_rows() {
        let states = vec![
            TransferChunkMapState::Unsupported,
            TransferChunkMapState::Loading,
        ];

        assert_eq!(
            transfer_chunk_map_state_or_unsupported(&states, Some(1)),
            TransferChunkMapState::Loading
        );
    }

    #[test]
    fn download_write_sampler_establishes_zero_baseline_on_first_sample() {
        let mut sampler = DownloadWriteSampler::default();
        let now = Instant::now();

        sampler.record(256);
        sampler.sample_now(now);

        assert_eq!(sampler.write_speed_bps(), 0);
    }

    #[test]
    fn download_write_sampler_computes_write_delta() {
        let mut sampler = DownloadWriteSampler::default();
        let start = Instant::now();

        sampler.record(2_000);
        sampler.sample_now(start);
        sampler.record(6_000);
        sampler.sample_now(start + Duration::from_secs(2));

        assert_eq!(sampler.write_speed_bps(), 3_000);
    }

    #[test]
    fn download_write_sampler_reports_zero_when_no_new_writes_arrive() {
        let mut sampler = DownloadWriteSampler::default();
        let start = Instant::now();

        sampler.record(32);
        sampler.sample_now(start);
        sampler.sample_now(start + Duration::from_secs(1));

        assert_eq!(sampler.write_speed_bps(), 0);
    }

    #[test]
    fn download_write_sampler_saturates_total_bytes() {
        let mut sampler = DownloadWriteSampler::default();
        let now = Instant::now();

        sampler.record(u64::MAX);
        sampler.sample_now(now);
        sampler.record(1);
        sampler.sample_now(now + Duration::from_secs(1));

        assert_eq!(sampler.write_speed_bps(), 0);
    }
}
