use std::path::PathBuf;

use crate::config::{CollisionPolicy, HttpOrderingMode, ServiceDestinationRule, ServiceSettings};

use super::*;

const CODEC_VERSION: u16 = 1;
const FRAME_RESPONSE: u8 = 1;
const FRAME_ERROR: u8 = 2;
const FRAME_UPDATE: u8 = 3;

#[derive(Debug)]
pub(crate) struct OpheliaCommandEnvelope {
    pub id: u64,
    pub command: OpheliaCommand,
}

#[derive(Debug)]
pub(crate) enum OpheliaFrameEnvelope {
    Response {
        id: u64,
        response: Box<OpheliaResponse>,
    },
    Error {
        id: u64,
        error: OpheliaError,
    },
    Update {
        update: Box<OpheliaUpdateBatch>,
    },
}

pub(super) fn command_to_body(command: &OpheliaCommandEnvelope) -> Result<Vec<u8>, OpheliaError> {
    let mut writer = CodecWriter::new();
    writer.u16(CODEC_VERSION);
    writer.u64(command.id);
    writer.command(&command.command)?;
    Ok(writer.finish())
}

pub(super) fn frame_to_body(frame: &OpheliaFrameEnvelope) -> Result<Vec<u8>, OpheliaError> {
    let mut writer = CodecWriter::new();
    writer.u16(CODEC_VERSION);
    match frame {
        OpheliaFrameEnvelope::Response { id, response } => {
            writer.u8(FRAME_RESPONSE);
            writer.u64(*id);
            writer.response(response)?;
        }
        OpheliaFrameEnvelope::Error { id, error } => {
            writer.u8(FRAME_ERROR);
            writer.u64(*id);
            writer.error(error)?;
        }
        OpheliaFrameEnvelope::Update { update } => {
            writer.u8(FRAME_UPDATE);
            writer.update_batch(update)?;
        }
    }
    Ok(writer.finish())
}

pub(super) fn command_from_body(body: &[u8]) -> Result<OpheliaCommandEnvelope, OpheliaError> {
    let mut reader = CodecReader::new(body);
    reader.expect_version()?;
    let id = reader.u64()?;
    let command = reader.command()?;
    reader.finish()?;
    Ok(OpheliaCommandEnvelope { id, command })
}

pub(super) fn frame_from_body(body: &[u8]) -> Result<OpheliaFrameEnvelope, OpheliaError> {
    let mut reader = CodecReader::new(body);
    reader.expect_version()?;
    let frame = match reader.u8()? {
        FRAME_RESPONSE => {
            let id = reader.u64()?;
            let response = reader.response()?;
            OpheliaFrameEnvelope::Response {
                id,
                response: Box::new(response),
            }
        }
        FRAME_ERROR => {
            let id = reader.u64()?;
            let error = reader.error()?;
            OpheliaFrameEnvelope::Error { id, error }
        }
        FRAME_UPDATE => {
            let update = reader.update_batch()?;
            OpheliaFrameEnvelope::Update {
                update: Box::new(update),
            }
        }
        other => {
            return Err(OpheliaError::BadRequest {
                message: format!("unknown service frame code {other}"),
            });
        }
    };
    reader.finish()?;
    Ok(frame)
}

pub(super) fn unexpected_xpc_frame(expected: &str, frame: OpheliaFrameEnvelope) -> OpheliaError {
    OpheliaError::Transport {
        message: format!("expected service {expected}, got {frame:?}"),
    }
}

struct CodecWriter {
    bytes: Vec<u8>,
}

impl CodecWriter {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn string(&mut self, value: &str) -> Result<(), OpheliaError> {
        let bytes = value.as_bytes();
        self.len(bytes.len())?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn path(&mut self, value: &std::path::Path) -> Result<(), OpheliaError> {
        self.string(&value.to_string_lossy())
    }

    fn option_string(&mut self, value: Option<&str>) -> Result<(), OpheliaError> {
        match value {
            Some(value) => {
                self.bool(true);
                self.string(value)
            }
            None => {
                self.bool(false);
                Ok(())
            }
        }
    }

    fn option_path(&mut self, value: Option<&std::path::Path>) -> Result<(), OpheliaError> {
        match value {
            Some(value) => {
                self.bool(true);
                self.path(value)
            }
            None => {
                self.bool(false);
                Ok(())
            }
        }
    }

