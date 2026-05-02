use std::collections::HashMap;
use std::path::Path;

use super::{
    OpheliaEvent, OpheliaSnapshot, ProgressUpdate, ServiceSettings, TransferId,
    TransferRuntimeEvent, TransferStatus, TransferSummary,
};

#[derive(Default)]
pub(super) struct OpheliaReadModel {
    transfers: HashMap<TransferId, TransferSummary>,
    order: Vec<TransferId>,
}

#[derive(Default)]
pub(super) struct OpheliaEventCoalescer {
    transfer_order: Vec<TransferId>,
    transfer_updates: HashMap<TransferId, TransferSummary>,
    byte_order: Vec<TransferId>,
    bytes_written: HashMap<TransferId, u64>,
    stats: OpheliaCoalescerStats,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct OpheliaCoalescerStats {
    pub raw_transfer_updates: u64,
    pub raw_write_updates: u64,
    pub emitted_transfer_updates: u64,
    pub emitted_write_updates: u64,
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
}

impl OpheliaEventCoalescer {
    pub(super) fn record_transfer(&mut self, snapshot: TransferSummary) {
        self.stats.raw_transfer_updates = self.stats.raw_transfer_updates.saturating_add(1);
        let id = snapshot.id;
        if !self.transfer_updates.contains_key(&id) {
            self.transfer_order.push(id);
        }
        self.transfer_updates.insert(id, snapshot);
    }

    pub(super) fn record_bytes_written(&mut self, id: TransferId, bytes: u64) {
        self.stats.raw_write_updates = self.stats.raw_write_updates.saturating_add(1);
        if !self.bytes_written.contains_key(&id) {
            self.byte_order.push(id);
        }
        let total = self.bytes_written.entry(id).or_insert(0);
        *total = total.saturating_add(bytes);
    }

    pub(super) fn remove_transfer(&mut self, id: TransferId) {
        self.transfer_updates.remove(&id);
    }

    pub(super) fn drain_events(&mut self) -> Vec<OpheliaEvent> {
        let mut events = Vec::with_capacity(self.transfer_updates.len() + self.bytes_written.len());
        for id in self.transfer_order.drain(..) {
            if let Some(snapshot) = self.transfer_updates.remove(&id) {
                self.stats.emitted_transfer_updates =
                    self.stats.emitted_transfer_updates.saturating_add(1);
                events.push(OpheliaEvent::TransferChanged { snapshot });
            }
        }
        for id in self.byte_order.drain(..) {
            if let Some(bytes) = self.bytes_written.remove(&id) {
                self.stats.emitted_write_updates =
                    self.stats.emitted_write_updates.saturating_add(1);
                events.push(OpheliaEvent::TransferBytesWritten { id, bytes });
            }
        }
        events
    }

    pub(super) fn stats(&self) -> OpheliaCoalescerStats {
        self.stats
    }
}

impl OpheliaReadModel {
    pub(super) fn snapshot(&self, settings: &ServiceSettings) -> OpheliaSnapshot {
        OpheliaSnapshot {
            transfers: self
                .order
                .iter()
                .filter_map(|id| self.transfers.get(id).cloned())
                .collect(),
            settings: settings.clone(),
        }
    }

    pub(super) fn apply_transfer_event(
        &mut self,
        event: TransferRuntimeEvent,
        coalescer: &mut OpheliaEventCoalescer,
    ) -> Option<OpheliaEvent> {
        match event {
            TransferRuntimeEvent::TransferAdded { snapshot }
            | TransferRuntimeEvent::TransferRestored { snapshot } => {
                let snapshot = self.upsert(snapshot);
                coalescer.remove_transfer(snapshot.id);
                Some(OpheliaEvent::TransferChanged { snapshot })
            }
            TransferRuntimeEvent::Progress(update) => self.apply_progress(update, coalescer),
            TransferRuntimeEvent::TransferBytesWritten { id, bytes } => {
                coalescer.record_bytes_written(id, bytes);
                None
            }
            TransferRuntimeEvent::DestinationChanged { id, destination } => self
                .mutate(id, |snapshot| {
                    snapshot.destination = destination;
                })
                .map(|snapshot| {
                    coalescer.remove_transfer(id);
                    OpheliaEvent::TransferChanged { snapshot }
                }),
            TransferRuntimeEvent::ControlSupportChanged { id, support } => self
                .mutate(id, |snapshot| {
                    snapshot.control_support = support;
                })
                .map(|snapshot| {
                    coalescer.remove_transfer(id);
                    OpheliaEvent::TransferChanged { snapshot }
                }),
            TransferRuntimeEvent::ChunkMapChanged { id, state } => {
                if let Some(snapshot) = self.mutate(id, |snapshot| {
                    snapshot.chunk_map_state = state;
                }) {
                    coalescer.record_transfer(snapshot);
                }
                None
            }
            TransferRuntimeEvent::TransferRemoved {
                id,
                action,
                artifact_state,
            } => {
                self.transfers.remove(&id);
                self.order.retain(|current| *current != id);
                coalescer.remove_transfer(id);
                Some(OpheliaEvent::TransferRemoved {
                    id,
                    action,
                    artifact_state,
                })
            }
            TransferRuntimeEvent::ControlUnsupported { id, action } => {
                Some(OpheliaEvent::ControlUnsupported { id, action })
            }
        }
    }

    pub(super) fn apply_progress(
        &mut self,
        update: ProgressUpdate,
        coalescer: &mut OpheliaEventCoalescer,
    ) -> Option<OpheliaEvent> {
        let status = update.status;
        let snapshot = self.mutate(update.id, |snapshot| {
            snapshot.status = update.status;
            snapshot.downloaded_bytes = update.downloaded_bytes;
            snapshot.total_bytes = update.total_bytes;
            snapshot.speed_bytes_per_sec = update.speed_bytes_per_sec;
        })?;

        if status == TransferStatus::Downloading {
            coalescer.record_transfer(snapshot);
            None
        } else {
            coalescer.remove_transfer(update.id);
            Some(OpheliaEvent::TransferChanged { snapshot })
        }
    }

    pub(super) fn upsert(&mut self, snapshot: TransferSummary) -> TransferSummary {
        if !self.transfers.contains_key(&snapshot.id) {
            self.order.push(snapshot.id);
        }
        self.transfers.insert(snapshot.id, snapshot.clone());
        snapshot
    }

    pub(super) fn mutate(
        &mut self,
        id: TransferId,
        mutate: impl FnOnce(&mut TransferSummary),
    ) -> Option<TransferSummary> {
        let snapshot = self.transfers.get_mut(&id)?;
        mutate(snapshot);
        Some(snapshot.clone())
    }

    pub(super) fn destination(&self, id: TransferId) -> Option<&Path> {
        self.transfers
            .get(&id)
            .map(|snapshot| snapshot.destination.as_path())
    }

    pub(super) fn remove(&mut self, id: TransferId) {
        self.transfers.remove(&id);
        self.order.retain(|current| *current != id);
    }

    pub(super) fn has_running_transfers(&self) -> bool {
        self.transfers.values().any(|snapshot| {
            matches!(
                snapshot.status,
                TransferStatus::Pending | TransferStatus::Downloading
            )
        })
    }
}
