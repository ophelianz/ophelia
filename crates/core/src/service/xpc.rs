use super::wire::{
    OpheliaWireCommand, OpheliaWireFrame, command_from_payload, command_to_payload,
    frame_from_payload, frame_to_payload, unexpected_wire_frame,
};
use super::*;
use block2::{Block, RcBlock};
use std::ffi::{CString, c_char, c_void};
use std::ptr;
use std::slice;
use tokio::sync::mpsc;

// Keep this FFI layer narrow: XPC dictionaries only carry JSON bytes under `payload`
type XpcObjectRaw = *mut c_void;
type XpcConnectionRaw = *mut c_void;
type DispatchQueueRaw = *mut c_void;

const XPC_CONNECTION_MACH_SERVICE_LISTENER: u64 = 1 << 0;

unsafe extern "C" {
    fn xpc_connection_create_mach_service(
        name: *const c_char,
        targetq: DispatchQueueRaw,
        flags: u64,
    ) -> XpcConnectionRaw;
    fn xpc_connection_set_event_handler(
        connection: XpcConnectionRaw,
        handler: &Block<dyn Fn(XpcObjectRaw)>,
    );
    fn xpc_connection_activate(connection: XpcConnectionRaw);
    fn xpc_connection_cancel(connection: XpcConnectionRaw);
    fn xpc_connection_send_message(connection: XpcConnectionRaw, message: XpcObjectRaw);
    fn xpc_connection_send_message_with_reply(
        connection: XpcConnectionRaw,
        message: XpcObjectRaw,
        replyq: DispatchQueueRaw,
        handler: &Block<dyn Fn(XpcObjectRaw)>,
    );
    fn xpc_connection_get_euid(connection: XpcConnectionRaw) -> libc::uid_t;

    fn xpc_dictionary_create(
        keys: *const *const c_char,
        values: *const XpcObjectRaw,
        count: usize,
    ) -> XpcObjectRaw;
    fn xpc_dictionary_create_reply(original: XpcObjectRaw) -> XpcObjectRaw;
    fn xpc_dictionary_set_data(
        dictionary: XpcObjectRaw,
        key: *const c_char,
        bytes: *const c_void,
        length: usize,
    );
    fn xpc_dictionary_get_data(
        dictionary: XpcObjectRaw,
        key: *const c_char,
        length: *mut usize,
    ) -> *const c_void;

    fn xpc_get_type(object: XpcObjectRaw) -> *const c_void;
    fn xpc_retain(object: XpcObjectRaw) -> XpcObjectRaw;
    fn xpc_release(object: XpcObjectRaw);

    static _xpc_type_error: u8;
}

pub(super) struct MachEventStream {
    rx: mpsc::Receiver<Result<OpheliaWireFrame, OpheliaError>>,
    _connection: XpcConnection,
}

impl MachEventStream {
    pub(super) async fn next_event(&mut self) -> Result<OpheliaEvent, OpheliaError> {
        let frame = self.rx.recv().await.ok_or(OpheliaError::Closed)??;
        match frame {
            OpheliaWireFrame::Event { event } => Ok(event),
            OpheliaWireFrame::Error { error, .. } => Err(error),
            frame => Err(unexpected_wire_frame("event", frame)),
        }
    }
}

pub(super) async fn dispatch_mach(
    id: u64,
    command: OpheliaCommand,
) -> Result<OpheliaResponse, OpheliaError> {
    tokio::task::spawn_blocking(move || dispatch_mach_blocking(id, command))
        .await
        .map_err(|error| OpheliaError::Transport {
            message: error.to_string(),
        })?
}