    fn len(&mut self, len: usize) -> Result<(), OpheliaError> {
        let len = u32::try_from(len).map_err(|_| OpheliaError::Transport {
            message: "service binary value exceeded u32 length".into(),
        })?;
        self.u32(len);
        Ok(())
    }

    fn command(&mut self, command: &OpheliaCommand) -> Result<(), OpheliaError> {
        match command {
            OpheliaCommand::Add { request } => {
                self.u8(1);
                self.transfer_request(request)
            }
            OpheliaCommand::Pause { id } => self.transfer_id_command(2, *id),
            OpheliaCommand::Resume { id } => self.transfer_id_command(3, *id),
            OpheliaCommand::Cancel { id } => self.transfer_id_command(4, *id),
            OpheliaCommand::DeleteArtifact { id } => self.transfer_id_command(5, *id),
            OpheliaCommand::UpdateSettings { settings } => {
                self.u8(6);
                self.settings(settings)
            }
            OpheliaCommand::LoadHistory { filter, query } => {
                self.u8(7);
                self.u8(*filter as u8);
                self.string(query)
            }
            OpheliaCommand::ServiceInfo => {
                self.u8(8);
                Ok(())
            }
            OpheliaCommand::Snapshot => {
                self.u8(9);
                Ok(())
            }
            OpheliaCommand::Subscribe => {
                self.u8(10);
                Ok(())
            }
        }
    }

    fn transfer_id_command(&mut self, code: u8, id: TransferId) -> Result<(), OpheliaError> {
        self.u8(code);
        self.u64(id.0);
        Ok(())
    }

    fn response(&mut self, response: &OpheliaResponse) -> Result<(), OpheliaError> {
        match response {
            OpheliaResponse::Ack => {
                self.u8(1);
                Ok(())
            }
            OpheliaResponse::TransferAdded { id } => {
                self.u8(2);
                self.u64(id.0);
                Ok(())
            }
            OpheliaResponse::History { rows } => {
                self.u8(3);
                self.history_rows(rows)
            }
            OpheliaResponse::ServiceInfo { info } => {
                self.u8(4);
                self.service_info(info)
            }
            OpheliaResponse::Snapshot { snapshot } => {
                self.u8(5);
                self.snapshot(snapshot)
            }
        }
    }

    fn error(&mut self, error: &OpheliaError) -> Result<(), OpheliaError> {
        match error {
            OpheliaError::Closed => {
                self.u8(1);
                Ok(())
            }
            OpheliaError::NotFound { id } => {
                self.u8(2);
                self.u64(id.0);
                Ok(())
            }
            OpheliaError::Unsupported { id, action } => {
                self.u8(3);
                self.u64(id.0);
                self.u8(*action as u8);
                Ok(())
            }
            OpheliaError::LockHeld { path } => {
                self.u8(4);
                self.path(path)
            }
            OpheliaError::StaleService { path } => {
                self.u8(5);
                self.path(path)
            }
            OpheliaError::ServiceApprovalRequired { service_name } => {
                self.u8(6);
                self.string(service_name)
            }
            OpheliaError::BadRequest { message } => {
                self.u8(7);
                self.string(message)
            }
            OpheliaError::Io { message } => {
                self.u8(8);
                self.string(message)
            }
            OpheliaError::Transport { message } => {
                self.u8(9);
                self.string(message)
            }
            OpheliaError::Lagged { skipped } => {
                self.u8(10);
                self.u64(*skipped);
                Ok(())
            }
        }
    }

    fn transfer_request(&mut self, request: &TransferRequest) -> Result<(), OpheliaError> {
        match &request.source {
            TransferRequestSource::Http { url } => {
                self.u8(1);
                self.string(url)?;
            }
        }
        match &request.destination {
            TransferDestination::Automatic { suggested_filename } => {
                self.u8(1);
                self.option_string(suggested_filename.as_deref())
            }
            TransferDestination::ExplicitPath(path) => {
                self.u8(2);
                self.path(path)
            }
        }
    }

    fn settings(&mut self, settings: &ServiceSettings) -> Result<(), OpheliaError> {
        self.u64(settings.max_connections_per_server as u64);
        self.u64(settings.max_connections_per_download as u64);
        self.u64(settings.max_concurrent_transfers as u64);
        self.option_path(settings.default_download_dir.as_deref())?;
        self.u64(settings.global_speed_limit_bps);
        self.u8(collision_policy_code(settings.collision_policy));
        self.bool(settings.destination_rules_enabled);
        self.len(settings.destination_rules.len())?;
        for rule in &settings.destination_rules {
            self.destination_rule(rule)?;
        }
        self.u8(http_ordering_code(settings.http_ordering_mode));
        self.len(settings.sequential_download_extensions.len())?;
        for extension in &settings.sequential_download_extensions {
            self.string(extension)?;
        }
        Ok(())
    }

