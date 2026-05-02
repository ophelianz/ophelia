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

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransferId(pub u64);

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferStatus {
    Pending,
    Downloading,
    Paused,
    Finished,
    Error,
    Cancelled,
}

/// Controls the engine can ask a download to perform
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferControlAction {
    Pause,
    Resume,
    Cancel,
    Restore,
}

/// Controls this transfer supports
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

    pub const fn supports(self, action: TransferControlAction) -> bool {
        match action {
            TransferControlAction::Pause => self.can_pause,
            TransferControlAction::Resume => self.can_resume,
            TransferControlAction::Cancel => self.can_cancel,
            TransferControlAction::Restore => self.can_restore,
        }
    }
}

/// File state for history rows
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactState {
    Present,
    Deleted,
    Missing,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChunkMapCellState {
    Empty,
    Partial,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectChunkMapSnapshot {
    pub total_bytes: u64,
    pub cells: Vec<ChunkMapCellState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirectChunkMapState {
    Unsupported,
    Loading,
    Segments(DirectChunkMapSnapshot),
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferKind {
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectTransferDetails {
    pub chunk_map_state: DirectChunkMapState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferDetails {
    Direct(DirectTransferDetails),
}

impl TransferDetails {
    pub fn direct(chunk_map_state: DirectChunkMapState) -> Self {
        Self::Direct(DirectTransferDetails { chunk_map_state })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferDetailsSnapshot {
    pub id: TransferId,
    pub details: TransferDetails,
}

/// Events sent to the SQLite worker
/// The worker is the only writer
pub enum DbEvent {
    Added {
        id: TransferId,
        source: PersistedDownloadSource,
        destination: PathBuf,
    },
    DestinationChanged {
        id: TransferId,
        destination: PathBuf,
    },
    Queued {
        id: TransferId,
    },
    Started {
        id: TransferId,
    },
    Paused {
        id: TransferId,
        downloaded_bytes: u64,
        resume_data: Option<RunnerResumeData>,
    },
    Resumed {
        id: TransferId,
    },
    Finished {
        id: TransferId,
        total_bytes: u64,
    },
    Error {
        id: TransferId,
    },
    Cancelled {
        id: TransferId,
    },
    ArtifactStateChanged {
        id: TransferId,
        artifact_state: ArtifactState,
    },
}

/// Filter for the history view
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryFilter {
    All,
    Finished,
    Error,
    Paused,
    Cancelled,
}

/// One download shown in history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRow {
    pub id: TransferId,
    /// Source kind saved in the database
    /// Kept as a string so old rows can still display
    pub provider_kind: String,
    /// Source label shown to the user
    pub source_label: String,
    pub destination: String,
    pub status: TransferStatus,
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
    pub id: TransferId,
    pub source: PersistedDownloadSource,
    pub destination: PathBuf,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub resume_data: Option<RunnerResumeData>,
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
pub enum RunnerResumeData {
    Http(HttpResumeData),
}

impl RunnerResumeData {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressUpdate {
    pub id: TransferId,
    pub status: TransferStatus,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_sec: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferSummary {
    pub id: TransferId,
    pub kind: TransferKind,
    pub provider_kind: String,
    pub source_label: String,
    pub destination: PathBuf,
    pub status: TransferStatus,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bytes_per_sec: u64,
    pub control_support: TransferControlSupport,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineError {
    Closed,
    NotFound {
        id: TransferId,
    },
    Unsupported {
        id: TransferId,
        action: TransferControlAction,
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
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiveTransferRemovalAction {
    Cancelled,
    DeleteArtifact,
}

/// Task-to-engine updates
/// Public only because direct HTTP task tests call `download_task`
#[doc(hidden)]
#[derive(Debug, Clone)]
pub enum RunnerEvent {
    Progress(ProgressUpdate),
    Done {
        id: TransferId,
        status: TransferStatus,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    TransferBytesWritten {
        id: TransferId,
        bytes: u64,
    },
    DestinationChanged {
        id: TransferId,
        destination: PathBuf,
    },
    ControlSupportChanged {
        id: TransferId,
        support: TransferControlSupport,
    },
    DetailsChanged {
        id: TransferId,
        details: TransferDetails,
    },
}