pub(super) async fn subscribe_mach(
    id: u64,
) -> Result<(OpheliaSnapshot, MachEventStream), OpheliaError> {
    let connection = XpcConnection::connect_client()?;
    let (tx, mut rx) = mpsc::channel(SERVICE_EVENT_CAPACITY);

    {
        // libxpc retains handler blocks; the ignored launchd smoke test should prove this path
        let event_tx = tx.clone();
        let event_handler = RcBlock::new(move |event: XpcObjectRaw| {
            let message = frame_from_xpc_event(event);
            let _ = event_tx.try_send(message);
        });
        unsafe {
            xpc_connection_set_event_handler(connection.raw(), &event_handler);
            xpc_connection_activate(connection.raw());
        }
    }

    let command = OpheliaWireCommand {
        id,
        command: OpheliaCommand::Subscribe,
    };
    let message = message_from_payload(&command_to_payload(&command)?)?;
    {
        // libxpc retains reply blocks; if that ever proves false, store them on the stream
        let reply_tx = tx.clone();
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

    let first = rx.recv().await.ok_or(OpheliaError::Closed)??;
    match first {
        OpheliaWireFrame::Response {
            id: frame_id,
            response: OpheliaResponse::Snapshot { snapshot },
        } if frame_id == id => Ok((
            snapshot,
            MachEventStream {
                rx,
                _connection: connection,
            },
        )),
        OpheliaWireFrame::Error {
            id: frame_id,
            error,
        } if frame_id == id => Err(error),
        frame => Err(unexpected_wire_frame("snapshot response", frame)),
    }
}

pub fn run_mach_service(runtime: &Handle, client: OpheliaClient) -> Result<(), OpheliaError> {
    let service_name =
        CString::new(OPHELIA_MACH_SERVICE_NAME).map_err(|error| OpheliaError::Transport {
            message: error.to_string(),
        })?;
    let listener = unsafe {
        xpc_connection_create_mach_service(
            service_name.as_ptr(),
            ptr::null_mut(),
            XPC_CONNECTION_MACH_SERVICE_LISTENER,
        )
    };
    let listener = XpcConnection::from_owned(listener)?;
    let runtime = runtime.clone();
    let handler = RcBlock::new(move |peer: XpcObjectRaw| {
        if peer.is_null() {
            return;
        }
        if !peer_is_same_user(peer) {
            unsafe { xpc_connection_cancel(peer) };
            return;
        }
        if let Ok(peer) = XpcConnection::retain(peer) {
            accept_peer_connection(peer, runtime.clone(), client.clone());
        }
    });
    unsafe {
        xpc_connection_set_event_handler(listener.raw(), &handler);
        xpc_connection_activate(listener.raw());
    }

    loop {
        std::thread::park();
    }
}

fn dispatch_mach_blocking(
    id: u64,
    command: OpheliaCommand,
) -> Result<OpheliaResponse, OpheliaError> {
    let connection = XpcConnection::connect_client()?;
    let noop = RcBlock::new(|_event: XpcObjectRaw| {});
    unsafe {
        xpc_connection_set_event_handler(connection.raw(), &noop);
        xpc_connection_activate(connection.raw());
    }

    let command = OpheliaWireCommand { id, command };
    let message = message_from_payload(&command_to_payload(&command)?)?;
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    let reply_handler = RcBlock::new(move |reply: XpcObjectRaw| {
        let _ = reply_tx.send(frame_from_xpc_event(reply));
    });
    unsafe {
        xpc_connection_send_message_with_reply(
            connection.raw(),
            message.raw(),
            ptr::null_mut(),
            &reply_handler,
        );
    }
    let frame = reply_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| OpheliaError::Closed)??;

    match frame {
        OpheliaWireFrame::Response {
            id: frame_id,
            response,
        } if frame_id == id => Ok(response),
        OpheliaWireFrame::Error {
            id: frame_id,
            error,
        } if frame_id == id => Err(error),
        frame => Err(unexpected_wire_frame("response", frame)),
    }
}

fn accept_peer_connection(peer: XpcConnection, runtime: Handle, client: OpheliaClient) {
    let peer_for_handler = peer.clone();
    let handler = RcBlock::new(move |event: XpcObjectRaw| {
        if event.is_null() || xpc_object_is_error(event) {
            return;
        }
        let reply = match unsafe { xpc_dictionary_create_reply(event) } {
            reply if !reply.is_null() => match XpcObject::from_owned(reply) {
                Ok(reply) => reply,
                Err(_) => return,
            },
            _ => return,
        };

        let command = match command_from_xpc_event(event) {
            Ok(command) => command,
            Err((id, error)) => {
                send_reply(
                    &peer_for_handler,
                    reply,
                    OpheliaWireFrame::Error { id, error },
                );
                return;
            }
        };

        let client = client.clone();
        let peer = peer_for_handler.clone();
        runtime.spawn(async move {
            handle_peer_command(client, peer, reply, command).await;
        });
    });

    unsafe {
        xpc_connection_set_event_handler(peer.raw(), &handler);
        xpc_connection_activate(peer.raw());
    }
}