    fn destination_rule(&mut self, rule: &ServiceDestinationRule) -> Result<(), OpheliaError> {
        self.string(&rule.id)?;
        self.string(&rule.label)?;
        self.bool(rule.enabled);
        self.path(&rule.target_dir)?;
        self.len(rule.extensions.len())?;
        for extension in &rule.extensions {
            self.string(extension)?;
        }
        self.option_string(rule.icon_name.as_deref())
    }

    fn history_rows(&mut self, rows: &[HistoryRow]) -> Result<(), OpheliaError> {
        self.len(rows.len())?;
        for row in rows {
            self.u64(row.id.0);
            self.string(&row.provider_kind)?;
            self.string(&row.source_label)?;
            self.string(&row.destination)?;
            self.u8(row.status as u8);
            self.u8(row.artifact_state as u8);
            match row.total_bytes {
                Some(total) => {
                    self.bool(true);
                    self.u64(total);
                }
                None => self.bool(false),
            }
            self.u64(row.downloaded_bytes);
            self.i64(row.added_at);
            match row.finished_at {
                Some(finished_at) => {
                    self.bool(true);
                    self.i64(finished_at);
                }
                None => self.bool(false),
            }
        }
        Ok(())
    }

    fn service_info(&mut self, info: &OpheliaServiceInfo) -> Result<(), OpheliaError> {
        self.string(&info.service_name)?;
        self.string(&info.version)?;
        self.service_owner(&info.owner)?;
        self.helper_info(&info.helper)?;
        self.profile_info(&info.profile)?;
        self.u8(endpoint_kind_code(info.endpoint.kind));
        self.string(&info.endpoint.name)
    }

    fn service_owner(&mut self, owner: &OpheliaServiceOwner) -> Result<(), OpheliaError> {
        self.u8(install_kind_code(owner.install_kind));
        self.option_path(owner.executable.as_deref())?;
        self.u64(owner.pid as u64);
        Ok(())
    }

    fn helper_info(&mut self, helper: &OpheliaHelperInfo) -> Result<(), OpheliaError> {
        self.u8(install_kind_code(helper.install_kind));
        self.option_path(helper.executable.as_deref())?;
        self.u64(helper.pid as u64);
        self.option_string(helper.executable_sha256.as_deref())
    }

    fn profile_info(&mut self, profile: &OpheliaProfileInfo) -> Result<(), OpheliaError> {
        self.path(&profile.config_dir)?;
        self.path(&profile.data_dir)?;
        self.path(&profile.logs_dir)?;
        self.path(&profile.database_path)?;
        self.path(&profile.settings_path)?;
        self.path(&profile.service_lock_path)?;
        self.path(&profile.default_download_dir)
    }

    fn snapshot(&mut self, snapshot: &OpheliaSnapshot) -> Result<(), OpheliaError> {
        self.transfer_table(&snapshot.transfers)?;
        self.direct_details_table(&snapshot.direct_details)?;
        self.settings(&snapshot.settings)
    }

    fn update_batch(&mut self, batch: &OpheliaUpdateBatch) -> Result<(), OpheliaError> {
        self.transfer_table(&batch.lifecycle.transfers)?;
        self.bytes(&batch.lifecycle.lifecycle_codes)?;
        self.progress_known(&batch.progress_known_total)?;
        self.progress_unknown(&batch.progress_unknown_total)?;
        self.physical_write(&batch.physical_write)?;
        self.destination_batch(&batch.destination)?;
        self.control_support_batch(&batch.control_support)?;
        self.direct_details_table(&batch.direct_details)?;
        self.removal_batch(&batch.removal)?;
        self.unsupported_control_batch(&batch.unsupported_control)?;
        match &batch.settings_changed {
            Some(settings) => {
                self.bool(true);
                self.settings(settings)
            }
            None => {
                self.bool(false);
                Ok(())
            }
        }
    }

    fn transfer_table(&mut self, table: &TransferSummaryTable) -> Result<(), OpheliaError> {
        self.transfer_ids(&table.ids)?;
        self.u64s(&table.downloaded_bytes)?;
        self.u64s(&table.speed_bytes_per_sec)?;
        self.u64s(&table.known_total_bytes)?;
        self.u32s(&table.known_total_rows)?;
        self.bytes(&table.kind_codes)?;
        self.bytes(&table.source_kind_codes)?;
        self.bytes(&table.status_codes)?;
        self.bytes(&table.control_flags)?;
        self.strings(&table.source_labels)?;
        self.paths(&table.destinations)
    }

