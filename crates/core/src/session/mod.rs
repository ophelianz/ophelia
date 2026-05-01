use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::config::{CoreConfig, CorePaths};
use crate::engine::state::{self, HistoryReader};
use crate::engine::{
    AddDownloadRequest, ArtifactState, DbEvent, DownloadControlAction, DownloadEngine, DownloadId,
    DownloadSpec, DownloadStatus, EngineError, EngineEvent, HistoryFilter, HistoryRow,
    LiveTransferRemovalAction, ProgressUpdate, RestoredDownload, TransferSnapshot,
    delete_artifact_files,
};

const SESSION_COMMAND_CAPACITY: usize = 64;
const SESSION_EVENT_CAPACITY: usize = 512;
const SESSION_PROTOCOL_VERSION: u32 = 1;
const HOT_SESSION_EVENT_FLUSH_MS: u64 = 100;

mod client;
mod host;
mod lock;
mod read_model;
mod wire;

#[cfg(test)]
mod tests;

pub use client::{SessionClient, SessionSubscription};
pub use host::SessionHost;
pub use lock::{session_descriptor_path, session_lock_path, session_socket_path};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub source: DownloadRequestSource,
    pub destination: DownloadDestination,
}

impl DownloadRequest {
    pub fn http(url: String) -> Self {
        Self {
            source: DownloadRequestSource::Http { url },
            destination: DownloadDestination::Automatic {
                suggested_filename: None,
            },
        }
    }

    pub fn from_add_request(request: AddDownloadRequest) -> Self {
        Self {
            source: DownloadRequestSource::Http {
                url: request.url().to_string(),
            },
            destination: DownloadDestination::Automatic {
                suggested_filename: request.suggested_filename,
            },
        }
    }

    pub fn into_spec(self, config: &CoreConfig) -> io::Result<DownloadSpec> {
        match (self.source, self.destination) {
            (
                DownloadRequestSource::Http { url },
                DownloadDestination::Automatic { suggested_filename },
            ) => DownloadSpec::from_auto_request(url, suggested_filename, config),
            (DownloadRequestSource::Http { url }, DownloadDestination::ExplicitPath(path)) => {
                DownloadSpec::from_user_input(url, path, config)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadRequestSource {
    Http { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadDestination {
    Automatic { suggested_filename: Option<String> },
    ExplicitPath(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum SessionCommand {
    Add {
        request: DownloadRequest,
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
    DeleteArtifact {
        id: DownloadId,
    },
    UpdateConfig {
        config: CoreConfig,
    },
    LoadHistory {
        filter: HistoryFilter,
        query: String,
    },
    Snapshot,
    Subscribe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum SessionResponse {
    Ack,
    DownloadAdded { id: DownloadId },
    History { rows: Vec<HistoryRow> },
    Snapshot { snapshot: SessionSnapshot },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvent {
    TransferChanged {
        snapshot: TransferSnapshot,
    },
    DownloadBytesWritten {
        id: DownloadId,
        bytes: u64,
    },
    TransferRemoved {
        id: DownloadId,
        action: LiveTransferRemovalAction,
        artifact_state: ArtifactState,
    },
    ControlUnsupported {
        id: DownloadId,
        action: DownloadControlAction,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub transfers: Vec<TransferSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionError {
    Closed,
    NotFound {
        id: DownloadId,
    },
    Unsupported {
        id: DownloadId,
        action: DownloadControlAction,
    },
    LockHeld {
        path: PathBuf,
    },
    StaleSession {
        path: PathBuf,
    },
    BadRequest {
        message: String,
    },
    Io {
        message: String,
    },
    Transport {
        message: String,
    },
    Lagged {
        skipped: u64,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => write!(f, "session is closed"),
            Self::NotFound { id } => write!(f, "download {} was not found", id.0),
            Self::Unsupported { id, action } => {
                write!(f, "download {} does not support {action:?}", id.0)
            }
            Self::LockHeld { path } => write!(f, "session lock is held at {}", path.display()),
            Self::StaleSession { path } => {
                write!(f, "stale session descriptor at {}", path.display())
            }
            Self::BadRequest { message } => write!(f, "bad request: {message}"),
            Self::Io { message } => write!(f, "io error: {message}"),
            Self::Transport { message } => write!(f, "transport error: {message}"),
            Self::Lagged { skipped } => write!(f, "session skipped {skipped} events"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<EngineError> for SessionError {
    fn from(error: EngineError) -> Self {
        match error {
            EngineError::Closed => Self::Closed,
            EngineError::NotFound { id } => Self::NotFound { id },
            EngineError::Unsupported { id, action } => Self::Unsupported { id, action },
        }
    }
}

impl From<io::Error> for SessionError {
    fn from(error: io::Error) -> Self {
        Self::Io {
            message: error.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDescriptor {
    pub protocol_version: u32,
    pub pid: u32,
    pub profile_database_path: PathBuf,
    pub socket_path: PathBuf,
    pub created_unix_ms: u128,
}

impl SessionDescriptor {
    pub fn for_paths(paths: &CorePaths) -> Self {
        Self {
            protocol_version: SESSION_PROTOCOL_VERSION,
            pid: std::process::id(),
            profile_database_path: paths.database_path.clone(),
            socket_path: session_socket_path(paths),
            created_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or_default(),
        }
    }
}
