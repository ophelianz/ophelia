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
    AddTransferRequest, ArtifactState, DbEvent, DownloadSpec, EngineError, HistoryFilter,
    HistoryRow, LiveTransferRemovalAction, ProgressUpdate, RestoredDownload, TransferControlAction,
    TransferControlSupport, TransferDetails, TransferId, TransferKind, TransferStatus,
    TransferSummary,
};

const SERVICE_COMMAND_CAPACITY: usize = 64;
const SERVICE_EVENT_CAPACITY: usize = 512;
const HOT_SERVICE_EVENT_FLUSH_MS: u64 = 100;
pub const OPHELIA_MACH_SERVICE_NAME: &str = "nz.ophelia.service";
pub const OPHELIA_RUN_SERVICE_ENV: &str = "OPHELIA_RUN_SERVICE";

mod client;
mod codec;
mod host;
mod lock;
#[cfg(target_os = "macos")]
mod macos_startup;
mod read_model;
mod transfer_runtime;
#[cfg(target_os = "macos")]
mod xpc;

#[cfg(test)]
mod tests;

use transfer_runtime::TransferRuntimeEvent;

pub use client::{
    LocalServiceConnection, LocalServiceOptions, LocalServiceRepairPolicy, LocalServiceWarning,
    OpheliaClient, OpheliaSubscription,
};
pub use host::OpheliaService;
pub use lock::service_lock_path;
#[cfg(target_os = "macos")]
pub use xpc::{MachServiceListener, run_mach_service};