    fn direct_details_table(&mut self, table: &DirectDetailsTable) -> Result<(), OpheliaError> {
        self.transfer_ids(&table.segment_ids)?;
        self.u64s(&table.segment_total_bytes)?;
        self.u32s(&table.segment_cell_offsets)?;
        self.u32s(&table.segment_cell_lengths)?;
        self.bytes(&table.segment_cells)?;
        self.transfer_ids(&table.unsupported_ids)?;
        self.transfer_ids(&table.loading_ids)
    }

    fn progress_known(&mut self, batch: &ProgressKnownTotalBatch) -> Result<(), OpheliaError> {
        self.transfer_ids(&batch.ids)?;
        self.u64s(&batch.downloaded_bytes)?;
        self.u64s(&batch.total_bytes)?;
        self.u64s(&batch.speed_bytes_per_sec)
    }

    fn progress_unknown(&mut self, batch: &ProgressUnknownTotalBatch) -> Result<(), OpheliaError> {
        self.transfer_ids(&batch.ids)?;
        self.u64s(&batch.downloaded_bytes)?;
        self.u64s(&batch.speed_bytes_per_sec)
    }

    fn physical_write(&mut self, batch: &PhysicalWriteBatch) -> Result<(), OpheliaError> {
        self.transfer_ids(&batch.ids)?;
        self.u64s(&batch.bytes)
    }

    fn destination_batch(&mut self, batch: &DestinationBatch) -> Result<(), OpheliaError> {
        self.transfer_ids(&batch.ids)?;
        self.paths(&batch.destinations)
    }

    fn control_support_batch(&mut self, batch: &ControlSupportBatch) -> Result<(), OpheliaError> {
        self.transfer_ids(&batch.ids)?;
        self.bytes(&batch.control_flags)
    }

    fn removal_batch(&mut self, batch: &TransferRemovalBatch) -> Result<(), OpheliaError> {
        self.transfer_ids(&batch.ids)?;
        self.bytes(&batch.action_codes)?;
        self.bytes(&batch.artifact_state_codes)
    }

    fn unsupported_control_batch(
        &mut self,
        batch: &UnsupportedControlBatch,
    ) -> Result<(), OpheliaError> {
        self.transfer_ids(&batch.ids)?;
        self.bytes(&batch.action_codes)
    }

    fn transfer_ids(&mut self, ids: &[TransferId]) -> Result<(), OpheliaError> {
        self.len(ids.len())?;
        for id in ids {
            self.u64(id.0);
        }
        Ok(())
    }

    fn u32s(&mut self, values: &[u32]) -> Result<(), OpheliaError> {
        self.len(values.len())?;
        for value in values {
            self.u32(*value);
        }
        Ok(())
    }

    fn u64s(&mut self, values: &[u64]) -> Result<(), OpheliaError> {
        self.len(values.len())?;
        for value in values {
            self.u64(*value);
        }
        Ok(())
    }

    fn bytes(&mut self, values: &[u8]) -> Result<(), OpheliaError> {
        self.len(values.len())?;
        self.bytes.extend_from_slice(values);
        Ok(())
    }

    fn strings(&mut self, values: &[String]) -> Result<(), OpheliaError> {
        self.len(values.len())?;
        for value in values {
            self.string(value)?;
        }
        Ok(())
    }

    fn paths(&mut self, values: &[PathBuf]) -> Result<(), OpheliaError> {
        self.len(values.len())?;
        for value in values {
            self.path(value)?;
        }
        Ok(())
    }
}

struct CodecReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> CodecReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn expect_version(&mut self) -> Result<(), OpheliaError> {
        let version = self.u16()?;
        if version == CODEC_VERSION {
            Ok(())
        } else {
            Err(OpheliaError::BadRequest {
                message: format!("unsupported service binary version {version}"),
            })
        }
    }

