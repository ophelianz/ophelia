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
//! Downloads owns live transfer lists and stats view state.
//! The core service owns downloads, history, and backend tasks.
//!
//! Startup sequence:
//!   1. Load saved settings
//!   2. App shell starts the core service host
//!   3. Start the browser-extension adapter
//!   4. Subscribe to service snapshots and events

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use gpui::{Context, SharedString};

use crate::engine::{
    ArtifactState, DirectChunkMapState, HistoryFilter, HistoryRow, TransferControlAction,
    TransferControlSupport, TransferId, TransferStatus, TransferSummary,
};
#[cfg(test)]
use crate::ipc::IpcServer;
use crate::settings::Settings;
use crate::views::overlays::notification::NotificationKind;
#[cfg(test)]
use ophelia::service::OpheliaService;
use ophelia::service::{
    OpheliaClient, OpheliaError, OpheliaSnapshot, OpheliaUpdateBatch, TransferDestination,
    TransferRequest, TransferRequestSource,
};

/// Live downloads stored as parallel vecs
/// Every vec uses the same row index
pub struct Downloads {
    service_client: OpheliaClient,
    pub settings: Settings,

    pub ids: Vec<TransferId>,
    row_by_id: HashMap<TransferId, usize>,
    /// Source kind per live transfer
    /// Kept app-side so future views do not need engine access
    pub provider_kinds: Vec<SharedString>,
    /// Source label per live transfer
    pub source_labels: Vec<SharedString>,
    pub filenames: Vec<SharedString>,
    pub destinations: Vec<SharedString>,
    pub statuses: Vec<TransferStatus>,
    /// Controls this transfer supports
    pub control_supports: Vec<TransferControlSupport>,
    pub transfer_chunk_maps: Vec<DirectChunkMapState>,
    pub downloaded_bytes: Vec<u64>,
    pub total_bytes: Vec<Option<u64>>,
    pub speeds: Vec<u64>,

    /// Rolling ~60-second download speed history
    pub speed_history: VecDeque<u64>,
    write_sampler: DownloadWriteSampler,
    poll_ticks: u8,

    pub history: Vec<HistoryRow>,
    pub history_filter: HistoryFilter,

