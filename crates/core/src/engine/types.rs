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

//! Shared engine types
//!
//! Most of this is not HTTP-only

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
    Cancelled,
}

/// Controls the engine can ask a download to perform
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadControlAction {
    Pause,
    Resume,
    Cancel,
    Restore,
}

/// Controls this transfer supports
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferControlSupport {
    pub can_pause: bool,
    pub can_resume: bool,
    pub can_cancel: bool,
    pub can_restore: bool,
}

impl TransferControlSupport {
    pub const fn all() -> Self {
        Self {
            can_pause: true,
            can_resume: true,
            can_cancel: true,
            can_restore: true,
        }
    }

    pub const fn supports(self, action: DownloadControlAction) -> bool {
        match action {
            DownloadControlAction::Pause => self.can_pause,
            DownloadControlAction::Resume => self.can_resume,
            DownloadControlAction::Cancel => self.can_cancel,
            DownloadControlAction::Restore => self.can_restore,
        }
    }
}

/// File state for history rows
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactState {
    Present,
    Deleted,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkMapCellState {
    Empty,
    Partial,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpChunkMapSnapshot {
    pub total_bytes: u64,
    pub cells: Vec<ChunkMapCellState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferChunkMapState {
    Unsupported,
    Loading,
    Http(HttpChunkMapSnapshot),
}

/// Events sent to the SQLite worker
/// The worker is the only writer
pub enum DbEvent {
    Added {
        id: DownloadId,
        source: PersistedDownloadSource,
        destination: PathBuf,
    },
    DestinationChanged {
        id: DownloadId,
        destination: PathBuf,
    },
    Queued {
        id: DownloadId,
    },
    Started {
        id: DownloadId,
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
    Cancelled {
        id: DownloadId,
    },
    ArtifactStateChanged {
        id: DownloadId,
        artifact_state: ArtifactState,
    },
}

/// Filter for the history view
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryFilter {
    All,
    Finished,
    Error,
    Paused,
    Cancelled,
}

/// One download shown in history
#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub id: DownloadId,
    /// Source kind saved in the database
    /// Kept as a string so old rows can still display
    pub provider_kind: String,
    /// Source label shown to the user
    pub source_label: String,
    pub destination: String,
    pub status: DownloadStatus,
    #[allow(dead_code)]
    /// Lets history say whether the file still exists
    pub artifact_state: ArtifactState,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    /// Unix milliseconds when the download was added
    pub added_at: i64,
    /// Unix milliseconds when the download finished
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

/// A download loaded from SQLite on startup to restore paused/pending state
#[derive(Debug, Clone)]
pub struct SavedDownload {
    pub id: DownloadId,
    pub source: PersistedDownloadSource,
    pub destination: PathBuf,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub resume_data: Option<ProviderResumeData>,
}

/// Source saved with a transfer record
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

    pub fn control_support(&self) -> TransferControlSupport {
        match self {
            Self::Http { .. } => TransferControlSupport::all(),
        }
    }

    pub fn from_parts(kind: &str, locator: String) -> Option<Self> {
        match kind {
            "http" => Some(Self::Http { url: locator }),
            _ => None,
        }
    }
}

/// One saved byte range for HTTP resume
///
/// `downloaded` is how many bytes from `start..end` are already on disk
#[derive(Debug, Clone)]
pub struct ChunkSnapshot {
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
}

/// Resume data for HTTP range downloads
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
        self.chunks.iter().map(|chunk| chunk.end).max()
    }
}

/// Resume data grouped by source kind
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
pub struct TransferSnapshot {
    pub id: DownloadId,
    pub provider_kind: String,
    pub source_label: String,
    pub destination: PathBuf,
    pub status: DownloadStatus,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_sec: u64,
    pub control_support: TransferControlSupport,
    pub chunk_map_state: TransferChunkMapState,
}

#[derive(Debug, Clone)]
pub enum EngineEvent {
    TransferAdded {
        snapshot: TransferSnapshot,
    },
    TransferRestored {
        snapshot: TransferSnapshot,
    },
    Progress(ProgressUpdate),
    DownloadBytesWritten {
        id: DownloadId,
        bytes: u64,
    },
    DestinationChanged {
        id: DownloadId,
        destination: PathBuf,
    },
    ControlSupportChanged {
        id: DownloadId,
        support: TransferControlSupport,
    },
    ChunkMapChanged {
        id: DownloadId,
        state: TransferChunkMapState,
    },
    LiveTransferRemoved {
        id: DownloadId,
        action: LiveTransferRemovalAction,
        artifact_state: ArtifactState,
    },
    ControlUnsupported {
        id: DownloadId,
        action: DownloadControlAction,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineError {
    Closed,
    NotFound {
        id: DownloadId,
    },
    Unsupported {
        id: DownloadId,
        action: DownloadControlAction,
    },
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "download engine is closed"),
            Self::NotFound { id } => write!(f, "download {} was not found", id.0),
            Self::Unsupported { id, action } => {
                write!(f, "download {} does not support {action:?}", id.0)
            }
        }
    }
}

impl std::error::Error for EngineError {}

/// Why a live transfer row left the Transfers view
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveTransferRemovalAction {
    Cancelled,
    DeleteArtifact,
}

/// Task-to-engine updates
/// Public only because direct HTTP task tests call `download_task`
#[doc(hidden)]
#[derive(Debug, Clone)]
pub enum TaskRuntimeUpdate {
    Progress(ProgressUpdate),
    Done {
        id: DownloadId,
        status: DownloadStatus,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    DownloadBytesWritten {
        id: DownloadId,
        bytes: u64,
    },
    DestinationChanged {
        id: DownloadId,
        destination: PathBuf,
    },
    ControlSupportChanged {
        id: DownloadId,
        support: TransferControlSupport,
    },
    ChunkMapChanged {
        id: DownloadId,
        state: TransferChunkMapState,
    },
}
