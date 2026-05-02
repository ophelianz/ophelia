use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::config::{EngineConfig, ProfilePaths, ServiceSettings};
use crate::engine::state::{self, HistoryReader};
use crate::engine::{
    AddTransferRequest, ArtifactState, DbEvent, DownloadEngine, DownloadSpec, EngineError,
    EngineEvent, HistoryFilter, HistoryRow, LiveTransferRemovalAction, ProgressUpdate,
    RestoredDownload, TransferControlAction, TransferId, TransferStatus, TransferSummary,
    delete_artifact_files,
};

const SERVICE_COMMAND_CAPACITY: usize = 64;
const SERVICE_EVENT_CAPACITY: usize = 512;
const HOT_SERVICE_EVENT_FLUSH_MS: u64 = 100;
pub const OPHELIA_MACH_SERVICE_NAME: &str = "nz.ophelia.service";

mod client;
mod host;
mod lock;
#[cfg(target_os = "macos")]
mod macos_startup;
mod read_model;
mod wire;
#[cfg(target_os = "macos")]
mod xpc;

#[cfg(test)]
mod tests;

pub use client::{
    LocalServiceConnection, LocalServiceOptions, LocalServiceRepairPolicy, LocalServiceWarning,
    OpheliaClient, OpheliaSubscription,
};
pub use host::OpheliaService;
pub use lock::service_lock_path;
#[cfg(target_os = "macos")]
pub use xpc::{MachServiceListener, run_mach_service};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferRequest {
    pub source: TransferRequestSource,
    pub destination: TransferDestination,
}

impl TransferRequest {
    pub fn http(url: String) -> Self {
        Self {
            source: TransferRequestSource::Http { url },
            destination: TransferDestination::Automatic {
                suggested_filename: None,
            },
        }
    }

    pub fn from_add_request(request: AddTransferRequest) -> Self {
        Self {
            source: TransferRequestSource::Http {
                url: request.url().to_string(),
            },
            destination: TransferDestination::Automatic {
                suggested_filename: request.suggested_filename,
            },
        }
    }

