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

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{Context, SharedString};

use crate::engine::state::{self, HistoryReader};
use crate::engine::{
    AddDownloadRequest, DownloadEngine, DownloadId, DownloadStatus, EngineNotification,
    HistoryFilter, HistoryRow, ProgressUpdate, RestoredDownload, SavedDownload,
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
    pub filenames: Vec<SharedString>,
    pub destinations: Vec<SharedString>,
    pub statuses: Vec<DownloadStatus>,
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
        let ipc = IpcServer::start();

        let mut model = Self {
            engine,
            ipc,
            settings,
            ids: Vec::new(),
            filenames: Vec::new(),
            destinations: Vec::new(),
            statuses: Vec::new(),
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
                            let destination = req.destination_in(&model.settings.download_dir());
                            model.add_request(req, destination, cx);
                        }
                        model.tick_speed();
                    })
                    .ok();
                })
                .ok();
            }
        })
        .detach();

        model
    }

    pub fn add(&mut self, url: String, destination: PathBuf, cx: &mut Context<Self>) -> DownloadId {
        self.add_request(AddDownloadRequest::from_url(url), destination, cx)
    }

    pub fn add_request(
        &mut self,
        request: AddDownloadRequest,
        destination: PathBuf,
        cx: &mut Context<Self>,
    ) -> DownloadId {
        let filename: SharedString = destination
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string()
            .into();
        let dest_str: SharedString = destination.to_string_lossy().to_string().into();

        let spec = request.into_spec(destination, &self.settings);
        let id = self.engine.add(spec);
        self.ids.push(id);
        self.filenames.push(filename);
        self.destinations.push(dest_str);
        self.statuses.push(DownloadStatus::Pending);
        self.downloaded_bytes.push(0);
        self.total_bytes.push(None);
        self.speeds.push(0);
        cx.notify();
        id
    }

    pub fn pause(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        self.engine.pause(id);
        cx.notify();
    }

    pub fn apply_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        self.engine.update_settings(settings.clone());
        self.settings = settings;
        cx.notify();
    }

    pub fn resume(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        self.engine.resume(id);
        cx.notify();
    }

    pub fn remove(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        self.engine.cancel(id);
        cx.notify();
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

        self.ids.push(saved.id);
        self.filenames.push(filename);
        self.destinations.push(dest_str);
        self.statuses.push(DownloadStatus::Paused);
        self.downloaded_bytes.push(saved.downloaded_bytes);
        self.total_bytes.push(saved.total_bytes);
        self.speeds.push(0);
    }

    fn apply_progress(&mut self, update: ProgressUpdate, cx: &mut Context<Self>) {
        if let Some(idx) = self.ids.iter().position(|&id| id == update.id) {
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
            EngineNotification::Removed { id } => self.remove_row(id, cx),
        }
    }

    fn remove_row(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        if let Some(idx) = self.ids.iter().position(|&d| d == id) {
            self.ids.remove(idx);
            self.filenames.remove(idx);
            self.destinations.remove(idx);
            self.statuses.remove(idx);
            self.downloaded_bytes.remove(idx);
            self.total_bytes.remove(idx);
            self.speeds.remove(idx);
            cx.notify();
        }
    }
}
