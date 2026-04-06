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

//! Application-level download state.
//!
//! Downloads is a GPUI entity that owns the DownloadEngine and the live download
//! data in SoA layout. A background task drains engine progress updates every 100ms
//! and calls cx.notify() to trigger a re-render.
//!
//! Startup sequence:
//!   1. Load persisted settings.
//!   2. Bootstrap backend state (SQLite restore data, DB worker, history reader).
//!   3. Create the app-owned IPC ingress server.
//!   4. Create DownloadEngine with db_tx and initial_next_id > DB max.
//!   5. Restore saved downloads into the engine's paused map and SoA vecs.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{Context, SharedString};

use crate::engine::state::{self, HistoryReader};
use crate::engine::{
    AddDownloadRequest, ArtifactState, DownloadControlAction, DownloadEngine, DownloadId,
    DownloadSpec, DownloadStatus, EngineNotification, HistoryFilter, HistoryRow, ProgressUpdate,
    RestoredDownload, SavedDownload, TransferControlSupport,
};
use crate::ipc::IpcServer;
use crate::settings::Settings;

/// All live download state in SoA layout.
/// One vec per field, all vecs share the same index space.
pub struct Downloads {
    engine: DownloadEngine,
    ipc: IpcServer,
    pub settings: Settings,

    pub ids: Vec<DownloadId>,
    row_by_id: HashMap<DownloadId, usize>,
    /// Provider identifier per live transfer, kept app-side so future workflow
    /// views do not have to infer it from engine internals.
    pub provider_kinds: Vec<SharedString>,
    /// User-facing source label per live transfer (URL today, richer labels later).
    pub source_labels: Vec<SharedString>,
    pub filenames: Vec<SharedString>,
    pub destinations: Vec<SharedString>,
    pub statuses: Vec<DownloadStatus>,
    /// Provider-declared lifecycle controls for each live transfer.
    pub control_supports: Vec<TransferControlSupport>,
    pub downloaded_bytes: Vec<u64>,
    pub total_bytes: Vec<Option<u64>>,
    pub speeds: Vec<u64>,

    /// Rolling ~60-second download speed history (one sample per second).
    pub speed_history: VecDeque<u64>,
    poll_ticks: u8,

    history_reader: HistoryReader,
    pub history: Vec<HistoryRow>,
    pub history_filter: HistoryFilter,
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
    // kept app-facing so future provider-aware surfaces can render source labels directly
    pub source_label: SharedString,
    pub filename: SharedString,
    pub destination: SharedString,
    pub status: DownloadStatus,
    pub progress: f32,
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

impl Downloads {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let settings = Settings::load();
        let bootstrap = state::bootstrap().expect("failed to bootstrap backend state");
        let history_reader = bootstrap.history_reader;
        let engine = DownloadEngine::new(
            settings.clone(),
            bootstrap.db_tx,
            bootstrap.next_download_id,
        );
        let ipc = IpcServer::start(settings.ipc_port);

        let mut model = Self {
            engine,
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
            downloaded_bytes: Vec::new(),
            total_bytes: Vec::new(),
            speeds: Vec::new(),
            speed_history: VecDeque::new(),
            poll_ticks: 0,
            history_reader,
            history: Vec::new(),
            history_filter: HistoryFilter::All,
        };

        for saved_dl in &bootstrap.saved_downloads {
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
                        model.tick_speed();
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
        Some(self.push_spec(spec, cx))
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
        Some(self.push_spec(spec, cx))
    }

    fn push_spec(&mut self, spec: DownloadSpec, cx: &mut Context<Self>) -> DownloadId {
        let destination = spec.destination().to_path_buf();
        let filename: SharedString = destination
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string()
            .into();
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
        crate::views::overlays::notification::show(
            cx,
            display_name.into(),
            crate::views::overlays::notification::NotificationKind::Error,
        );
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

    /// Cancel a live transfer without deleting any bytes already on disk.
    #[allow(dead_code)] // reserved for a future UI with a distinct cancel-transfer action.
    pub fn cancel_transfer(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        self.engine.cancel(id);
        cx.notify();
    }

    /// Delete the artifact for a live transfer. History is preserved separately.
    pub fn delete_artifact(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        let Some(idx) = self.index_of(id) else {
            return;
        };
        let destination = PathBuf::from(self.destinations[idx].as_ref());
        self.engine.delete_artifact(id, destination);
        cx.notify();
    }

    /// Current live-transfer remove semantics are delete-the-artifact and let the
    /// history surface retain the persisted record.
    pub fn remove(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        self.delete_artifact(id, cx);
    }

    /// Samples total speed once per second (every 10 × 100 ms polls).
    fn tick_speed(&mut self) {
        self.poll_ticks = self.poll_ticks.wrapping_add(1);
        if self.poll_ticks % 10 == 0 {
            let total: u64 = self.speeds.iter().sum();
            if self.speed_history.len() >= 60 {
                self.speed_history.pop_front();
            }
            self.speed_history.push_back(total);
        }
    }

    /// Current total download speed across all active tasks, in bytes/s.
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

    /// (active, finished, queued) counts.
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

    pub fn storage_summary(&self) -> SidebarStorageSummary {
        storage_summary_for_path(&self.settings.download_dir())
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Re-query the history DB and notify the UI. Called when the history view
    /// becomes visible and when a download reaches a terminal state.
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

    /// Push a restored download into the SoA vecs without going through the engine.
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

            // Show notification on first terminal transition.
            let is_terminal = matches!(
                update.status,
                DownloadStatus::Finished | DownloadStatus::Error
            );
            let was_terminal = matches!(prev, DownloadStatus::Finished | DownloadStatus::Error);
            if is_terminal && !was_terminal {
                let filename = self.filenames[idx].clone();
                let kind = if update.status == DownloadStatus::Finished {
                    crate::views::overlays::notification::NotificationKind::Success
                } else {
                    crate::views::overlays::notification::NotificationKind::Error
                };
                crate::views::overlays::notification::show(cx, filename, kind);
                self.refresh_history(cx);
            }

            cx.notify();
        }
    }

    fn apply_notification(&mut self, notification: EngineNotification, cx: &mut Context<Self>) {
        match notification {
            EngineNotification::Update(update) => self.apply_progress(update, cx),
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
    fn build_row_index_tracks_current_positions() {
        let map = build_row_index(&[DownloadId(7), DownloadId(11), DownloadId(3)]);

        assert_eq!(map.get(&DownloadId(7)), Some(&0));
        assert_eq!(map.get(&DownloadId(11)), Some(&1));
        assert_eq!(map.get(&DownloadId(3)), Some(&2));
    }
}