async fn handle_peer_command(
    client: OpheliaClient,
    peer: XpcConnection,
    reply: XpcObject,
    command: OpheliaWireCommand,
) {
    if matches!(command.command, OpheliaCommand::Subscribe) {
        handle_subscribe_command(client, peer, reply, command.id).await;
        return;
    }

    let frame = match client.dispatch(command.command).await {
        Ok(response) => OpheliaWireFrame::Response {
            id: command.id,
            response,
        },
        Err(error) => OpheliaWireFrame::Error {
            id: command.id,
            error,
        },
    };
    send_reply(&peer, reply, frame);
}

async fn handle_subscribe_command(
    client: OpheliaClient,
    peer: XpcConnection,
    reply: XpcObject,
    id: u64,
) {
    let mut subscription = match client.subscribe().await {
        Ok(subscription) => subscription,
        Err(error) => {
            send_reply(&peer, reply, OpheliaWireFrame::Error { id, error });
            return;
        }
    };

    send_reply(
        &peer,
        reply,
        OpheliaWireFrame::Response {
            id,
            response: OpheliaResponse::Snapshot {
                snapshot: subscription.snapshot.clone(),
            },
        },
    );

    loop {
        match subscription.next_event().await {
            Ok(event) => send_message(&peer, OpheliaWireFrame::Event { event }),
            Err(OpheliaError::Closed) => break,
            Err(error) => {
                send_message(&peer, OpheliaWireFrame::Error { id, error });
                break;
            }
        }
    }
}

fn send_reply(peer: &XpcConnection, reply: XpcObject, frame: OpheliaWireFrame) {
    if set_frame_payload(reply.raw(), &frame).is_ok() {
        unsafe { xpc_connection_send_message(peer.raw(), reply.raw()) };
    }
}

fn send_message(peer: &XpcConnection, frame: OpheliaWireFrame) {
    if let Ok(payload) = frame_to_payload(&frame)
        && let Ok(message) = message_from_payload(&payload)
    {
        unsafe { xpc_connection_send_message(peer.raw(), message.raw()) };
    }
}

fn command_from_xpc_event(event: XpcObjectRaw) -> Result<OpheliaWireCommand, (u64, OpheliaError)> {
    let payload = payload_from_xpc_object(event).map_err(|error| (0, error))?;
    command_from_payload(&payload).map_err(|error| (id_from_bad_command_payload(&payload), error))
}

fn frame_from_xpc_event(event: XpcObjectRaw) -> Result<OpheliaWireFrame, OpheliaError> {
    if event.is_null() || xpc_object_is_error(event) {
        return Err(OpheliaError::Closed);
    }
    let payload = payload_from_xpc_object(event)?;
    frame_from_payload(&payload)
}

fn message_from_payload(payload: &[u8]) -> Result<XpcObject, OpheliaError> {
    let object = unsafe { xpc_dictionary_create(ptr::null(), ptr::null(), 0) };
    let object = XpcObject::from_owned(object)?;
    set_payload(object.raw(), payload);
    Ok(object)
}

fn set_frame_payload(object: XpcObjectRaw, frame: &OpheliaWireFrame) -> Result<(), OpheliaError> {
    let payload = frame_to_payload(frame)?;
    set_payload(object, &payload);
    Ok(())
}

fn set_payload(object: XpcObjectRaw, payload: &[u8]) {
    unsafe {
        xpc_dictionary_set_data(
            object,
            payload_key(),
            payload.as_ptr().cast::<c_void>(),
            payload.len(),
        );
    }
}

