//! Application-level download state.
//!
//! Downloads is a GPUI entity that owns the DownloadEngine and the live download
//! data in SoA layout. A background task drains engine progress updates every 100ms
//! and calls cx.notify() to trigger a re-render.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{Context, SharedString};

use crate::engine::http::HttpDownloadConfig;
use crate::engine::{DownloadEngine, DownloadId, DownloadStatus, ProgressUpdate};
use crate::settings::Settings;

/// All live download state in SoA layout.
/// One vec per field, all vecs share the same index space.
pub struct Downloads {
    engine: DownloadEngine,
    #[allow(dead_code)] // future settings panel
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
}

impl Downloads {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let settings = Settings::load();
        let model = Self {
            engine: DownloadEngine::new(settings.clone()),
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
        };

        cx.spawn(async |this, cx: &mut gpui::AsyncApp| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                cx.update(|app| {
                    this.update(app, |model, cx| {
                        while let Some(update) = model.engine.poll_progress() {
                            model.apply_progress(update, cx);
                        }
                        while let Some(req) = model.engine.poll_ipc() {
                            let dir = model.settings.download_dir();
                            let name = req.filename
                                .filter(|n| !n.is_empty())
                                .unwrap_or_else(|| {
                                    req.url.rsplit('/').next()
                                        .and_then(|s| s.split('?').next())
                                        .filter(|s| !s.is_empty())
                                        .unwrap_or("download")
                                        .to_string()
                                });
                            model.add(req.url, dir.join(name), HttpDownloadConfig::default(), cx);
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

    pub fn add(
        &mut self,
        url: String,
        destination: PathBuf,
        config: HttpDownloadConfig,
        cx: &mut Context<Self>,
    ) -> DownloadId {
        let filename: SharedString = destination
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string()
            .into();
        let dest_str: SharedString = destination.to_string_lossy().to_string().into();

        let id = self.engine.add(url, destination, config);
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
        if let Some(idx) = self.ids.iter().position(|&d| d == id) {
            self.statuses[idx] = DownloadStatus::Paused;
            cx.notify();
        }
    }

    pub fn resume(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        self.engine.resume(id);
        if let Some(idx) = self.ids.iter().position(|&d| d == id) {
            self.statuses[idx] = DownloadStatus::Downloading;
            cx.notify();
        }
    }

    pub fn remove(&mut self, id: DownloadId, cx: &mut Context<Self>) {
        self.engine.cancel(id);
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
        self.speed_history.iter().map(|&s| s as f32 / 1_000_000.0).collect()
    }

    /// (active, finished, queued) counts.
    pub fn status_counts(&self) -> (usize, usize, usize) {
        let active   = self.statuses.iter().filter(|&&s| s == DownloadStatus::Downloading).count();
        let finished = self.statuses.iter().filter(|&&s| s == DownloadStatus::Finished).count();
        let queued   = self.statuses.iter().filter(|&&s| matches!(s, DownloadStatus::Pending | DownloadStatus::Paused)).count();
        (active, finished, queued)
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    fn apply_progress(&mut self, update: ProgressUpdate, cx: &mut Context<Self>) {
        if let Some(idx) = self.ids.iter().position(|&id| id == update.id) {
            self.statuses[idx] = update.status;
            self.downloaded_bytes[idx] = update.downloaded_bytes;
            self.total_bytes[idx] = update.total_bytes;
            self.speeds[idx] = update.speed_bytes_per_sec;
            cx.notify();
        }
    }
}
