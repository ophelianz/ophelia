use super::*;

#[derive(Default)]
pub(super) struct SessionReadModel {
    transfers: HashMap<DownloadId, TransferSnapshot>,
    order: Vec<DownloadId>,
}

#[derive(Default)]
pub(super) struct SessionEventCoalescer {
    transfer_order: Vec<DownloadId>,
    transfer_updates: HashMap<DownloadId, TransferSnapshot>,
    byte_order: Vec<DownloadId>,
    bytes_written: HashMap<DownloadId, u64>,
}

impl SessionEventCoalescer {
    pub(super) fn record_transfer(&mut self, snapshot: TransferSnapshot) {
        let id = snapshot.id;
        if !self.transfer_updates.contains_key(&id) {
            self.transfer_order.push(id);
        }
        self.transfer_updates.insert(id, snapshot);
    }

    pub(super) fn record_bytes_written(&mut self, id: DownloadId, bytes: u64) {
        if !self.bytes_written.contains_key(&id) {
            self.byte_order.push(id);
        }
        let total = self.bytes_written.entry(id).or_insert(0);
        *total = total.saturating_add(bytes);
    }

    pub(super) fn remove_transfer(&mut self, id: DownloadId) {
        self.transfer_updates.remove(&id);
    }

    pub(super) fn drain_events(&mut self) -> Vec<SessionEvent> {
        let mut events = Vec::with_capacity(self.transfer_updates.len() + self.bytes_written.len());
        for id in self.transfer_order.drain(..) {
            if let Some(snapshot) = self.transfer_updates.remove(&id) {
                events.push(SessionEvent::TransferChanged { snapshot });
            }
        }
        for id in self.byte_order.drain(..) {
            if let Some(bytes) = self.bytes_written.remove(&id) {
                events.push(SessionEvent::DownloadBytesWritten { id, bytes });
            }
        }
        events
    }
}

impl SessionReadModel {
    pub(super) fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            transfers: self
                .order
                .iter()
                .filter_map(|id| self.transfers.get(id).cloned())
                .collect(),
        }
    }

    pub(super) fn apply_engine_event(
        &mut self,
        event: EngineEvent,
        coalescer: &mut SessionEventCoalescer,
    ) -> Option<SessionEvent> {
        match event {
            EngineEvent::TransferAdded { snapshot }
            | EngineEvent::TransferRestored { snapshot } => {
                let snapshot = self.upsert(snapshot);
                coalescer.remove_transfer(snapshot.id);
                Some(SessionEvent::TransferChanged { snapshot })
            }
            EngineEvent::Progress(update) => self.apply_progress(update, coalescer),
            EngineEvent::DownloadBytesWritten { id, bytes } => {
                coalescer.record_bytes_written(id, bytes);
                None
            }
            EngineEvent::DestinationChanged { id, destination } => self
                .mutate(id, |snapshot| {
                    snapshot.destination = destination;
                })
                .map(|snapshot| {
                    coalescer.remove_transfer(id);
                    SessionEvent::TransferChanged { snapshot }
                }),
            EngineEvent::ControlSupportChanged { id, support } => self
                .mutate(id, |snapshot| {
                    snapshot.control_support = support;
                })
                .map(|snapshot| {
                    coalescer.remove_transfer(id);
                    SessionEvent::TransferChanged { snapshot }
                }),
            EngineEvent::ChunkMapChanged { id, state } => {
                if let Some(snapshot) = self.mutate(id, |snapshot| {
                    snapshot.chunk_map_state = state;
                }) {
                    coalescer.record_transfer(snapshot);
                }
                None
            }
            EngineEvent::LiveTransferRemoved {
                id,
                action,
                artifact_state,
            } => {
                self.transfers.remove(&id);
                self.order.retain(|current| *current != id);
                coalescer.remove_transfer(id);
                Some(SessionEvent::TransferRemoved {
                    id,
                    action,
                    artifact_state,
                })
            }
            EngineEvent::ControlUnsupported { id, action } => {
                Some(SessionEvent::ControlUnsupported { id, action })
            }
        }
    }

    pub(super) fn apply_progress(
        &mut self,
        update: ProgressUpdate,
        coalescer: &mut SessionEventCoalescer,
    ) -> Option<SessionEvent> {
        let status = update.status;
        let snapshot = self.mutate(update.id, |snapshot| {
            snapshot.status = update.status;
            snapshot.downloaded_bytes = update.downloaded_bytes;
            snapshot.total_bytes = update.total_bytes;
            snapshot.speed_bytes_per_sec = update.speed_bytes_per_sec;
        })?;

        if status == DownloadStatus::Downloading {
            coalescer.record_transfer(snapshot);
            None
        } else {
            coalescer.remove_transfer(update.id);
            Some(SessionEvent::TransferChanged { snapshot })
        }
    }

    pub(super) fn upsert(&mut self, snapshot: TransferSnapshot) -> TransferSnapshot {
        if !self.transfers.contains_key(&snapshot.id) {
            self.order.push(snapshot.id);
        }
        self.transfers.insert(snapshot.id, snapshot.clone());
        snapshot
    }

    pub(super) fn mutate(
        &mut self,
        id: DownloadId,
        mutate: impl FnOnce(&mut TransferSnapshot),
    ) -> Option<TransferSnapshot> {
        let snapshot = self.transfers.get_mut(&id)?;
        mutate(snapshot);
        Some(snapshot.clone())
    }

    pub(super) fn destination(&self, id: DownloadId) -> Option<&Path> {
        self.transfers
            .get(&id)
            .map(|snapshot| snapshot.destination.as_path())
    }

    pub(super) fn remove(&mut self, id: DownloadId) {
        self.transfers.remove(&id);
        self.order.retain(|current| *current != id);
    }
}
