//! Protocol-agnostic types shared across the entire engine.
//! Nothing in this file should be specific to HTTP, FTP, or any other protocol.

use std::path::PathBuf;

use crate::engine::http::HttpDownloadConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DownloadId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStatus {
    Pending,
    Downloading,
    Paused,
    Finished,
    Error,
}

#[allow(dead_code)]
pub enum EngineCommand {
    AddHttp {
        id: DownloadId,
        url: String,
        destination: PathBuf,
        config: HttpDownloadConfig,
    },
    Pause {
        id: DownloadId,
    },
    Resume {
        id: DownloadId,
    },
    Cancel {
        id: DownloadId,
    },
    /// Pre-populate the paused map on startup without starting a task.
    Restore {
        id: DownloadId,
        url: String,
        destination: PathBuf,
        config: HttpDownloadConfig,
        chunks: Vec<ChunkSnapshot>,
    },
    Shutdown,
}

/// Events emitted by the engine actor and app layer, consumed by the DbEventWorker.
/// The worker is the sole writer to SQLite and nothing else touches the DB.
pub enum DbEvent {
    Started {
        id: DownloadId,
        url: String,
        destination: PathBuf,
    },
    Paused {
        id: DownloadId,
        downloaded_bytes: u64,
        chunks: Vec<ChunkSnapshot>,
    },
    Resumed {
        id: DownloadId,
    },
    Finished {
        id: DownloadId,
        total_bytes: u64,
    },
    Error {
        id: DownloadId,
    },
    Removed {
        id: DownloadId,
    },
}

/// Filter for the history view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryFilter {
    All,
    Finished,
    Error,
    Paused,
}

/// A row returned by the history query, one entry per download ever recorded.
#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub id: DownloadId,
    pub url: String,
    pub destination: String,
    pub status: DownloadStatus,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    /// Unix milliseconds when the download was added.
    pub added_at: i64,
    /// Unix milliseconds when the download finished (if it did).
    pub finished_at: Option<i64>,
}

impl HistoryRow {
    pub fn filename(&self) -> &str {
        std::path::Path::new(&self.destination)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&self.destination)
    }
}

/// A download loaded from SQLite on startup to restore paused/pending state.
#[derive(Debug)]
pub struct SavedDownload {
    pub id: DownloadId,
    pub url: String,
    pub destination: PathBuf,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub chunks: Vec<ChunkSnapshot>,
}

/// Per-chunk resume state. `start` is the stable identity (aria2 / AB DM both key on
/// the first byte of a chunk). `downloaded` is how many bytes of the chunk are on disk.
#[derive(Debug, Clone)]
pub struct ChunkSnapshot {
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
}

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub id: DownloadId,
    pub status: DownloadStatus,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_sec: u64,
}