#[cfg(target_os = "macos")]
pub fn run_default_profile_mach_service(runtime: &Handle) -> Result<(), OpheliaError> {
    let paths = ProfilePaths::default_profile();
    let service = OpheliaService::start(runtime, paths)?;
    let _listener = run_mach_service(runtime, service.client())?;
    tracing::info!(service = OPHELIA_MACH_SERVICE_NAME, "Ophelia service ready");
    runtime.block_on(service.wait());
    Ok(())
}

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
    ServiceInfo { info: Box<OpheliaServiceInfo> },
    Snapshot { snapshot: Box<OpheliaSnapshot> },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpheliaSnapshot {
    pub transfers: TransferSummaryTable,
    pub direct_details: DirectDetailsTable,
    pub settings: ServiceSettings,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpheliaUpdateBatch {
    pub lifecycle: TransferLifecycleBatch,
    pub progress_known_total: ProgressKnownTotalBatch,
    pub progress_unknown_total: ProgressUnknownTotalBatch,
    pub physical_write: PhysicalWriteBatch,
    pub destination: DestinationBatch,
    pub control_support: ControlSupportBatch,
    pub direct_details: DirectDetailsTable,
    pub removal: TransferRemovalBatch,
    pub unsupported_control: UnsupportedControlBatch,
    pub settings_changed: Option<ServiceSettings>,
}

impl OpheliaUpdateBatch {
    pub fn is_empty(&self) -> bool {
        self.lifecycle.is_empty()
            && self.progress_known_total.is_empty()
            && self.progress_unknown_total.is_empty()
            && self.physical_write.is_empty()
            && self.destination.is_empty()
            && self.control_support.is_empty()
            && self.direct_details.is_empty()
            && self.removal.is_empty()
            && self.unsupported_control.is_empty()
            && self.settings_changed.is_none()
    }

    pub fn settings_changed(settings: ServiceSettings) -> Self {
        Self {
            settings_changed: Some(settings),
            ..Self::default()
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferLifecycleCode {
    Added = 1,
    Restored = 2,
    Terminal = 3,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferSourceKindCode {
    Unknown = 0,
    DirectHttp = 1,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DirectDetailStateCode {
    Unsupported = 0,
    Loading = 1,
    Segments = 2,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferSummaryTable {
    // Hot transfer columns first; cold labels and paths stay at the end.
    pub ids: Vec<TransferId>,
    pub downloaded_bytes: Vec<u64>,
    pub speed_bytes_per_sec: Vec<u64>,
    pub total_bytes: Vec<u64>,
    pub known_total_words: Vec<u64>,
    pub kind_codes: Vec<u8>,
    pub source_kind_codes: Vec<u8>,
    pub status_codes: Vec<u8>,
    pub control_flags: Vec<u8>,
    pub source_labels: Vec<String>,
    pub destinations: Vec<PathBuf>,
}

impl TransferSummaryTable {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn push_summary(&mut self, summary: TransferSummary) {
        let row = self.ids.len();
        self.ids.push(summary.id);
        self.kind_codes.push(transfer_kind_code(summary.kind));
        self.source_kind_codes
            .push(source_kind_code(&summary.provider_kind));
        self.status_codes.push(transfer_status_code(summary.status));
        self.downloaded_bytes.push(summary.downloaded_bytes);
        self.speed_bytes_per_sec.push(summary.speed_bytes_per_sec);
        self.total_bytes.push(summary.total_bytes.unwrap_or(0));
        self.control_flags
            .push(control_support_flags(summary.control_support));
        self.source_labels.push(summary.source_label);
        self.destinations.push(summary.destination);
        self.set_total_known(row, summary.total_bytes.is_some());
    }

    pub fn replace_summary(&mut self, row: usize, summary: TransferSummary) {
        if row >= self.len() {
            return;
        }
        self.ids[row] = summary.id;
        self.kind_codes[row] = transfer_kind_code(summary.kind);
        self.source_kind_codes[row] = source_kind_code(&summary.provider_kind);
        self.status_codes[row] = transfer_status_code(summary.status);
        self.downloaded_bytes[row] = summary.downloaded_bytes;
        self.speed_bytes_per_sec[row] = summary.speed_bytes_per_sec;
        self.control_flags[row] = control_support_flags(summary.control_support);
        self.source_labels[row] = summary.source_label;
        self.destinations[row] = summary.destination;
        self.set_total(row, summary.total_bytes);
    }

    pub fn remove_row(&mut self, row: usize) {
        let old_len = self.len();
        let mut known_total_words = Vec::with_capacity(total_word_len(old_len.saturating_sub(1)));
        let mut next_row = 0;
        for old_row in 0..old_len {
            if old_row == row {
                continue;
            }
            if self.has_total(old_row) {
                set_total_bit(&mut known_total_words, next_row, true);
            }
            next_row += 1;
        }

        self.ids.remove(row);
        self.kind_codes.remove(row);
        self.source_kind_codes.remove(row);
        self.status_codes.remove(row);
        self.downloaded_bytes.remove(row);
        self.speed_bytes_per_sec.remove(row);
        self.total_bytes.remove(row);
        self.control_flags.remove(row);
        self.source_labels.remove(row);
        self.destinations.remove(row);
        self.known_total_words = known_total_words;
    }

    pub fn summary(&self, row: usize) -> Option<TransferSummary> {
        Some(TransferSummary {
            id: *self.ids.get(row)?,
            kind: transfer_kind_from_code(*self.kind_codes.get(row)?),
            provider_kind: source_kind_label(*self.source_kind_codes.get(row)?).to_string(),
            source_label: self.source_labels.get(row)?.clone(),
            destination: self.destinations.get(row)?.clone(),
            status: transfer_status_from_code(*self.status_codes.get(row)?),
            downloaded_bytes: *self.downloaded_bytes.get(row)?,
            total_bytes: self.total_bytes(row),
            speed_bytes_per_sec: *self.speed_bytes_per_sec.get(row)?,
            control_support: control_support_from_flags(*self.control_flags.get(row)?),
        })
    }

    pub fn summaries(&self) -> Vec<TransferSummary> {
        (0..self.len())
            .filter_map(|row| self.summary(row))
            .collect()
    }

    pub fn total_bytes(&self, row: usize) -> Option<u64> {
        self.has_total(row).then(|| self.total_bytes[row])
    }

    pub fn set_total(&mut self, row: usize, total: Option<u64>) {
        if row >= self.len() {
            return;
        }
        match total {
            Some(total) => {
                self.total_bytes[row] = total;
                self.set_total_known(row, true);
            }
            None => {
                self.total_bytes[row] = 0;
                self.set_total_known(row, false);
            }
        }
    }

    fn has_total(&self, row: usize) -> bool {
        if row >= self.len() {
            return false;
        }
        let word = row / u64::BITS as usize;
        let bit = row % u64::BITS as usize;
        self.known_total_words
            .get(word)
            .is_some_and(|word| word & (1u64 << bit) != 0)
    }

    fn set_total_known(&mut self, row: usize, known: bool) {
        set_total_bit(&mut self.known_total_words, row, known);
    }
}

fn total_word_len(rows: usize) -> usize {
    rows.div_ceil(u64::BITS as usize)
}

fn set_total_bit(words: &mut Vec<u64>, row: usize, known: bool) {
    let word = row / u64::BITS as usize;
    let bit = row % u64::BITS as usize;
    if words.len() <= word {
        words.resize(word + 1, 0);
    }
    if known {
        words[word] |= 1u64 << bit;
    } else {
        words[word] &= !(1u64 << bit);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectDetailsTable {
    // Segment metadata is separate from packed cell codes; state sets stay explicit.
    pub segment_ids: Vec<TransferId>,
    pub segment_total_bytes: Vec<u64>,
    pub segment_cell_offsets: Vec<u32>,
    pub segment_cell_lengths: Vec<u32>,
    pub segment_cells: Vec<u8>,
    pub unsupported_ids: Vec<TransferId>,
    pub loading_ids: Vec<TransferId>,
}

impl DirectDetailsTable {
    pub fn is_empty(&self) -> bool {
        self.unsupported_ids.is_empty()
            && self.loading_ids.is_empty()
            && self.segment_ids.is_empty()
    }

    pub fn push_state(&mut self, id: TransferId, state: crate::engine::DirectChunkMapState) {
        self.remove(id);
        match state {
            crate::engine::DirectChunkMapState::Unsupported => self.unsupported_ids.push(id),
            crate::engine::DirectChunkMapState::Loading => self.loading_ids.push(id),
            crate::engine::DirectChunkMapState::Segments(snapshot) => {
                self.segment_ids.push(id);
                self.segment_total_bytes.push(snapshot.total_bytes);
                self.segment_cell_offsets
                    .push(self.segment_cells.len() as u32);
                self.segment_cell_lengths.push(snapshot.cells.len() as u32);
                self.segment_cells
                    .extend(snapshot.cells.into_iter().map(chunk_map_cell_code));
            }
        }
    }

    pub fn push_details(&mut self, id: TransferId, details: TransferDetails) {
        match details {
            TransferDetails::Direct(details) => self.push_state(id, details.chunk_map_state),
        }
    }

    pub fn state_for(&self, id: TransferId) -> crate::engine::DirectChunkMapState {
        if self.unsupported_ids.contains(&id) {
            return crate::engine::DirectChunkMapState::Unsupported;
        }
        if self.loading_ids.contains(&id) {
            return crate::engine::DirectChunkMapState::Loading;
        }
        if let Some(index) = self.segment_ids.iter().position(|current| *current == id) {
            let offset = self.segment_cell_offsets[index] as usize;
            let len = self.segment_cell_lengths[index] as usize;
            let cells = self.segment_cells[offset..offset + len]
                .iter()
                .map(|code| chunk_map_cell_from_code(*code))
                .collect();
            return crate::engine::DirectChunkMapState::Segments(
                crate::engine::DirectChunkMapSnapshot {
                    total_bytes: self.segment_total_bytes[index],
                    cells,
                },
            );
        }
        crate::engine::DirectChunkMapState::Unsupported
    }

    pub fn remove(&mut self, id: TransferId) {
        self.unsupported_ids.retain(|current| *current != id);
        self.loading_ids.retain(|current| *current != id);
        if let Some(index) = self.segment_ids.iter().position(|current| *current == id) {
            self.segment_ids.remove(index);
            self.segment_total_bytes.remove(index);
            self.segment_cell_offsets.remove(index);
            self.segment_cell_lengths.remove(index);
            self.rebuild_segment_cells();
        }
    }

    fn rebuild_segment_cells(&mut self) {
        let states: Vec<_> = self
            .segment_ids
            .iter()
            .map(|id| self.state_for(*id))
            .collect();
        let ids = self.segment_ids.clone();
        self.segment_ids.clear();
        self.segment_total_bytes.clear();
        self.segment_cell_offsets.clear();
        self.segment_cell_lengths.clear();
        self.segment_cells.clear();
        for (id, state) in ids.into_iter().zip(states) {
            self.push_state(id, state);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferLifecycleBatch {
    pub transfers: TransferSummaryTable,
    pub lifecycle_codes: Vec<u8>,
}

impl TransferLifecycleBatch {
    pub fn is_empty(&self) -> bool {
        self.lifecycle_codes.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressKnownTotalBatch {
    pub ids: Vec<TransferId>,
    pub downloaded_bytes: Vec<u64>,
    pub total_bytes: Vec<u64>,
    pub speed_bytes_per_sec: Vec<u64>,
}

impl ProgressKnownTotalBatch {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressUnknownTotalBatch {
    pub ids: Vec<TransferId>,
    pub downloaded_bytes: Vec<u64>,
    pub speed_bytes_per_sec: Vec<u64>,
}

impl ProgressUnknownTotalBatch {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalWriteBatch {
    pub ids: Vec<TransferId>,
    pub bytes: Vec<u64>,
}

impl PhysicalWriteBatch {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationBatch {
    pub ids: Vec<TransferId>,
    pub destinations: Vec<PathBuf>,
}

impl DestinationBatch {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlSupportBatch {
    pub ids: Vec<TransferId>,
    pub control_flags: Vec<u8>,
}

impl ControlSupportBatch {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn support(&self, row: usize) -> Option<TransferControlSupport> {
        self.control_flags
            .get(row)
            .copied()
            .map(control_support_from_flags)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferRemovalBatch {
    pub ids: Vec<TransferId>,
    pub action_codes: Vec<u8>,
    pub artifact_state_codes: Vec<u8>,
}

impl TransferRemovalBatch {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn action(&self, row: usize) -> Option<LiveTransferRemovalAction> {
        self.action_codes
            .get(row)
            .copied()
            .map(removal_action_from_code)
    }

    pub fn artifact_state(&self, row: usize) -> Option<ArtifactState> {
        self.artifact_state_codes
            .get(row)
            .copied()
            .map(artifact_state_from_code)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedControlBatch {
    pub ids: Vec<TransferId>,
    pub action_codes: Vec<u8>,
}

impl UnsupportedControlBatch {
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn action(&self, row: usize) -> Option<TransferControlAction> {
        self.action_codes
            .get(row)
            .copied()
            .map(control_action_from_code)
    }
}

pub(crate) fn transfer_kind_code(kind: TransferKind) -> u8 {
    kind as u8
}

pub(crate) fn transfer_kind_from_code(code: u8) -> TransferKind {
    match code {
        0 => TransferKind::Direct,
        _ => TransferKind::Direct,
    }
}

pub(crate) fn source_kind_code(kind: &str) -> u8 {
    match kind {
        "http" => TransferSourceKindCode::DirectHttp as u8,
        _ => TransferSourceKindCode::Unknown as u8,
    }
}

pub(crate) fn source_kind_label(code: u8) -> &'static str {
    match code {
        code if code == TransferSourceKindCode::DirectHttp as u8 => "http",
        _ => "unknown",
    }
}

pub(crate) fn transfer_status_code(status: TransferStatus) -> u8 {
    status as u8
}

pub(crate) fn transfer_status_from_code(code: u8) -> TransferStatus {
    match code {
        0 => TransferStatus::Pending,
        1 => TransferStatus::Downloading,
        2 => TransferStatus::Paused,
        3 => TransferStatus::Finished,
        4 => TransferStatus::Error,
        5 => TransferStatus::Cancelled,
        _ => TransferStatus::Error,
    }
}

pub(crate) fn control_support_flags(support: TransferControlSupport) -> u8 {
    u8::from(support.can_pause)
        | (u8::from(support.can_resume) << 1)
        | (u8::from(support.can_cancel) << 2)
        | (u8::from(support.can_restore) << 3)
}

pub(crate) fn control_support_from_flags(flags: u8) -> TransferControlSupport {
    TransferControlSupport {
        can_pause: flags & 1 != 0,
        can_resume: flags & (1 << 1) != 0,
        can_cancel: flags & (1 << 2) != 0,
        can_restore: flags & (1 << 3) != 0,
    }
}

pub(crate) fn chunk_map_cell_code(state: crate::engine::ChunkMapCellState) -> u8 {
    state as u8
}

pub(crate) fn chunk_map_cell_from_code(code: u8) -> crate::engine::ChunkMapCellState {
    match code {
        0 => crate::engine::ChunkMapCellState::Empty,
        1 => crate::engine::ChunkMapCellState::Partial,
        2 => crate::engine::ChunkMapCellState::Complete,
        _ => crate::engine::ChunkMapCellState::Empty,
    }
}

pub(crate) fn control_action_code(action: TransferControlAction) -> u8 {
    action as u8
}

pub(crate) fn control_action_from_code(code: u8) -> TransferControlAction {
    match code {
        0 => TransferControlAction::Pause,
        1 => TransferControlAction::Resume,
        2 => TransferControlAction::Cancel,
        3 => TransferControlAction::Restore,
        _ => TransferControlAction::Cancel,
    }
}

pub(crate) fn artifact_state_code(state: ArtifactState) -> u8 {
    state as u8
}

pub(crate) fn artifact_state_from_code(code: u8) -> ArtifactState {
    match code {
        0 => ArtifactState::Present,
        1 => ArtifactState::Deleted,
        2 => ArtifactState::Missing,
        _ => ArtifactState::Missing,
    }
}

pub(crate) fn removal_action_code(action: LiveTransferRemovalAction) -> u8 {
    action as u8
}

pub(crate) fn removal_action_from_code(code: u8) -> LiveTransferRemovalAction {
    match code {
        0 => LiveTransferRemovalAction::Cancelled,
        1 => LiveTransferRemovalAction::DeleteArtifact,
        _ => LiveTransferRemovalAction::Cancelled,
    }
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
