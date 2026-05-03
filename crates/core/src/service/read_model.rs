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

use std::collections::HashMap;
use std::path::Path;

use super::{
    DirectDetailsTable, OpheliaSnapshot, OpheliaUpdateBatch, ProgressUpdate, ServiceSettings,
    TransferId, TransferLifecycleCode, TransferRuntimeEvent, TransferStatus, TransferSummary,
    TransferSummaryTable, artifact_state_code, control_action_code, control_support_flags,
    removal_action_code,
};
use crate::engine::{DirectChunkMapState, TransferDetails};

#[derive(Default)]
pub(super) struct OpheliaReadModel {
    transfers: TransferSummaryTable,
    row_by_id: HashMap<TransferId, usize>,
    direct_details: DirectDetailsTable,
}

#[derive(Default)]
pub(super) struct OpheliaUpdateBuilder {
    batch: OpheliaUpdateBatch,
    progress_known: HashMap<TransferId, usize>,
    progress_unknown: HashMap<TransferId, usize>,
    writes: HashMap<TransferId, usize>,
    destinations: HashMap<TransferId, usize>,
    control_support: HashMap<TransferId, usize>,
    stats: OpheliaCoalescerStats,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct OpheliaCoalescerStats {
    pub raw_transfer_updates: u64,
    pub raw_write_updates: u64,
    pub raw_detail_updates: u64,
    pub emitted_transfer_updates: u64,
    pub emitted_write_updates: u64,
    pub emitted_detail_updates: u64,
}

impl OpheliaCoalescerStats {
    pub(super) fn coalesced_transfer_updates(self) -> u64 {
        self.raw_transfer_updates
            .saturating_sub(self.emitted_transfer_updates)
    }

    pub(super) fn coalesced_write_updates(self) -> u64 {
        self.raw_write_updates
            .saturating_sub(self.emitted_write_updates)
    }

    pub(super) fn coalesced_detail_updates(self) -> u64 {
        self.raw_detail_updates
            .saturating_sub(self.emitted_detail_updates)
    }
}

impl OpheliaUpdateBuilder {
    pub(super) fn record_lifecycle(
        &mut self,
        code: TransferLifecycleCode,
        snapshot: TransferSummary,
    ) {
        self.stats.raw_transfer_updates = self.stats.raw_transfer_updates.saturating_add(1);
        self.batch.lifecycle.lifecycle_codes.push(code as u8);
        self.batch.lifecycle.transfers.push_summary(snapshot);
    }

    pub(super) fn record_progress(&mut self, update: &ProgressUpdate) {
        self.stats.raw_transfer_updates = self.stats.raw_transfer_updates.saturating_add(1);
        match update.total_bytes {
            Some(total) => self.record_known_progress(
                update.id,
                update.downloaded_bytes,
                total,
                update.speed_bytes_per_sec,
            ),
            None => self.record_unknown_progress(
                update.id,
                update.downloaded_bytes,
                update.speed_bytes_per_sec,
            ),
        }
    }

    pub(super) fn record_bytes_written(&mut self, id: TransferId, bytes: u64) {
        self.stats.raw_write_updates = self.stats.raw_write_updates.saturating_add(1);
        if let Some(index) = self.writes.get(&id).copied() {
            self.batch.physical_write.bytes[index] =
                self.batch.physical_write.bytes[index].saturating_add(bytes);
            return;
        }
        self.writes.insert(id, self.batch.physical_write.ids.len());
        self.batch.physical_write.ids.push(id);
        self.batch.physical_write.bytes.push(bytes);
    }

    pub(super) fn record_destination(&mut self, id: TransferId, destination: std::path::PathBuf) {
        self.stats.raw_transfer_updates = self.stats.raw_transfer_updates.saturating_add(1);
        if let Some(index) = self.destinations.get(&id).copied() {
            self.batch.destination.destinations[index] = destination;
            return;
        }
        self.destinations
            .insert(id, self.batch.destination.ids.len());
        self.batch.destination.ids.push(id);
        self.batch.destination.destinations.push(destination);
    }

