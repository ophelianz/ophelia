use super::ffi::{
    XpcConnection, XpcObjectRaw, frame_from_xpc_event, message_from_body, xpc_connection_activate,
    xpc_connection_send_message_with_reply, xpc_connection_set_event_handler,
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

pub(in crate::service) struct MachEventStream {
    rx: mpsc::Receiver<Result<OpheliaFrameEnvelope, OpheliaError>>,
    state: Arc<MachStreamState>,
    _connection: XpcConnection,
}

impl MachEventStream {
    pub(in crate::service) async fn next_update(
        &mut self,
    ) -> Result<OpheliaUpdateBatch, OpheliaError> {
        if let Some(error) = self.state.take_pending_error() {
            return Err(error);
        }

        let frame = self.rx.recv().await.ok_or(OpheliaError::Closed)??;
        if let Some(error) = self.state.take_pending_error() {
            return Err(error);
        }

        match frame {
            OpheliaFrameEnvelope::Update { update } => Ok(*update),
            OpheliaFrameEnvelope::Error { error, .. } => Err(error),
            frame => Err(unexpected_xpc_frame("update", frame)),
        }
    }
}

pub(in crate::service) async fn subscribe_mach(
    id: u64,
) -> Result<(OpheliaSnapshot, MachEventStream), OpheliaError> {
    let connection = XpcConnection::connect_client()?;
    let (event_tx, event_rx) = mpsc::channel(SERVICE_EVENT_CAPACITY);
    let (reply_tx, mut reply_rx) = mpsc::channel(1);
    let stream_state = Arc::new(MachStreamState::default());

    {
        let stream_state = stream_state.clone();
        let event_handler = RcBlock::new(move |event: XpcObjectRaw| {
            let message = frame_from_xpc_event(event);
            record_stream_frame(&event_tx, &stream_state, message);
        });
        unsafe {
            xpc_connection_set_event_handler(connection.raw(), &event_handler);
            xpc_connection_activate(connection.raw());
        }
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
                connection.raw(),
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

    fn take_pending_error(&self) -> Option<OpheliaError> {
        let skipped = self.skipped.swap(0, Ordering::AcqRel);
        if skipped > 0 {
            return Some(OpheliaError::Lagged { skipped });
        }
        if self.closed.load(Ordering::Acquire) {
            return Some(OpheliaError::Closed);
        }
        None
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
            state.take_pending_error(),
            Some(OpheliaError::Lagged { skipped: 1 })
        ));
    }

    #[test]
    fn stream_frame_closed_is_visible_even_if_frame_is_queued() {
        let state = MachStreamState::default();
        let (tx, _rx) = mpsc::channel(1);

        record_stream_frame(&tx, &state, Err(OpheliaError::Closed));

        assert!(matches!(
            state.take_pending_error(),
            Some(OpheliaError::Closed)
        ));
    }
}
