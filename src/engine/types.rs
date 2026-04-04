//! Shared engine-facing types.
//!
//! Most types in this module are protocol-neutral. Persistence now keeps
//! provider-specific source and resume details behind small enums so the
//! generic engine/app layers do not have to traffic in raw HTTP chunk vectors.

use std::path::PathBuf;

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

/// Events emitted by the engine actor and app layer, consumed by the DbEventWorker.
/// The worker is the sole writer to SQLite and nothing else touches the DB.
pub enum DbEvent {
    Started {
        id: DownloadId,
        source: PersistedDownloadSource,
        destination: PathBuf,
    },
    Paused {
        id: DownloadId,
        downloaded_bytes: u64,
        resume_data: Option<ProviderResumeData>,
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
    /// Storage-facing provider identifier. Kept as a string so history can still
    /// display rows for providers this build does not yet support restoring.
    pub provider_kind: String,
    /// User-facing source label for the transfer, derived from the persisted
    /// provider/source pair when possible and falling back to the stored locator.
    pub source_label: String,
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
#[derive(Debug, Clone)]
pub struct SavedDownload {
    pub id: DownloadId,
    pub source: PersistedDownloadSource,
    pub destination: PathBuf,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub resume_data: Option<ProviderResumeData>,
}

/// Provider-specific source information persisted with a transfer record.
#[derive(Debug, Clone)]
pub enum PersistedDownloadSource {
    Http { url: String },
}

impl PersistedDownloadSource {
    pub fn locator(&self) -> &str {
        match self {
            Self::Http { url } => url,
        }
    }

    pub fn display_label(&self) -> &str {
        self.locator()
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Http { .. } => "http",
        }
    }

    pub fn from_parts(kind: &str, locator: String) -> Option<Self> {
        match kind {
            "http" => Some(Self::Http { url: locator }),
            _ => None,
        }
    }
}

/// Per-chunk resume state. `start` is the stable identity (aria2 / AB DM both key on
/// the first byte of a chunk). `downloaded` is how many bytes of the chunk are on disk.
#[derive(Debug, Clone)]
pub struct ChunkSnapshot {
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
}

/// HTTP-specific resume data persisted for byte-range downloads.
#[derive(Debug, Clone)]
pub struct HttpResumeData {
    pub chunks: Vec<ChunkSnapshot>,
}

impl HttpResumeData {
    pub fn new(chunks: Vec<ChunkSnapshot>) -> Self {
        Self { chunks }
    }

    pub fn downloaded_bytes(&self) -> u64 {
        self.chunks.iter().map(|chunk| chunk.downloaded).sum()
    }

    pub fn total_bytes(&self) -> Option<u64> {
        self.chunks.last().map(|chunk| chunk.end)
    }
}

/// Provider-specific resume data stored behind a generic boundary.
#[derive(Debug, Clone)]
pub enum ProviderResumeData {
    Http(HttpResumeData),
}

impl ProviderResumeData {
    pub fn as_http(&self) -> Option<&HttpResumeData> {
        match self {
            Self::Http(data) => Some(data),
        }
    }

    pub fn downloaded_bytes(&self) -> u64 {
        match self {
            Self::Http(data) => data.downloaded_bytes(),
        }
    }

    pub fn total_bytes(&self) -> Option<u64> {
        match self {
            Self::Http(data) => data.total_bytes(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub id: DownloadId,
    pub status: DownloadStatus,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_sec: u64,
}

#[derive(Debug, Clone)]
pub enum EngineNotification {
    Update(ProgressUpdate),
    Removed { id: DownloadId },
}