    pub(super) fn record_control_support(
        &mut self,
        id: TransferId,
        support: crate::engine::TransferControlSupport,
    ) {
        self.stats.raw_transfer_updates = self.stats.raw_transfer_updates.saturating_add(1);
        let flags = control_support_flags(support);
        if let Some(index) = self.control_support.get(&id).copied() {
            self.batch.control_support.control_flags[index] = flags;
            return;
        }
        self.control_support
            .insert(id, self.batch.control_support.ids.len());
        self.batch.control_support.ids.push(id);
        self.batch.control_support.control_flags.push(flags);
    }

    pub(super) fn record_direct_details(&mut self, id: TransferId, details: TransferDetails) {
        self.stats.raw_detail_updates = self.stats.raw_detail_updates.saturating_add(1);
        self.batch.direct_details.push_details(id, details);
    }

    pub(super) fn record_removal(
        &mut self,
        id: TransferId,
        action: crate::engine::LiveTransferRemovalAction,
        artifact_state: crate::engine::ArtifactState,
    ) {
        self.remove_transfer(id);
        self.batch.removal.ids.push(id);
        self.batch
            .removal
            .action_codes
            .push(removal_action_code(action));
        self.batch
            .removal
            .artifact_state_codes
            .push(artifact_state_code(artifact_state));
    }

    pub(super) fn record_unsupported_control(
        &mut self,
        id: TransferId,
        action: crate::engine::TransferControlAction,
    ) {
        self.batch.unsupported_control.ids.push(id);
        self.batch
            .unsupported_control
            .action_codes
            .push(control_action_code(action));
    }

    pub(super) fn record_settings_changed(&mut self, settings: ServiceSettings) {
        self.batch.settings_changed = Some(settings);
    }

    pub(super) fn remove_transfer(&mut self, id: TransferId) {
        self.remove_known_progress(id);
        self.remove_unknown_progress(id);
        self.remove_destination(id);
        self.remove_control_support(id);
        self.batch.direct_details.remove(id);
    }

    pub(super) fn drain_batch(&mut self) -> Option<OpheliaUpdateBatch> {
        if self.batch.is_empty() {
            return None;
        }
        self.stats.emitted_transfer_updates = self.stats.emitted_transfer_updates.saturating_add(
            self.batch.lifecycle.lifecycle_codes.len() as u64
                + self.batch.progress_known_total.ids.len() as u64
                + self.batch.progress_unknown_total.ids.len() as u64
                + self.batch.destination.ids.len() as u64
                + self.batch.control_support.ids.len() as u64,
        );
        self.stats.emitted_write_updates = self
            .stats
            .emitted_write_updates
            .saturating_add(self.batch.physical_write.ids.len() as u64);
        self.stats.emitted_detail_updates = self.stats.emitted_detail_updates.saturating_add(
            self.batch.direct_details.unsupported_ids.len() as u64
                + self.batch.direct_details.loading_ids.len() as u64
                + self.batch.direct_details.segment_ids.len() as u64,
        );
        self.progress_known.clear();
        self.progress_unknown.clear();
        self.writes.clear();
        self.destinations.clear();
        self.control_support.clear();
        Some(std::mem::take(&mut self.batch))
    }

    pub(super) fn stats(&self) -> OpheliaCoalescerStats {
        self.stats
    }

    fn record_known_progress(&mut self, id: TransferId, downloaded: u64, total: u64, speed: u64) {
        self.remove_unknown_progress(id);
        if let Some(index) = self.progress_known.get(&id).copied() {
            self.batch.progress_known_total.downloaded_bytes[index] = downloaded;
            self.batch.progress_known_total.total_bytes[index] = total;
            self.batch.progress_known_total.speed_bytes_per_sec[index] = speed;
            return;
        }
        self.progress_known
            .insert(id, self.batch.progress_known_total.ids.len());
        self.batch.progress_known_total.ids.push(id);
        self.batch
            .progress_known_total
            .downloaded_bytes
            .push(downloaded);
        self.batch.progress_known_total.total_bytes.push(total);
        self.batch
            .progress_known_total
            .speed_bytes_per_sec
            .push(speed);
    }

