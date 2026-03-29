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

pub enum EngineCommand {
    AddHttp { id: DownloadId, url: String, destination: PathBuf, config: HttpDownloadConfig },
    Pause { id: DownloadId },
    Resume { id: DownloadId },
    Cancel { id: DownloadId },
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub id: DownloadId,
    pub status: DownloadStatus,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_sec: u64,
}
