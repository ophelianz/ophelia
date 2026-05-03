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

use super::ffi::{
    ManagedXpcConnection, XpcObjectRaw, activate_connection, frame_from_xpc_event,
    install_event_handler, message_from_body, xpc_connection_send_message_with_reply,
};
use crate::service::codec::{
    OpheliaCommandEnvelope, OpheliaFrameEnvelope, command_to_body, unexpected_xpc_frame,
};
use crate::service::{
    OpheliaCommand, OpheliaError, OpheliaResponse, OpheliaSnapshot, OpheliaUpdateBatch,
    SERVICE_EVENT_CAPACITY,
};
use block2::RcBlock;
use std::ptr;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;

pub(in crate::service) struct MachEventStream {
    rx: mpsc::Receiver<Result<OpheliaFrameEnvelope, OpheliaError>>,
    state: Arc<MachStreamState>,
    _connection: ManagedXpcConnection,
}

impl MachEventStream {
    pub(in crate::service) async fn next_update(
        &mut self,
    ) -> Result<OpheliaUpdateBatch, OpheliaError> {
        let frame = next_stream_frame(&mut self.rx, &self.state).await?;

        match frame {
            OpheliaFrameEnvelope::Update { update } => {
                let update = *update;
                tracing::trace!(
                    lifecycle = update.lifecycle.lifecycle_codes.len(),
                    progress_known = update.progress_known_total.ids.len(),
                    progress_unknown = update.progress_unknown_total.ids.len(),
                    physical_writes = update.physical_write.ids.len(),
                    destinations = update.destination.ids.len(),
                    control_support = update.control_support.ids.len(),
                    direct_details = update.direct_details.unsupported_ids.len()
                        + update.direct_details.loading_ids.len()
                        + update.direct_details.segment_ids.len(),
                    removals = update.removal.ids.len(),
                    unsupported_controls = update.unsupported_control.ids.len(),
                    settings_changed = update.settings_changed.is_some(),
                    "received Mach update batch"
                );
                Ok(update)
            }
            OpheliaFrameEnvelope::Error { error, .. } => Err(error),
            frame => Err(unexpected_xpc_frame("update", frame)),
        }
    }
}

pub(in crate::service) async fn subscribe_mach(
    id: u64,
) -> Result<(OpheliaSnapshot, MachEventStream), OpheliaError> {
    let connection = ManagedXpcConnection::connect_client()?;
    let (event_tx, event_rx) = mpsc::channel(SERVICE_EVENT_CAPACITY);
    let (reply_tx, mut reply_rx) = mpsc::channel(1);
    let stream_state = Arc::new(MachStreamState::default());

    {
        let stream_state = stream_state.clone();
        let event_handler = RcBlock::new(move |event: XpcObjectRaw| {
            let message = frame_from_xpc_event(event);
            record_stream_frame(&event_tx, &stream_state, message);
        });
        let _installed = install_event_handler(connection.handle(), &event_handler);
        activate_connection(connection.handle());
    }

    let command = OpheliaCommandEnvelope {
        id,
        command: OpheliaCommand::Subscribe,
    };
    let message = message_from_body(&command_to_body(&command)?)?;
    {
        let reply_handler = RcBlock::new(move |reply: XpcObjectRaw| {
            let message = frame_from_xpc_event(reply);
            let _ = reply_tx.try_send(message);
        });
        unsafe {
            xpc_connection_send_message_with_reply(
                connection.handle().raw(),
                message.raw(),
                ptr::null_mut(),
                &reply_handler,
            );
        }
    }

    let first = reply_rx.recv().await.ok_or(OpheliaError::Closed)??;
    match first {
        OpheliaFrameEnvelope::Response {
            id: frame_id,
            response,
        } if frame_id == id => match *response {
            OpheliaResponse::Snapshot { snapshot } => Ok((
                *snapshot,
                MachEventStream {
                    rx: event_rx,
                    state: stream_state,
                    _connection: connection,
                },
            )),
            response => Err(unexpected_xpc_frame(
                "snapshot response",
                OpheliaFrameEnvelope::Response {
                    id: frame_id,
                    response: Box::new(response),
                },
            )),
        },
        OpheliaFrameEnvelope::Error {
            id: frame_id,
            error,
        } if frame_id == id => Err(error),
        frame => Err(unexpected_xpc_frame("snapshot response", frame)),
    }
}