    #[cfg(test)]
    _test_db_dir: Option<tempfile::TempDir>,
    #[cfg(test)]
    _test_service_host: Option<OpheliaService>,
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
    fn from_status(status: TransferStatus) -> Self {
        match status {
            TransferStatus::Downloading => Self::Active,
            TransferStatus::Paused => Self::Paused,
            TransferStatus::Finished => Self::Finished,
            TransferStatus::Error | TransferStatus::Cancelled => Self::Error,
            TransferStatus::Pending => Self::Queued,
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
        status: TransferStatus,
        support: TransferControlSupport,
    ) -> Self {
        Self {
            pause: matches!(
                status,
                TransferStatus::Pending | TransferStatus::Downloading
            ) && support.can_pause,
            resume: matches!(status, TransferStatus::Paused) && support.can_resume,
            cancel: matches!(
                status,
                TransferStatus::Pending | TransferStatus::Downloading | TransferStatus::Paused
            ) && support.can_cancel,
            delete_artifact: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransferListRow {
    pub id: TransferId,
    #[allow(dead_code)]
    // kept app-facing so future runner filters/badges do not need engine access
    pub provider_kind: SharedString,
    #[allow(dead_code)]
    // kept app-facing so future views can render source labels directly
    pub source_label: SharedString,
    pub filename: SharedString,
    pub destination: SharedString,
    pub status: TransferStatus,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub progress: f32,
    #[allow(dead_code)] // keeping this for possible (?) ui changes
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
    pub id: TransferId,
    pub provider_kind: SharedString,
    pub source_label: SharedString,
    pub filename: SharedString,
    #[allow(dead_code)] // retained for future history row actions like reveal/copy destination
    pub destination: SharedString,
    pub status: TransferStatus,
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
    pub fn new(service_client: OpheliaClient, settings: Settings, cx: &mut Context<Self>) -> Self {
        Self::from_service(settings, service_client, cx)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(cx: &mut Context<Self>) -> Self {
        let db_dir = tempfile::tempdir().expect("failed to create test database directory");
        let settings = Settings {
            default_download_dir: Some(db_dir.path().join("downloads")),
            ..Settings::default()
        };
        let paths = crate::engine::ProfilePaths::new(
            db_dir.path().join("downloads.db"),
            settings.download_dir(),
        );
        let runtime = crate::runtime::Tokio::handle(cx);
        let service_host =
            OpheliaService::start_with_settings(&runtime, paths, settings.service_settings())
                .expect("failed to start test backend service");
        let service_client = service_host.client();
        crate::service_services::install(IpcServer::disabled(), cx);
        let mut model = Self::from_service(settings, service_client, cx);
        model._test_db_dir = Some(db_dir);
        model._test_service_host = Some(service_host);
        model
    }

    fn from_service(
        settings: Settings,
        service_client: OpheliaClient,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut model = Self {
            service_client: service_client.clone(),
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
            history: Vec::new(),
            history_filter: HistoryFilter::All,
            #[cfg(test)]
            _test_db_dir: None,
            #[cfg(test)]
            _test_service_host: None,
        };

        model.refresh_history(cx);

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            match service_client.subscribe().await {
                Ok(mut subscription) => {
                    let snapshot = subscription.snapshot.clone();
                    cx.update(|app| {
                        this.update(app, |model, cx| {
                            model.apply_service_snapshot(snapshot, cx);
                        })
                        .ok();
                    });

                    loop {
                        match subscription.next_update().await {
                            Ok(update) => {
                                cx.update(|app| {
                                    this.update(app, |model, cx| {
                                        model.apply_service_update(update, cx);
                                    })
                                    .ok();
                                });
                            }
                            Err(error) => {
                                if let Some(skipped) = service_lagged_skip_count(&error) {
                                    tracing::warn!(
                                        skipped,
                                        "backend service event stream lagged, refreshing snapshot"
                                    );
                                    match service_client.subscribe().await {
                                        Ok(next_subscription) => {
                                            let snapshot = next_subscription.snapshot.clone();
                                            subscription = next_subscription;
                                            cx.update(|app| {
                                                this.update(app, |model, cx| {
                                                    model.apply_service_snapshot(snapshot, cx);
                                                })
                                                .ok();
                                            });
                                        }
                                        Err(error) => {
                                            tracing::warn!(
                                                "backend service resubscribe failed: {error}"
                                            );
                                            break;
                                        }
                                    }
                                } else {
                                    tracing::warn!("backend service event stream closed: {error}");
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!("backend service subscribe failed: {error}");
                }
            }
        })
        .detach();

        cx.spawn(async |this, cx: &mut gpui::AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                cx.update(|app| {
                    this.update(app, |model, _cx| {
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
    ) -> Option<TransferId> {
        let display_name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("download")
            .to_string();
        let request = TransferRequest {
            source: TransferRequestSource::Http { url },
            destination: TransferDestination::ExplicitPath(destination),
        };
        self.add_request(request, display_name, cx)
    }

    fn add_request(
        &mut self,
        request: TransferRequest,
        display_name: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Option<TransferId> {
        let _display_name: SharedString = display_name.into();
        let client = self.service_client.clone();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            if let Err(error) = client.add(request).await {
                cx.update(|app| {
                    this.update(app, |model, cx| {
                        model.report_command_failure(None, OpheliaCommandKind::Add, error, cx);
                    })
                    .ok();
                });
            }
        })
        .detach();
        cx.notify();
        None
    }

    pub fn pause(&mut self, id: TransferId, cx: &mut Context<Self>) {
        self.send_control_command(
            id,
            OpheliaCommandKind::Pause,
            move |client| async move { client.pause(id).await },
            cx,
        );
        cx.notify();
    }

    pub fn apply_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        if settings.ipc_port != self.settings.ipc_port {
            let runtime = crate::runtime::Tokio::handle(cx);
            crate::service_services::restart_ipc(
                settings.ipc_port,
                &runtime,
                self.service_client.clone(),
                cx,
            );
        }
        let client = self.service_client.clone();
        let service_settings = settings.service_settings();
        cx.spawn(async move |_this, _cx: &mut gpui::AsyncApp| {
            if let Err(error) = client.update_settings(service_settings).await {
                tracing::warn!("backend service settings update failed: {error}");
            }
        })
        .detach();
        self.settings = settings;
        cx.notify();
    }

    pub fn resume(&mut self, id: TransferId, cx: &mut Context<Self>) {
        self.send_control_command(
            id,
            OpheliaCommandKind::Resume,
            move |client| async move { client.resume(id).await },
            cx,
        );
        cx.notify();
    }

    /// Cancel a live transfer without deleting bytes already on disk
    #[allow(dead_code)] // reserved for a future UI with a distinct cancel-transfer action.
    pub fn cancel_transfer(&mut self, id: TransferId, cx: &mut Context<Self>) {
        self.send_control_command(
            id,
            OpheliaCommandKind::Cancel,
            move |client| async move { client.cancel(id).await },
            cx,
        );
        cx.notify();
    }

    /// Delete the file for a live transfer
    /// History is kept separately
    pub fn delete_artifact(&mut self, id: TransferId, cx: &mut Context<Self>) {
        if self.index_of(id).is_none() {
            return;
        };
        self.send_control_command(
            id,
            OpheliaCommandKind::DeleteArtifact,
            move |client| async move { client.delete_artifact(id).await },
            cx,
        );
        cx.notify();
    }

    /// Removing a live transfer deletes the file but keeps history
    pub fn remove(&mut self, id: TransferId, cx: &mut Context<Self>) {
        self.delete_artifact(id, cx);
    }

    /// Open the folder containing a transfer's destination
    pub fn open_destination_folder(&mut self, id: TransferId, _cx: &mut Context<Self>) {
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
        if self.poll_ticks.is_multiple_of(10) {
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

    /// Current visible file-read slot. This does not report OS process reads.
    pub fn file_read_speed_bps(&self) -> Option<u64> {
        Some(0)
    }

    /// Current app-level rate of bytes successfully written by download tasks, not OS process I/O.
    pub fn file_write_speed_bps(&self) -> Option<u64> {
        Some(self.write_sampler.write_speed_bps())
    }

    /// active, finished, queued counts
    pub fn status_counts(&self) -> (usize, usize, usize) {
        let active = self
            .statuses
            .iter()
            .filter(|&&s| s == TransferStatus::Downloading)
            .count();
        let finished = self
            .statuses
            .iter()
            .filter(|&&s| s == TransferStatus::Finished)
            .count();
        let queued = self
            .statuses
            .iter()
            .filter(|&&s| matches!(s, TransferStatus::Pending | TransferStatus::Paused))
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
    pub fn transfer_chunk_map_state(&self, id: TransferId) -> DirectChunkMapState {
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
        let client = self.service_client.clone();
        let filter = self.history_filter;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            match client.load_history(filter, "").await {
                Ok(rows) => {
                    cx.update(|app| {
                        this.update(app, |model, cx| {
                            model.history = rows;
                            cx.notify();
                        })
                        .ok();
                    });
                }
                Err(error) => tracing::warn!("history query failed: {error}"),
            }
        })
        .detach();
    }

    pub fn set_history_filter(&mut self, filter: HistoryFilter, cx: &mut Context<Self>) {
        self.history_filter = filter;
        self.refresh_history(cx);
    }

    fn apply_service_snapshot(&mut self, snapshot: OpheliaSnapshot, cx: &mut Context<Self>) {
        let OpheliaSnapshot {
            transfers,
            direct_details,
            settings,
        } = snapshot;
        self.settings.apply_service_settings(settings);
        self.ids.clear();
        self.row_by_id.clear();
        self.provider_kinds.clear();
        self.source_labels.clear();
        self.filenames.clear();
        self.destinations.clear();
        self.statuses.clear();
        self.control_supports.clear();
        self.transfer_chunk_maps.clear();
        self.downloaded_bytes.clear();
        self.total_bytes.clear();
        self.speeds.clear();

        for transfer in transfers.summaries() {
            self.upsert_snapshot(transfer, cx);
        }
        self.apply_direct_details(direct_details, cx);
        cx.notify();
    }

    pub fn settings_snapshot(&self) -> Settings {
        self.settings.clone()
    }

    fn apply_service_update(&mut self, update: OpheliaUpdateBatch, cx: &mut Context<Self>) {
        if let Some(settings) = update.settings_changed {
            self.settings.apply_service_settings(settings);
            cx.notify();
        }

        for snapshot in update.lifecycle.transfers.summaries() {
            self.upsert_snapshot(snapshot, cx);
        }

        for (id, ((downloaded, total), speed)) in
            update.progress_known_total.ids.iter().copied().zip(
                update
                    .progress_known_total
                    .downloaded_bytes
                    .iter()
                    .copied()
                    .zip(update.progress_known_total.total_bytes.iter().copied())
                    .zip(
                        update
                            .progress_known_total
                            .speed_bytes_per_sec
                            .iter()
                            .copied(),
                    ),
            )
        {
            self.apply_progress(id, downloaded, Some(total), speed, cx);
        }

        for (id, (downloaded, speed)) in update.progress_unknown_total.ids.iter().copied().zip(
            update
                .progress_unknown_total
                .downloaded_bytes
                .iter()
                .copied()
                .zip(
                    update
                        .progress_unknown_total
                        .speed_bytes_per_sec
                        .iter()
                        .copied(),
                ),
        ) {
            self.apply_progress(id, downloaded, None, speed, cx);
        }

        for bytes in update.physical_write.bytes {
            self.write_sampler.record(bytes);
        }

        for (id, destination) in update
            .destination
            .ids
            .iter()
            .copied()
            .zip(update.destination.destinations)
        {
            self.apply_destination(id, destination, cx);
        }

        for (row, id) in update.control_support.ids.iter().copied().enumerate() {
            if let Some(support) = update.control_support.support(row)
                && let Some(idx) = self.index_of(id)
            {
                self.control_supports[idx] = support;
                cx.notify();
            }
        }

        self.apply_direct_details(update.direct_details, cx);

        for (row, id) in update.removal.ids.iter().copied().enumerate() {
            if let (Some(action), Some(artifact_state)) = (
                update.removal.action(row),
                update.removal.artifact_state(row),
            ) {
                tracing::debug!(
                    id = id.0,
                    action = live_transfer_removal_action_name(action),
                    artifact_state = artifact_state_name(artifact_state),
                    "live transfer left the active surface"
                );
                self.remove_row(id, cx);
                self.refresh_history(cx);
            }
        }

        for (row, id) in update.unsupported_control.ids.iter().copied().enumerate() {
            if let Some(action) = update.unsupported_control.action(row) {
                tracing::warn!(
                    id = id.0,
                    action = control_action_name(action),
                    "runner does not support requested control action"
                );
                if let Some(idx) = self.index_of(id) {
                    let filename = self.filenames[idx].clone();
                    self.show_notification(cx, filename, NotificationKind::Error);
                }
            }
        }
    }

    fn apply_progress(
        &mut self,
        id: TransferId,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        speed_bytes_per_sec: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(idx) = self.index_of(id) else {
            return;
        };
        self.downloaded_bytes[idx] = downloaded_bytes;
        self.total_bytes[idx] = total_bytes;
        self.speeds[idx] = speed_bytes_per_sec;
        cx.notify();
    }

    fn apply_destination(&mut self, id: TransferId, destination: PathBuf, cx: &mut Context<Self>) {
        let Some(idx) = self.index_of(id) else {
            return;
        };
        self.filenames[idx] = notification_filename(&destination);
        self.destinations[idx] = destination.to_string_lossy().to_string().into();
        cx.notify();
    }

    fn apply_direct_details(
        &mut self,
        details: ophelia::service::DirectDetailsTable,
        cx: &mut Context<Self>,
    ) {
        for id in details
            .unsupported_ids
            .iter()
            .chain(details.loading_ids.iter())
            .chain(details.segment_ids.iter())
            .copied()
        {
            if let Some(idx) = self.index_of(id) {
                self.transfer_chunk_maps[idx] = details.state_for(id);
            }
        }
        cx.notify();
    }

    fn upsert_snapshot(&mut self, snapshot: TransferSummary, cx: &mut Context<Self>) {
        let previous_status = self.index_of(snapshot.id).map(|idx| self.statuses[idx]);
        let next_status = snapshot.status;
        let filename = notification_filename(&snapshot.destination);
        let notification_filename = filename.clone();
        let destination: SharedString = snapshot.destination.to_string_lossy().to_string().into();
        let provider_kind: SharedString = snapshot.provider_kind.into();
        let source_label: SharedString = snapshot.source_label.into();

        if let Some(idx) = self.index_of(snapshot.id) {
            self.provider_kinds[idx] = provider_kind;
            self.source_labels[idx] = source_label;
            self.filenames[idx] = filename;
            self.destinations[idx] = destination;
            self.statuses[idx] = snapshot.status;
            self.control_supports[idx] = snapshot.control_support;
            self.downloaded_bytes[idx] = snapshot.downloaded_bytes;
            self.total_bytes[idx] = snapshot.total_bytes;
            self.speeds[idx] = snapshot.speed_bytes_per_sec;
        } else {
            self.ids.push(snapshot.id);
            self.provider_kinds.push(provider_kind);
            self.source_labels.push(source_label);
            self.filenames.push(filename);
            self.destinations.push(destination);
            self.statuses.push(snapshot.status);
            self.control_supports.push(snapshot.control_support);
            self.transfer_chunk_maps
                .push(DirectChunkMapState::Unsupported);
            self.downloaded_bytes.push(snapshot.downloaded_bytes);
            self.total_bytes.push(snapshot.total_bytes);
            self.speeds.push(snapshot.speed_bytes_per_sec);
            self.row_by_id.insert(snapshot.id, self.ids.len() - 1);
        }

        if let Some(previous) = previous_status
            && let Some(kind) = terminal_notification_kind(previous, next_status)
        {
            self.show_notification(cx, notification_filename, kind);
            self.refresh_history(cx);
        }

        cx.notify();
    }

    fn report_command_failure(
        &self,
        id: Option<TransferId>,
        action: OpheliaCommandKind,
        error: OpheliaError,
        cx: &mut Context<Self>,
    ) {
        tracing::warn!(
            id = id.map(|id| id.0),
            action = service_command_kind_name(action),
            "download service command failed: {error}"
        );

        if let Some(id) = id.and_then(|id| self.index_of(id)) {
            let filename = self.filenames[id].clone();
            self.show_notification(cx, filename, NotificationKind::Error);
        }
    }

    fn remove_row(&mut self, id: TransferId, cx: &mut Context<Self>) {
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

    fn index_of(&self, id: TransferId) -> Option<usize> {
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

    fn send_control_command<Fut>(
        &self,
        id: TransferId,
        action: OpheliaCommandKind,
        command: impl FnOnce(OpheliaClient) -> Fut + 'static,
        cx: &mut Context<Self>,
    ) where
        Fut: std::future::Future<Output = Result<(), OpheliaError>> + 'static,
    {
        let client = self.service_client.clone();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            if let Err(error) = command(client).await {
                cx.update(|app| {
                    this.update(app, |model, cx| {
                        model.report_command_failure(Some(id), action, error, cx);
                    })
                    .ok();
                });
            }
        })
        .detach();
    }
}

fn service_lagged_skip_count(error: &OpheliaError) -> Option<u64> {
    match error {
        OpheliaError::Lagged { skipped } => Some(*skipped),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpheliaCommandKind {
    Add,
    Pause,
    Resume,
    Cancel,
    DeleteArtifact,
}

fn control_action_name(action: TransferControlAction) -> &'static str {
    match action {
        TransferControlAction::Pause => "pause",
        TransferControlAction::Resume => "resume",
        TransferControlAction::Cancel => "cancel",
        TransferControlAction::Restore => "restore",
    }
}

fn service_command_kind_name(action: OpheliaCommandKind) -> &'static str {
    match action {
        OpheliaCommandKind::Add => "add",
        OpheliaCommandKind::Pause => "pause",
        OpheliaCommandKind::Resume => "resume",
        OpheliaCommandKind::Cancel => "cancel",
        OpheliaCommandKind::DeleteArtifact => "delete_artifact",
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

fn terminal_notification_kind(
    previous: TransferStatus,
    next: TransferStatus,
) -> Option<NotificationKind> {
    let was_terminal = matches!(previous, TransferStatus::Finished | TransferStatus::Error);
    if was_terminal {
        return None;
    }

    match next {
        TransferStatus::Finished => Some(NotificationKind::Success),
        TransferStatus::Error => Some(NotificationKind::Error),
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

fn build_row_index(ids: &[TransferId]) -> HashMap<TransferId, usize> {
    ids.iter()
        .copied()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect()
}

#[cfg_attr(not(test), allow(dead_code))]
fn transfer_chunk_map_state_or_unsupported(
    states: &[DirectChunkMapState],
    index: Option<usize>,
) -> DirectChunkMapState {
    index
        .and_then(|idx| states.get(idx).cloned())
        .unwrap_or(DirectChunkMapState::Unsupported)
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
    use gpui::{AppContext, TestApp};

    fn test_downloads(app: &mut TestApp) -> gpui::Entity<Downloads> {
        app.update(|cx| {
            crate::runtime::init(cx);
            cx.new(Downloads::new_for_test)
        })
    }

    fn test_snapshot(id: u64, destination: &str, status: TransferStatus) -> TransferSummary {
        TransferSummary {
            id: TransferId(id),
            kind: crate::engine::TransferKind::Direct,
            provider_kind: "http".into(),
            source_label: "https://example.com/file.bin".into(),
            destination: PathBuf::from(destination),
            status,
            downloaded_bytes: 128,
            total_bytes: Some(256),
            speed_bytes_per_sec: 64,
            control_support: TransferControlSupport::all(),
        }
    }

    fn summary_table(
        summaries: impl IntoIterator<Item = TransferSummary>,
    ) -> ophelia::service::TransferSummaryTable {
        let mut table = ophelia::service::TransferSummaryTable::default();
        for summary in summaries {
            table.push_summary(summary);
        }
        table
    }

    fn snapshot(summaries: Vec<TransferSummary>) -> OpheliaSnapshot {
        OpheliaSnapshot {
            transfers: summary_table(summaries),
            direct_details: Default::default(),
            settings: Default::default(),
        }
    }

    fn transfer_update(summaries: Vec<TransferSummary>) -> OpheliaUpdateBatch {
        let mut update = OpheliaUpdateBatch::default();
        for _ in &summaries {
            update
                .lifecycle
                .lifecycle_codes
                .push(ophelia::service::TransferLifecycleCode::Added as u8);
        }
        update.lifecycle.transfers = summary_table(summaries);
        update
    }

    fn removal_update(
        id: TransferId,
        action: crate::engine::LiveTransferRemovalAction,
        artifact_state: ArtifactState,
    ) -> OpheliaUpdateBatch {
        let mut update = OpheliaUpdateBatch::default();
        update.removal.ids.push(id);
        update.removal.action_codes.push(action as u8);
        update
            .removal
            .artifact_state_codes
            .push(artifact_state as u8);
        update
    }

    fn unsupported_control_update(
        id: TransferId,
        action: TransferControlAction,
    ) -> OpheliaUpdateBatch {
        let mut update = OpheliaUpdateBatch::default();
        update.unsupported_control.ids.push(id);
        update.unsupported_control.action_codes.push(action as u8);
        update
    }

    fn physical_write_update(id: TransferId, bytes: u64) -> OpheliaUpdateBatch {
        let mut update = OpheliaUpdateBatch::default();
        update.physical_write.ids.push(id);
        update.physical_write.bytes.push(bytes);
        update
    }

    #[test]
    fn transfer_available_actions_follow_status_and_capabilities() {
        let support = TransferControlSupport {
            can_pause: true,
            can_resume: true,
            can_cancel: true,
            can_restore: false,
        };

        assert_eq!(
            TransferAvailableActions::from_status_and_support(TransferStatus::Downloading, support,),
            TransferAvailableActions {
                pause: true,
                resume: false,
                cancel: true,
                delete_artifact: true,
            }
        );

        assert_eq!(
            TransferAvailableActions::from_status_and_support(TransferStatus::Paused, support),
            TransferAvailableActions {
                pause: false,
                resume: true,
                cancel: true,
                delete_artifact: true,
            }
        );

        assert_eq!(
            TransferAvailableActions::from_status_and_support(TransferStatus::Finished, support),
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
            id: TransferId(7),
            provider_kind: "soulseek".into(),
            source_label: "artist/track.flac".into(),
            destination: "/tmp/Music/track.flac".into(),
            status: TransferStatus::Cancelled,
            artifact_state: ArtifactState::Missing,
            total_bytes: Some(1024),
            downloaded_bytes: 512,
            added_at: 10,
            finished_at: Some(20),
        };

        let model = HistoryListRow::from_history_row(&row);

        assert_eq!(model.id, TransferId(7));
        assert_eq!(model.filename.as_ref(), "track.flac");
        assert_eq!(
            model.source_summary().as_ref(),
            "soulseek: artist/track.flac"
        );
        assert_eq!(model.artifact_state, ArtifactState::Missing);
        assert_eq!(model.status, TransferStatus::Cancelled);
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
    fn terminal_notification_kind_only_emits_on_first_terminal_transition() {
        assert_eq!(
            terminal_notification_kind(TransferStatus::Downloading, TransferStatus::Finished),
            Some(NotificationKind::Success)
        );
        assert_eq!(
            terminal_notification_kind(TransferStatus::Downloading, TransferStatus::Error),
            Some(NotificationKind::Error)
        );
        assert_eq!(
            terminal_notification_kind(TransferStatus::Finished, TransferStatus::Finished),
            None
        );
        assert_eq!(
            terminal_notification_kind(TransferStatus::Error, TransferStatus::Error),
            None
        );
        assert_eq!(
            terminal_notification_kind(TransferStatus::Paused, TransferStatus::Paused),
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
        let enabled_settings = Settings {
            notifications_enabled: true,
            ..Default::default()
        };

        app.update(|cx| {
            show_popup_notification(
                &enabled_settings,
                cx,
                "file.mp4".into(),
                NotificationKind::Started,
            );
        });
        assert_eq!(app.windows().len(), 1);

        let disabled_settings = Settings {
            notifications_enabled: false,
            ..Default::default()
        };
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
        let map = build_row_index(&[TransferId(7), TransferId(11), TransferId(3)]);

        assert_eq!(map.get(&TransferId(7)), Some(&0));
        assert_eq!(map.get(&TransferId(11)), Some(&1));
        assert_eq!(map.get(&TransferId(3)), Some(&2));
    }

    #[test]
    fn lagged_subscription_errors_are_resubscribed() {
        assert_eq!(
            service_lagged_skip_count(&OpheliaError::Lagged { skipped: 12 }),
            Some(12)
        );
        assert_eq!(service_lagged_skip_count(&OpheliaError::Closed), None);
    }

    #[test]
    fn service_snapshot_replaces_transfer_rows() {
        let mut app = TestApp::new();
        let downloads = test_downloads(&mut app);

        app.update(|cx| {
            downloads.update(cx, |downloads, cx| {
                downloads.apply_service_snapshot(
                    snapshot(vec![
                        test_snapshot(1, "/tmp/first.bin", TransferStatus::Downloading),
                        test_snapshot(2, "/tmp/second.bin", TransferStatus::Paused),
                    ]),
                    cx,
                );
                downloads.apply_service_snapshot(
                    snapshot(vec![test_snapshot(
                        2,
                        "/tmp/second-renamed.bin",
                        TransferStatus::Downloading,
                    )]),
                    cx,
                );
            });

            let rows = downloads.read(cx).transfer_rows();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id, TransferId(2));
            assert_eq!(rows[0].filename.as_ref(), "second-renamed.bin");
            assert_eq!(rows[0].status, TransferStatus::Downloading);
        });
    }

    #[test]
    fn service_transfer_changed_events_upsert_transfer_rows() {
        let mut app = TestApp::new();
        let downloads = test_downloads(&mut app);

        app.update(|cx| {
            downloads.update(cx, |downloads, cx| {
                downloads.apply_service_update(
                    transfer_update(vec![test_snapshot(
                        7,
                        "/tmp/first.bin",
                        TransferStatus::Downloading,
                    )]),
                    cx,
                );
                downloads.apply_service_update(
                    transfer_update(vec![test_snapshot(
                        7,
                        "/tmp/renamed.bin",
                        TransferStatus::Paused,
                    )]),
                    cx,
                );
            });

            let rows = downloads.read(cx).transfer_rows();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id, TransferId(7));
            assert_eq!(rows[0].filename.as_ref(), "renamed.bin");
            assert_eq!(rows[0].status, TransferStatus::Paused);
        });
    }

    #[test]
    fn live_transfer_removed_event_removes_transfer_row() {
        let mut app = TestApp::new();
        let downloads = test_downloads(&mut app);

        app.update(|cx| {
            downloads.update(cx, |downloads, cx| {
                downloads.apply_service_update(
                    transfer_update(vec![test_snapshot(
                        3,
                        "/tmp/restored.bin",
                        TransferStatus::Paused,
                    )]),
                    cx,
                );
                downloads.apply_service_update(
                    removal_update(
                        TransferId(3),
                        crate::engine::LiveTransferRemovalAction::Cancelled,
                        ArtifactState::Present,
                    ),
                    cx,
                );
            });

            assert!(downloads.read(cx).transfer_rows().is_empty());
        });
    }

    #[test]
    fn control_unsupported_event_keeps_transfer_row() {
        let mut app = TestApp::new();
        let downloads = test_downloads(&mut app);

        app.update(|cx| {
            downloads.update(cx, |downloads, cx| {
                downloads.settings.notifications_enabled = false;
                downloads.apply_service_update(
                    transfer_update(vec![test_snapshot(
                        8,
                        "/tmp/file.bin",
                        TransferStatus::Downloading,
                    )]),
                    cx,
                );
                downloads.apply_service_update(
                    unsupported_control_update(TransferId(8), TransferControlAction::Pause),
                    cx,
                );
            });

            let rows = downloads.read(cx).transfer_rows();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].id, TransferId(8));
            assert_eq!(rows[0].status, TransferStatus::Downloading);
        });
    }

    #[test]
    fn download_bytes_written_updates_file_write_rate() {
        let mut app = TestApp::new();
        let downloads = test_downloads(&mut app);
        let start = Instant::now();

        app.update(|cx| {
            downloads.update(cx, |downloads, cx| {
                downloads.apply_service_update(physical_write_update(TransferId(99), 2_000), cx);
                downloads.write_sampler.sample_now(start);
                downloads.apply_service_update(physical_write_update(TransferId(99), 6_000), cx);
                downloads
                    .write_sampler
                    .sample_now(start + Duration::from_secs(2));

                assert_eq!(downloads.file_read_speed_bps(), Some(0));
                assert_eq!(downloads.file_write_speed_bps(), Some(3_000));
            });
        });
    }

    #[test]
    fn transfer_chunk_map_state_defaults_to_unsupported_for_missing_rows() {
        let states = vec![DirectChunkMapState::Loading];

        assert_eq!(
            transfer_chunk_map_state_or_unsupported(&states, None),
            DirectChunkMapState::Unsupported
        );
    }

    #[test]
    fn transfer_chunk_map_state_returns_stored_state_for_present_rows() {
        let states = vec![
            DirectChunkMapState::Unsupported,
            DirectChunkMapState::Loading,
        ];

        assert_eq!(
            transfer_chunk_map_state_or_unsupported(&states, Some(1)),
            DirectChunkMapState::Loading
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