    fn record_unknown_progress(&mut self, id: TransferId, downloaded: u64, speed: u64) {
        self.remove_known_progress(id);
        if let Some(index) = self.progress_unknown.get(&id).copied() {
            self.batch.progress_unknown_total.downloaded_bytes[index] = downloaded;
            self.batch.progress_unknown_total.speed_bytes_per_sec[index] = speed;
            return;
        }
        self.progress_unknown
            .insert(id, self.batch.progress_unknown_total.ids.len());
        self.batch.progress_unknown_total.ids.push(id);
        self.batch
            .progress_unknown_total
            .downloaded_bytes
            .push(downloaded);
        self.batch
            .progress_unknown_total
            .speed_bytes_per_sec
            .push(speed);
    }

    fn remove_known_progress(&mut self, id: TransferId) {
        if let Some(index) = self.progress_known.remove(&id) {
            remove_progress_known_row(&mut self.batch.progress_known_total, index);
            rebuild_index(
                &self.batch.progress_known_total.ids,
                &mut self.progress_known,
            );
        }
    }

    fn remove_unknown_progress(&mut self, id: TransferId) {
        if let Some(index) = self.progress_unknown.remove(&id) {
            remove_progress_unknown_row(&mut self.batch.progress_unknown_total, index);
            rebuild_index(
                &self.batch.progress_unknown_total.ids,
                &mut self.progress_unknown,
            );
        }
    }

    fn remove_destination(&mut self, id: TransferId) {
        if let Some(index) = self.destinations.remove(&id) {
            self.batch.destination.ids.remove(index);
            self.batch.destination.destinations.remove(index);
            rebuild_index(&self.batch.destination.ids, &mut self.destinations);
        }
    }

    fn remove_control_support(&mut self, id: TransferId) {
        if let Some(index) = self.control_support.remove(&id) {
            self.batch.control_support.ids.remove(index);
            self.batch.control_support.control_flags.remove(index);
            rebuild_index(&self.batch.control_support.ids, &mut self.control_support);
        }
    }
}

impl OpheliaReadModel {
    pub(super) fn snapshot(&self, settings: &ServiceSettings) -> OpheliaSnapshot {
        OpheliaSnapshot {
            transfers: self.transfers.clone(),
            direct_details: self.direct_details.clone(),
            settings: settings.clone(),
        }
    }

    pub(super) fn apply_transfer_event(
        &mut self,
        event: TransferRuntimeEvent,
        builder: &mut OpheliaUpdateBuilder,
    ) {
        match event {
            TransferRuntimeEvent::TransferAdded { snapshot } => {
                let snapshot = self.upsert(snapshot);
                builder.remove_transfer(snapshot.id);
                builder.record_lifecycle(TransferLifecycleCode::Added, snapshot);
            }
            TransferRuntimeEvent::TransferRestored { snapshot } => {
                let snapshot = self.upsert(snapshot);
                builder.remove_transfer(snapshot.id);
                builder.record_lifecycle(TransferLifecycleCode::Restored, snapshot);
            }
            TransferRuntimeEvent::Progress(update) => self.apply_progress(update, builder),
            TransferRuntimeEvent::TransferBytesWritten { id, bytes } => {
                builder.record_bytes_written(id, bytes);
            }
            TransferRuntimeEvent::DestinationChanged { id, destination } => {
                if self
                    .mutate(id, |snapshot| snapshot.destination = destination.clone())
                    .is_some()
                {
                    builder.record_destination(id, destination);
                }
            }
            TransferRuntimeEvent::ControlSupportChanged { id, support } => {
                if self
                    .mutate(id, |snapshot| snapshot.control_support = support)
                    .is_some()
                {
                    builder.record_control_support(id, support);
                }
            }
            TransferRuntimeEvent::DetailsChanged { id, details } => {
                if self.row_by_id.contains_key(&id) {
                    self.direct_details.push_details(id, details.clone());
                    builder.record_direct_details(id, details);
                }
            }
            TransferRuntimeEvent::TransferRemoved {
                id,
                action,
                artifact_state,
            } => {
                self.remove(id);
                builder.record_removal(id, action, artifact_state);
            }
            TransferRuntimeEvent::ControlUnsupported { id, action } => {
                builder.record_unsupported_control(id, action);
            }
        }
    }