    fn finish(&self) -> Result<(), OpheliaError> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err(OpheliaError::BadRequest {
                message: "service binary body had trailing bytes".into(),
            })
        }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], OpheliaError> {
        let end = self
            .cursor
            .checked_add(len)
            .ok_or_else(|| OpheliaError::BadRequest {
                message: "service binary length overflow".into(),
            })?;
        let bytes = self
            .bytes
            .get(self.cursor..end)
            .ok_or_else(|| OpheliaError::BadRequest {
                message: "truncated service binary body".into(),
            })?;
        self.cursor = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, OpheliaError> {
        Ok(*self.take(1)?.first().expect("one byte requested"))
    }

    fn bool(&mut self) -> Result<bool, OpheliaError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(OpheliaError::BadRequest {
                message: format!("invalid service binary bool {other}"),
            }),
        }
    }

    fn u16(&mut self) -> Result<u16, OpheliaError> {
        let bytes: [u8; 2] = self.take(2)?.try_into().expect("fixed length");
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, OpheliaError> {
        let bytes: [u8; 4] = self.take(4)?.try_into().expect("fixed length");
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, OpheliaError> {
        let bytes: [u8; 8] = self.take(8)?.try_into().expect("fixed length");
        Ok(u64::from_le_bytes(bytes))
    }

    fn i64(&mut self) -> Result<i64, OpheliaError> {
        let bytes: [u8; 8] = self.take(8)?.try_into().expect("fixed length");
        Ok(i64::from_le_bytes(bytes))
    }

    fn string(&mut self) -> Result<String, OpheliaError> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|error| OpheliaError::BadRequest {
                message: format!("invalid service binary string: {error}"),
            })
    }

    fn path(&mut self) -> Result<PathBuf, OpheliaError> {
        Ok(PathBuf::from(self.string()?))
    }

    fn option_string(&mut self) -> Result<Option<String>, OpheliaError> {
        self.bool()?.then(|| self.string()).transpose()
    }

    fn option_path(&mut self) -> Result<Option<PathBuf>, OpheliaError> {
        self.bool()?.then(|| self.path()).transpose()
    }

    fn command(&mut self) -> Result<OpheliaCommand, OpheliaError> {
        Ok(match self.u8()? {
            1 => OpheliaCommand::Add {
                request: self.transfer_request()?,
            },
            2 => OpheliaCommand::Pause {
                id: TransferId(self.u64()?),
            },
            3 => OpheliaCommand::Resume {
                id: TransferId(self.u64()?),
            },
            4 => OpheliaCommand::Cancel {
                id: TransferId(self.u64()?),
            },
            5 => OpheliaCommand::DeleteArtifact {
                id: TransferId(self.u64()?),
            },
            6 => OpheliaCommand::UpdateSettings {
                settings: self.settings()?,
            },
            7 => OpheliaCommand::LoadHistory {
                filter: history_filter_from_code(self.u8()?),
                query: self.string()?,
            },
            8 => OpheliaCommand::ServiceInfo,
            9 => OpheliaCommand::Snapshot,
            10 => OpheliaCommand::Subscribe,
            other => {
                return Err(OpheliaError::BadRequest {
                    message: format!("unknown service command code {other}"),
                });
            }
        })
    }

    fn response(&mut self) -> Result<OpheliaResponse, OpheliaError> {
        Ok(match self.u8()? {
            1 => OpheliaResponse::Ack,
            2 => OpheliaResponse::TransferAdded {
                id: TransferId(self.u64()?),
            },
            3 => OpheliaResponse::History {
                rows: self.history_rows()?,
            },
            4 => OpheliaResponse::ServiceInfo {
                info: Box::new(self.service_info()?),
            },
            5 => OpheliaResponse::Snapshot {
                snapshot: Box::new(self.snapshot()?),
            },
            other => {
                return Err(OpheliaError::BadRequest {
                    message: format!("unknown service response code {other}"),
                });
            }
        })
    }

    fn error(&mut self) -> Result<OpheliaError, OpheliaError> {
        Ok(match self.u8()? {
            1 => OpheliaError::Closed,
            2 => OpheliaError::NotFound {
                id: TransferId(self.u64()?),
            },
            3 => OpheliaError::Unsupported {
                id: TransferId(self.u64()?),
                action: super::control_action_from_code(self.u8()?),
            },
            4 => OpheliaError::LockHeld { path: self.path()? },
            5 => OpheliaError::StaleService { path: self.path()? },
            6 => OpheliaError::ServiceApprovalRequired {
                service_name: self.string()?,
            },
            7 => OpheliaError::BadRequest {
                message: self.string()?,
            },
            8 => OpheliaError::Io {
                message: self.string()?,
            },
            9 => OpheliaError::Transport {
                message: self.string()?,
            },
            10 => OpheliaError::Lagged {
                skipped: self.u64()?,
            },
            other => {
                return Err(OpheliaError::BadRequest {
                    message: format!("unknown service error code {other}"),
                });
            }
        })
    }

    fn transfer_request(&mut self) -> Result<TransferRequest, OpheliaError> {
        let source = match self.u8()? {
            1 => TransferRequestSource::Http {
                url: self.string()?,
            },
            other => {
                return Err(OpheliaError::BadRequest {
                    message: format!("unknown transfer source code {other}"),
                });
            }
        };
        let destination = match self.u8()? {
            1 => TransferDestination::Automatic {
                suggested_filename: self.option_string()?,
            },
            2 => TransferDestination::ExplicitPath(self.path()?),
            other => {
                return Err(OpheliaError::BadRequest {
                    message: format!("unknown transfer destination code {other}"),
                });
            }
        };
        Ok(TransferRequest {
            source,
            destination,
        })
    }

    fn settings(&mut self) -> Result<ServiceSettings, OpheliaError> {
        let max_connections_per_server = self.usize()?;
        let max_connections_per_download = self.usize()?;
        let max_concurrent_transfers = self.usize()?;
        let default_download_dir = self.option_path()?;
        let global_speed_limit_bps = self.u64()?;
        let collision_policy = collision_policy_from_code(self.u8()?);
        let destination_rules_enabled = self.bool()?;
        let mut destination_rules = Vec::with_capacity(self.vec_len()?);
        for _ in 0..destination_rules.capacity() {
            destination_rules.push(self.destination_rule()?);
        }
        let http_ordering_mode = http_ordering_from_code(self.u8()?);
        let mut sequential_download_extensions = Vec::with_capacity(self.vec_len()?);
        for _ in 0..sequential_download_extensions.capacity() {
            sequential_download_extensions.push(self.string()?);
        }
        Ok(ServiceSettings {
            max_connections_per_server,
            max_connections_per_download,
            max_concurrent_transfers,
            default_download_dir,
            global_speed_limit_bps,
            collision_policy,
            destination_rules_enabled,
            destination_rules,
            http_ordering_mode,
            sequential_download_extensions,
        })
    }

    fn destination_rule(&mut self) -> Result<ServiceDestinationRule, OpheliaError> {
        let id = self.string()?;
        let label = self.string()?;
        let enabled = self.bool()?;
        let target_dir = self.path()?;
        let mut extensions = Vec::with_capacity(self.vec_len()?);
        for _ in 0..extensions.capacity() {
            extensions.push(self.string()?);
        }
        let icon_name = self.option_string()?;
        Ok(ServiceDestinationRule {
            id,
            label,
            enabled,
            target_dir,
            extensions,
            icon_name,
        })
    }

    fn history_rows(&mut self) -> Result<Vec<HistoryRow>, OpheliaError> {
        let mut rows = Vec::with_capacity(self.vec_len()?);
        for _ in 0..rows.capacity() {
            rows.push(HistoryRow {
                id: TransferId(self.u64()?),
                provider_kind: self.string()?,
                source_label: self.string()?,
                destination: self.string()?,
                status: super::transfer_status_from_code(self.u8()?),
                artifact_state: super::artifact_state_from_code(self.u8()?),
                total_bytes: self.bool()?.then(|| self.u64()).transpose()?,
                downloaded_bytes: self.u64()?,
                added_at: self.i64()?,
                finished_at: self.bool()?.then(|| self.i64()).transpose()?,
            });
        }
        Ok(rows)
    }

    fn service_info(&mut self) -> Result<OpheliaServiceInfo, OpheliaError> {
        let service_name = self.string()?;
        let version = self.string()?;
        let owner = self.service_owner()?;
        let helper = self.helper_info()?;
        let profile = self.profile_info()?;
        let endpoint = OpheliaServiceEndpoint {
            kind: endpoint_kind_from_code(self.u8()?),
            name: self.string()?,
        };
        Ok(OpheliaServiceInfo {
            service_name,
            version,
            owner,
            helper,
            profile,
            endpoint,
        })
    }

    fn service_owner(&mut self) -> Result<OpheliaServiceOwner, OpheliaError> {
        Ok(OpheliaServiceOwner {
            install_kind: install_kind_from_code(self.u8()?),
            executable: self.option_path()?,
            pid: self.u64()? as u32,
        })
    }

    fn helper_info(&mut self) -> Result<OpheliaHelperInfo, OpheliaError> {
        Ok(OpheliaHelperInfo {
            install_kind: install_kind_from_code(self.u8()?),
            executable: self.option_path()?,
            pid: self.u64()? as u32,
            executable_sha256: self.option_string()?,
        })
    }

    fn profile_info(&mut self) -> Result<OpheliaProfileInfo, OpheliaError> {
        Ok(OpheliaProfileInfo {
            config_dir: self.path()?,
            data_dir: self.path()?,
            logs_dir: self.path()?,
            database_path: self.path()?,
            settings_path: self.path()?,
            service_lock_path: self.path()?,
            default_download_dir: self.path()?,
        })
    }

    fn snapshot(&mut self) -> Result<OpheliaSnapshot, OpheliaError> {
        Ok(OpheliaSnapshot {
            transfers: self.transfer_table()?,
            direct_details: self.direct_details_table()?,
            settings: self.settings()?,
        })
    }

    fn update_batch(&mut self) -> Result<OpheliaUpdateBatch, OpheliaError> {
        let lifecycle = TransferLifecycleBatch {
            transfers: self.transfer_table()?,
            lifecycle_codes: self.bytes()?,
        };
        let progress_known_total = self.progress_known()?;
        let progress_unknown_total = self.progress_unknown()?;
        let physical_write = self.physical_write()?;
        let destination = self.destination_batch()?;
        let control_support = self.control_support_batch()?;
        let direct_details = self.direct_details_table()?;
        let removal = self.removal_batch()?;
        let unsupported_control = self.unsupported_control_batch()?;
        let settings_changed = self.bool()?.then(|| self.settings()).transpose()?;
        Ok(OpheliaUpdateBatch {
            lifecycle,
            progress_known_total,
            progress_unknown_total,
            physical_write,
            destination,
            control_support,
            direct_details,
            removal,
            unsupported_control,
            settings_changed,
        })
    }

    fn transfer_table(&mut self) -> Result<TransferSummaryTable, OpheliaError> {
        Ok(TransferSummaryTable {
            ids: self.transfer_ids()?,
            downloaded_bytes: self.u64s()?,
            speed_bytes_per_sec: self.u64s()?,
            known_total_bytes: self.u64s()?,
            known_total_rows: self.u32s()?,
            kind_codes: self.bytes()?,
            source_kind_codes: self.bytes()?,
            status_codes: self.bytes()?,
            control_flags: self.bytes()?,
            source_labels: self.strings()?,
            destinations: self.paths()?,
        })
    }

    fn direct_details_table(&mut self) -> Result<DirectDetailsTable, OpheliaError> {
        Ok(DirectDetailsTable {
            segment_ids: self.transfer_ids()?,
            segment_total_bytes: self.u64s()?,
            segment_cell_offsets: self.u32s()?,
            segment_cell_lengths: self.u32s()?,
            segment_cells: self.bytes()?,
            unsupported_ids: self.transfer_ids()?,
            loading_ids: self.transfer_ids()?,
        })
    }

    fn progress_known(&mut self) -> Result<ProgressKnownTotalBatch, OpheliaError> {
        Ok(ProgressKnownTotalBatch {
            ids: self.transfer_ids()?,
            downloaded_bytes: self.u64s()?,
            total_bytes: self.u64s()?,
            speed_bytes_per_sec: self.u64s()?,
        })
    }

    fn progress_unknown(&mut self) -> Result<ProgressUnknownTotalBatch, OpheliaError> {
        Ok(ProgressUnknownTotalBatch {
            ids: self.transfer_ids()?,
            downloaded_bytes: self.u64s()?,
            speed_bytes_per_sec: self.u64s()?,
        })
    }

    fn physical_write(&mut self) -> Result<PhysicalWriteBatch, OpheliaError> {
        Ok(PhysicalWriteBatch {
            ids: self.transfer_ids()?,
            bytes: self.u64s()?,
        })
    }

    fn destination_batch(&mut self) -> Result<DestinationBatch, OpheliaError> {
        Ok(DestinationBatch {
            ids: self.transfer_ids()?,
            destinations: self.paths()?,
        })
    }

    fn control_support_batch(&mut self) -> Result<ControlSupportBatch, OpheliaError> {
        Ok(ControlSupportBatch {
            ids: self.transfer_ids()?,
            control_flags: self.bytes()?,
        })
    }

    fn removal_batch(&mut self) -> Result<TransferRemovalBatch, OpheliaError> {
        Ok(TransferRemovalBatch {
            ids: self.transfer_ids()?,
            action_codes: self.bytes()?,
            artifact_state_codes: self.bytes()?,
        })
    }

    fn unsupported_control_batch(&mut self) -> Result<UnsupportedControlBatch, OpheliaError> {
        Ok(UnsupportedControlBatch {
            ids: self.transfer_ids()?,
            action_codes: self.bytes()?,
        })
    }

    fn transfer_ids(&mut self) -> Result<Vec<TransferId>, OpheliaError> {
        let mut ids = Vec::with_capacity(self.vec_len()?);
        for _ in 0..ids.capacity() {
            ids.push(TransferId(self.u64()?));
        }
        Ok(ids)
    }

    fn bytes(&mut self) -> Result<Vec<u8>, OpheliaError> {
        let len = self.vec_len()?;
        Ok(self.take(len)?.to_vec())
    }

    fn u32s(&mut self) -> Result<Vec<u32>, OpheliaError> {
        let mut values = Vec::with_capacity(self.vec_len()?);
        for _ in 0..values.capacity() {
            values.push(self.u32()?);
        }
        Ok(values)
    }

    fn u64s(&mut self) -> Result<Vec<u64>, OpheliaError> {
        let mut values = Vec::with_capacity(self.vec_len()?);
        for _ in 0..values.capacity() {
            values.push(self.u64()?);
        }
        Ok(values)
    }

    fn strings(&mut self) -> Result<Vec<String>, OpheliaError> {
        let mut values = Vec::with_capacity(self.vec_len()?);
        for _ in 0..values.capacity() {
            values.push(self.string()?);
        }
        Ok(values)
    }

    fn paths(&mut self) -> Result<Vec<PathBuf>, OpheliaError> {
        let mut values = Vec::with_capacity(self.vec_len()?);
        for _ in 0..values.capacity() {
            values.push(self.path()?);
        }
        Ok(values)
    }

    fn vec_len(&mut self) -> Result<usize, OpheliaError> {
        Ok(self.u32()? as usize)
    }

    fn usize(&mut self) -> Result<usize, OpheliaError> {
        usize::try_from(self.u64()?).map_err(|_| OpheliaError::BadRequest {
            message: "service binary usize overflow".into(),
        })
    }
}

