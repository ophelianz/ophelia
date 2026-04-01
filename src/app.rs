//! Application-level download state.
//!
//! Downloads is a GPUI entity that owns the DownloadEngine and the live download
//! data in SoA layout. A background task drains engine progress updates every 100ms
//! and calls cx.notify() to trigger a re-render.

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
    pub settings: Settings,

    pub ids: Vec<DownloadId>,
    pub filenames: Vec<SharedString>,
    pub destinations: Vec<SharedString>,
    pub statuses: Vec<DownloadStatus>,
    pub downloaded_bytes: Vec<u64>,
    pub total_bytes: Vec<Option<u64>>,
    pub speeds: Vec<u64>,
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
        };

        cx.spawn(async |this, cx: &mut gpui::AsyncApp| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                cx.update(|app| {
                    this.update(app, |model, cx| {
                        while let Some(update) = model.engine.poll_progress() {
                            model.apply_progress(update, cx);
                        }
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

    pub fn cancel(&mut self, id: DownloadId) {
        self.engine.cancel(id);
        if let Some(idx) = self.ids.iter().position(|&d| d == id) {
            self.statuses[idx] = DownloadStatus::Error;
        }
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