    pub(super) fn apply_progress(
        &mut self,
        update: ProgressUpdate,
        builder: &mut OpheliaUpdateBuilder,
    ) {
        let status = update.status;
        let snapshot = self.mutate(update.id, |snapshot| {
            snapshot.status = update.status;
            snapshot.downloaded_bytes = update.downloaded_bytes;
            snapshot.total_bytes = update.total_bytes;
            snapshot.speed_bytes_per_sec = update.speed_bytes_per_sec;
        });

        match (status, snapshot) {
            (TransferStatus::Downloading, Some(_)) => builder.record_progress(&update),
            (_, Some(snapshot)) => {
                builder.remove_transfer(update.id);
                builder.record_lifecycle(TransferLifecycleCode::Terminal, snapshot);
            }
            (_, None) => {}
        }
    }

    pub(super) fn upsert(&mut self, snapshot: TransferSummary) -> TransferSummary {
        if let Some(row) = self.row_by_id.get(&snapshot.id).copied() {
            self.transfers.replace_summary(row, snapshot.clone());
        } else {
            let row = self.transfers.len();
            self.row_by_id.insert(snapshot.id, row);
            self.transfers.push_summary(snapshot.clone());
            self.direct_details
                .push_state(snapshot.id, DirectChunkMapState::Unsupported);
        }
        snapshot
    }

    pub(super) fn mutate(
        &mut self,
        id: TransferId,
        mutate: impl FnOnce(&mut TransferSummary),
    ) -> Option<TransferSummary> {
        let row = self.row_by_id.get(&id).copied()?;
        let mut snapshot = self.transfers.summary(row)?;
        mutate(&mut snapshot);
        self.transfers.replace_summary(row, snapshot.clone());
        Some(snapshot)
    }

    pub(super) fn destination(&self, id: TransferId) -> Option<&Path> {
        let row = self.row_by_id.get(&id).copied()?;
        self.transfers
            .destinations
            .get(row)
            .map(std::path::PathBuf::as_path)
    }

    pub(super) fn remove(&mut self, id: TransferId) {
        let Some(row) = self.row_by_id.remove(&id) else {
            self.direct_details.remove(id);
            return;
        };
        self.transfers.remove_row(row);
        self.direct_details.remove(id);
        for value in self.row_by_id.values_mut() {
            if *value > row {
                *value -= 1;
            }
        }
    }

    pub(super) fn has_running_transfers(&self) -> bool {
        self.transfers.status_codes.iter().any(|code| {
            matches!(
                super::transfer_status_from_code(*code),
                TransferStatus::Pending | TransferStatus::Downloading
            )
        })
    }
}

fn remove_progress_known_row(batch: &mut super::ProgressKnownTotalBatch, index: usize) {
    batch.ids.remove(index);
    batch.downloaded_bytes.remove(index);
    batch.total_bytes.remove(index);
    batch.speed_bytes_per_sec.remove(index);
}

fn remove_progress_unknown_row(batch: &mut super::ProgressUnknownTotalBatch, index: usize) {
    batch.ids.remove(index);
    batch.downloaded_bytes.remove(index);
    batch.speed_bytes_per_sec.remove(index);
}

fn rebuild_index(ids: &[TransferId], index: &mut HashMap<TransferId, usize>) {
    index.clear();
    for (row, id) in ids.iter().copied().enumerate() {
        index.insert(id, row);
    }
}