fn history_filter_from_code(code: u8) -> HistoryFilter {
    match code {
        0 => HistoryFilter::All,
        1 => HistoryFilter::Finished,
        2 => HistoryFilter::Error,
        3 => HistoryFilter::Paused,
        4 => HistoryFilter::Cancelled,
        _ => HistoryFilter::All,
    }
}

fn collision_policy_code(policy: CollisionPolicy) -> u8 {
    match policy {
        CollisionPolicy::Rename => 0,
        CollisionPolicy::Replace => 1,
    }
}

fn collision_policy_from_code(code: u8) -> CollisionPolicy {
    match code {
        1 => CollisionPolicy::Replace,
        _ => CollisionPolicy::Rename,
    }
}

fn http_ordering_code(mode: HttpOrderingMode) -> u8 {
    match mode {
        HttpOrderingMode::Balanced => 0,
        HttpOrderingMode::FileSpecific => 1,
        HttpOrderingMode::Sequential => 2,
    }
}

fn http_ordering_from_code(code: u8) -> HttpOrderingMode {
    match code {
        0 => HttpOrderingMode::Balanced,
        2 => HttpOrderingMode::Sequential,
        _ => HttpOrderingMode::FileSpecific,
    }
}

fn install_kind_code(kind: OpheliaInstallKind) -> u8 {
    match kind {
        OpheliaInstallKind::AppBundle => 1,
        OpheliaInstallKind::HomebrewFormula => 2,
        OpheliaInstallKind::Development => 3,
        OpheliaInstallKind::Other => 4,
        OpheliaInstallKind::Unknown => 0,
    }
}

fn install_kind_from_code(code: u8) -> OpheliaInstallKind {
    match code {
        1 => OpheliaInstallKind::AppBundle,
        2 => OpheliaInstallKind::HomebrewFormula,
        3 => OpheliaInstallKind::Development,
        4 => OpheliaInstallKind::Other,
        _ => OpheliaInstallKind::Unknown,
    }
}

fn endpoint_kind_code(kind: OpheliaEndpointKind) -> u8 {
    match kind {
        OpheliaEndpointKind::MachService => 1,
    }
}

fn endpoint_kind_from_code(code: u8) -> OpheliaEndpointKind {
    match code {
        1 => OpheliaEndpointKind::MachService,
        _ => OpheliaEndpointKind::MachService,
    }
}