fn payload_from_xpc_object(object: XpcObjectRaw) -> Result<Vec<u8>, OpheliaError> {
    let mut len = 0usize;
    let bytes = unsafe { xpc_dictionary_get_data(object, payload_key(), &mut len) };
    if bytes.is_null() {
        return Err(OpheliaError::Transport {
            message: "service XPC message did not include a payload".into(),
        });
    }
    Ok(unsafe { slice::from_raw_parts(bytes.cast::<u8>(), len).to_vec() })
}

fn payload_key() -> *const c_char {
    c"payload".as_ptr()
}

fn peer_is_same_user(peer: XpcConnectionRaw) -> bool {
    unsafe { xpc_connection_get_euid(peer) == libc::geteuid() }
}

fn xpc_object_is_error(object: XpcObjectRaw) -> bool {
    unsafe { xpc_get_type(object) == (&_xpc_type_error as *const u8).cast::<c_void>() }
}

fn id_from_bad_command_payload(payload: &[u8]) -> u64 {
    serde_json::from_slice::<serde_json::Value>(payload)
        .ok()
        .and_then(|value| value.get("id").and_then(serde_json::Value::as_u64))
        .unwrap_or(0)
}

#[derive(Debug)]
struct XpcObject {
    raw: XpcObjectRaw,
}

unsafe impl Send for XpcObject {}
unsafe impl Sync for XpcObject {}

impl XpcObject {
    fn from_owned(raw: XpcObjectRaw) -> Result<Self, OpheliaError> {
        if raw.is_null() {
            return Err(OpheliaError::Closed);
        }
        Ok(Self { raw })
    }

    fn raw(&self) -> XpcObjectRaw {
        self.raw
    }
}

impl Drop for XpcObject {
    fn drop(&mut self) {
        unsafe { xpc_release(self.raw) };
    }
}

#[derive(Debug)]
struct XpcConnection {
    raw: XpcConnectionRaw,
    cancel_on_drop: bool,
}

unsafe impl Send for XpcConnection {}
unsafe impl Sync for XpcConnection {}

impl XpcConnection {
    fn connect_client() -> Result<Self, OpheliaError> {
        let service_name =
            CString::new(OPHELIA_MACH_SERVICE_NAME).map_err(|error| OpheliaError::Transport {
                message: error.to_string(),
            })?;
        let raw = unsafe {
            xpc_connection_create_mach_service(service_name.as_ptr(), ptr::null_mut(), 0)
        };
        Self::from_owned(raw)
    }

    fn from_owned(raw: XpcConnectionRaw) -> Result<Self, OpheliaError> {
        if raw.is_null() {
            return Err(OpheliaError::Closed);
        }
        Ok(Self {
            raw,
            cancel_on_drop: true,
        })
    }

    fn retain(raw: XpcConnectionRaw) -> Result<Self, OpheliaError> {
        let retained = unsafe { xpc_retain(raw) };
        if retained.is_null() {
            return Err(OpheliaError::Closed);
        }
        Ok(Self {
            raw: retained,
            cancel_on_drop: false,
        })
    }

    fn raw(&self) -> XpcConnectionRaw {
        self.raw
    }
}

impl Clone for XpcConnection {
    fn clone(&self) -> Self {
        Self {
            raw: unsafe { xpc_retain(self.raw) },
            cancel_on_drop: false,
        }
    }
}

impl Drop for XpcConnection {
    fn drop(&mut self) {
        unsafe {
            if self.cancel_on_drop {
                xpc_connection_cancel(self.raw);
            }
            xpc_release(self.raw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xpc_payload_roundtrip_keeps_wire_frame() {
        let frame = OpheliaWireFrame::Error {
            id: 42,
            error: OpheliaError::BadRequest {
                message: "bad command".into(),
            },
        };
        let payload = frame_to_payload(&frame).unwrap();
        let object = message_from_payload(&payload).unwrap();
        let decoded = frame_from_xpc_event(object.raw()).unwrap();

        assert!(matches!(
            decoded,
            OpheliaWireFrame::Error {
                id: 42,
                error: OpheliaError::BadRequest { .. }
            }
        ));
    }
}