    pub fn into_spec(self, config: &EngineConfig) -> io::Result<DownloadSpec> {
        match (self.source, self.destination) {
            (
                TransferRequestSource::Http { url },
                TransferDestination::Automatic { suggested_filename },
            ) => DownloadSpec::from_auto_request(url, suggested_filename, config),
            (TransferRequestSource::Http { url }, TransferDestination::ExplicitPath(path)) => {
                DownloadSpec::from_user_input(url, path, config)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum TransferRequestSource {
    Http { url: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum TransferDestination {
    Automatic { suggested_filename: Option<String> },
    ExplicitPath(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpheliaServiceInfo {
    pub service_name: String,
    pub version: String,
    pub owner: OpheliaServiceOwner,
    pub helper: OpheliaHelperInfo,
    pub profile: OpheliaProfileInfo,
    pub endpoint: OpheliaServiceEndpoint,
}

impl OpheliaServiceInfo {
    pub fn current(paths: &ProfilePaths) -> Self {
        let executable = std::env::current_exe().ok();
        let install_kind = infer_install_kind(executable.as_deref());
        let executable_sha256 = executable
            .as_deref()
            .and_then(|path| executable_sha256(path).ok());
        Self {
            service_name: OPHELIA_MACH_SERVICE_NAME.to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            owner: OpheliaServiceOwner {
                install_kind,
                executable: executable.clone(),
                pid: std::process::id(),
            },
            helper: OpheliaHelperInfo {
                install_kind,
                executable,
                pid: std::process::id(),
                executable_sha256,
            },
            profile: OpheliaProfileInfo::from_paths(paths),
            endpoint: OpheliaServiceEndpoint {
                kind: OpheliaEndpointKind::MachService,
                name: OPHELIA_MACH_SERVICE_NAME.to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpheliaServiceOwner {
    pub install_kind: OpheliaInstallKind,
    pub executable: Option<PathBuf>,
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpheliaHelperInfo {
    pub install_kind: OpheliaInstallKind,
    pub executable: Option<PathBuf>,
    pub pid: u32,
    pub executable_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpheliaInstallKind {
    AppBundle,
    HomebrewFormula,
    Development,
    Other,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpheliaProfileInfo {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub database_path: PathBuf,
    pub settings_path: PathBuf,
    pub service_lock_path: PathBuf,
    pub default_download_dir: PathBuf,
}

impl OpheliaProfileInfo {
    fn from_paths(paths: &ProfilePaths) -> Self {
        Self {
            config_dir: paths.config_dir.clone(),
            data_dir: paths.data_dir.clone(),
            logs_dir: paths.logs_dir.clone(),
            database_path: paths.database_path.clone(),
            settings_path: paths.settings_path.clone(),
            service_lock_path: paths.service_lock_path.clone(),
            default_download_dir: paths.default_download_dir.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpheliaServiceEndpoint {
    pub kind: OpheliaEndpointKind,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpheliaEndpointKind {
    MachService,
}

pub(super) fn infer_install_kind(executable: Option<&Path>) -> OpheliaInstallKind {
    let Some(executable) = executable else {
        return OpheliaInstallKind::Unknown;
    };
    let text = executable.to_string_lossy();
    if text.contains(".app/Contents/") {
        OpheliaInstallKind::AppBundle
    } else if text.starts_with("/opt/homebrew/")
        || text.starts_with("/usr/local/Homebrew/")
        || text.starts_with("/usr/local/Cellar/")
    {
        OpheliaInstallKind::HomebrewFormula
    } else if text.contains("/target/debug/") || text.contains("/target/release/") {
        OpheliaInstallKind::Development
    } else {
        OpheliaInstallKind::Other
    }
}

pub(super) fn executable_sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    sha256_reader(&mut file)
}

pub(super) fn sha256_reader(reader: &mut impl Read) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok(hex::encode(digest))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub(crate) enum OpheliaCommand {
    Add {
        request: TransferRequest,
    },
    Pause {
        id: TransferId,
    },
    Resume {
        id: TransferId,
    },
    Cancel {
        id: TransferId,
    },
    DeleteArtifact {
        id: TransferId,
    },
    UpdateSettings {
        settings: ServiceSettings,
    },
    LoadHistory {
        filter: HistoryFilter,
        query: String,
    },
    ServiceInfo,
    Snapshot,
    Subscribe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub(crate) enum OpheliaResponse {
    Ack,
    TransferAdded { id: TransferId },
    History { rows: Vec<HistoryRow> },
    ServiceInfo { info: OpheliaServiceInfo },
    Snapshot { snapshot: OpheliaSnapshot },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum OpheliaEvent {
    TransferChanged {
        snapshot: TransferSummary,
    },
    TransferBytesWritten {
        id: TransferId,
        bytes: u64,
    },
    TransferRemoved {
        id: TransferId,
        action: LiveTransferRemovalAction,
        artifact_state: ArtifactState,
    },
    ControlUnsupported {
        id: TransferId,
        action: TransferControlAction,
    },
    SettingsChanged {
        settings: ServiceSettings,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpheliaSnapshot {
    pub transfers: Vec<TransferSummary>,
    pub settings: ServiceSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum OpheliaError {
    Closed,
    NotFound {
        id: TransferId,
    },
    Unsupported {
        id: TransferId,
        action: TransferControlAction,
    },
    LockHeld {
        path: PathBuf,
    },
    StaleService {
        path: PathBuf,
    },
    ServiceApprovalRequired {
        service_name: String,
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

impl fmt::Display for OpheliaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => write!(f, "service is closed"),
            Self::NotFound { id } => write!(f, "transfer {} was not found", id.0),
            Self::Unsupported { id, action } => {
                write!(f, "transfer {} does not support {action:?}", id.0)
            }
            Self::LockHeld { path } => write!(f, "service lock is held at {}", path.display()),
            Self::StaleService { path } => write!(f, "stale service state at {}", path.display()),
            Self::ServiceApprovalRequired { .. } => {
                write!(f, "OpheliaService needs approval in System Settings")
            }
            Self::BadRequest { message } => write!(f, "bad request: {message}"),
            Self::Io { message } => write!(f, "io error: {message}"),
            Self::Transport { message } => write!(f, "transport error: {message}"),
            Self::Lagged { skipped } => write!(f, "service skipped {skipped} events"),
        }
    }
}

impl std::error::Error for OpheliaError {}

impl From<EngineError> for OpheliaError {
    fn from(error: EngineError) -> Self {
        match error {
            EngineError::Closed => Self::Closed,
            EngineError::NotFound { id } => Self::NotFound { id },
            EngineError::Unsupported { id, action } => Self::Unsupported { id, action },
        }
    }
}

impl From<io::Error> for OpheliaError {
    fn from(error: io::Error) -> Self {
        Self::Io {
            message: error.to_string(),
        }
    }
}