async fn next_stream_frame(
    rx: &mut mpsc::Receiver<Result<OpheliaFrameEnvelope, OpheliaError>>,
    state: &MachStreamState,
) -> Result<OpheliaFrameEnvelope, OpheliaError> {
    if let Some(error) = state.take_lagged() {
        return Err(error);
    }

    match rx.try_recv() {
        Ok(frame) => return frame,
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => return Err(OpheliaError::Closed),
    }

    if state.is_closed() {
        return Err(OpheliaError::Closed);
    }

    let frame = rx.recv().await.ok_or(OpheliaError::Closed)??;
    if let Some(error) = state.take_lagged() {
        return Err(error);
    }
    Ok(frame)
}

#[derive(Default)]
struct MachStreamState {
    skipped: AtomicU64,
    closed: AtomicBool,
}

impl MachStreamState {
    fn record_lagged(&self) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
    }

    fn record_closed(&self) {
        self.closed.store(true, Ordering::Release);
    }

    fn take_lagged(&self) -> Option<OpheliaError> {
        let skipped = self.skipped.swap(0, Ordering::AcqRel);
        if skipped > 0 {
            return Some(OpheliaError::Lagged { skipped });
        }
        None
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

fn record_stream_frame(
    tx: &mpsc::Sender<Result<OpheliaFrameEnvelope, OpheliaError>>,
    state: &MachStreamState,
    frame: Result<OpheliaFrameEnvelope, OpheliaError>,
) {
    if matches!(&frame, Err(OpheliaError::Closed)) {
        state.record_closed();
    }
    if let Err(error) = tx.try_send(frame) {
        match error {
            mpsc::error::TrySendError::Full(_) => state.record_lagged(),
            mpsc::error::TrySendError::Closed(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServiceSettings;

    #[test]
    fn stream_frame_backpressure_reports_lag() {
        let state = MachStreamState::default();
        let (tx, _rx) = mpsc::channel(1);
        let event = || {
            Ok(OpheliaFrameEnvelope::Update {
                update: Box::new(OpheliaUpdateBatch::settings_changed(
                    ServiceSettings::default(),
                )),
            })
        };

        record_stream_frame(&tx, &state, event());
        record_stream_frame(&tx, &state, event());

        assert!(matches!(
            state.take_lagged(),
            Some(OpheliaError::Lagged { skipped: 1 })
        ));
    }

    #[test]
    fn stream_frame_closed_is_visible() {
        let state = MachStreamState::default();
        let (tx, _rx) = mpsc::channel(1);

        record_stream_frame(&tx, &state, Err(OpheliaError::Closed));

        assert!(state.is_closed());
    }

    #[test]
    fn queued_terminal_error_wins_over_closed_state() {
        let state = MachStreamState::default();
        let (tx, mut rx) = mpsc::channel(4);

        record_stream_frame(
            &tx,
            &state,
            Ok(OpheliaFrameEnvelope::Error {
                id: 7,
                error: OpheliaError::Lagged { skipped: 3 },
            }),
        );
        record_stream_frame(&tx, &state, Err(OpheliaError::Closed));

        let frame = futures::executor::block_on(next_stream_frame(&mut rx, &state)).unwrap();
        assert!(matches!(
            frame,
            OpheliaFrameEnvelope::Error {
                id: 7,
                error: OpheliaError::Lagged { skipped: 3 }
            }
        ));
    }

    #[test]
    fn local_backpressure_lag_wins_before_queued_update() {
        let state = MachStreamState::default();
        let (tx, mut rx) = mpsc::channel(1);
        let update = || {
            Ok(OpheliaFrameEnvelope::Update {
                update: Box::new(OpheliaUpdateBatch::settings_changed(
                    ServiceSettings::default(),
                )),
            })
        };

        record_stream_frame(&tx, &state, update());
        record_stream_frame(&tx, &state, update());

        let error = futures::executor::block_on(next_stream_frame(&mut rx, &state)).unwrap_err();
        assert!(matches!(error, OpheliaError::Lagged { skipped: 1 }));
    }
}
