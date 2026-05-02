use crate::service::wire::{
    OpheliaWireCommand, OpheliaWireFrame, command_from_payload, frame_from_payload,
    frame_to_payload,
};
use crate::service::{OPHELIA_MACH_SERVICE_NAME, OpheliaError};
use block2::Block;
use std::ffi::{CString, c_char, c_void};
use std::ptr;
use std::slice;

pub(super) type XpcObjectRaw = *mut c_void;
pub(super) type XpcConnectionRaw = *mut c_void;
type DispatchQueueRaw = *mut c_void;

const XPC_CONNECTION_MACH_SERVICE_LISTENER: u64 = 1 << 0;

unsafe extern "C" {
    fn xpc_connection_create_mach_service(
        name: *const c_char,
        targetq: DispatchQueueRaw,
        flags: u64,
    ) -> XpcConnectionRaw;
    pub(super) fn xpc_connection_set_event_handler(
        connection: XpcConnectionRaw,
        handler: &Block<dyn Fn(XpcObjectRaw)>,
    );
    pub(super) fn xpc_connection_activate(connection: XpcConnectionRaw);
    fn xpc_connection_cancel(connection: XpcConnectionRaw);
    fn xpc_connection_send_message(connection: XpcConnectionRaw, message: XpcObjectRaw);
    pub(super) fn xpc_connection_send_message_with_reply(
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
    pub(super) fn xpc_dictionary_create_reply(original: XpcObjectRaw) -> XpcObjectRaw;
    fn xpc_dictionary_get_remote_connection(dictionary: XpcObjectRaw) -> XpcConnectionRaw;
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

pub(super) fn send_reply(peer: &XpcConnection, reply: XpcObject, frame: OpheliaWireFrame) {
    if set_frame_payload(reply.raw(), &frame).is_ok() {
        unsafe { xpc_connection_send_message(peer.raw(), reply.raw()) };
    }
}

pub(super) fn send_message(peer: &XpcConnection, frame: OpheliaWireFrame) {
    if let Ok(payload) = frame_to_payload(&frame)
        && let Ok(message) = message_from_payload(&payload)
    {
        unsafe { xpc_connection_send_message(peer.raw(), message.raw()) };
    }
}

pub(super) fn command_from_xpc_event(
    event: XpcObjectRaw,
) -> Result<OpheliaWireCommand, (u64, OpheliaError)> {
    let payload = payload_from_xpc_object(event).map_err(|error| (0, error))?;
    command_from_payload(&payload).map_err(|error| (id_from_bad_command_payload(&payload), error))
}

pub(super) fn frame_from_xpc_event(event: XpcObjectRaw) -> Result<OpheliaWireFrame, OpheliaError> {
    if event.is_null() || xpc_object_is_error(event) {
        return Err(OpheliaError::Closed);
    }
    let payload = payload_from_xpc_object(event)?;
    frame_from_payload(&payload)
}

pub(super) fn message_from_payload(payload: &[u8]) -> Result<XpcObject, OpheliaError> {
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

pub(super) fn peer_is_same_user(peer: XpcConnectionRaw) -> bool {
    unsafe { xpc_connection_get_euid(peer) == libc::geteuid() }
}

pub(super) fn cancel_raw_connection(peer: XpcConnectionRaw) {
    unsafe { xpc_connection_cancel(peer) };
}

pub(super) fn xpc_object_is_error(object: XpcObjectRaw) -> bool {
    unsafe { xpc_get_type(object) == (&_xpc_type_error as *const u8).cast::<c_void>() }
}

fn id_from_bad_command_payload(payload: &[u8]) -> u64 {
    serde_json::from_slice::<serde_json::Value>(payload)
        .ok()
        .and_then(|value| value.get("id").and_then(serde_json::Value::as_u64))
        .unwrap_or(0)
}

#[derive(Debug)]
pub(super) struct XpcObject {
    raw: XpcObjectRaw,
}

// XPC objects are reference-counted OS objects; this wrapper only moves retained handles
unsafe impl Send for XpcObject {}
unsafe impl Sync for XpcObject {}

impl XpcObject {
    pub(super) fn from_owned(raw: XpcObjectRaw) -> Result<Self, OpheliaError> {
        if raw.is_null() {
            return Err(OpheliaError::Closed);
        }
        Ok(Self { raw })
    }

    pub(super) fn raw(&self) -> XpcObjectRaw {
        self.raw
    }
}

impl Drop for XpcObject {
    fn drop(&mut self) {
        unsafe { xpc_release(self.raw) };
    }
}

#[derive(Debug)]
pub(super) struct XpcConnection {
    raw: XpcConnectionRaw,
    cancel_on_drop: bool,
}

// XPC connections are thread-safe OS handles; drop releases our retained reference
unsafe impl Send for XpcConnection {}
unsafe impl Sync for XpcConnection {}

impl XpcConnection {
    pub(super) fn connect_client() -> Result<Self, OpheliaError> {
        Self::connect_with_flags(0)
    }

    pub(super) fn connect_listener() -> Result<Self, OpheliaError> {
        Self::connect_with_flags(XPC_CONNECTION_MACH_SERVICE_LISTENER)
    }

    fn connect_with_flags(flags: u64) -> Result<Self, OpheliaError> {
        let service_name =
            CString::new(OPHELIA_MACH_SERVICE_NAME).map_err(|error| OpheliaError::Transport {
                message: error.to_string(),
            })?;
        let raw = unsafe {
            xpc_connection_create_mach_service(service_name.as_ptr(), ptr::null_mut(), flags)
        };
        Self::from_owned(raw)
    }

    pub(super) fn from_owned(raw: XpcConnectionRaw) -> Result<Self, OpheliaError> {
        if raw.is_null() {
            return Err(OpheliaError::Closed);
        }
        Ok(Self {
            raw,
            cancel_on_drop: true,
        })
    }

    pub(super) fn retain(raw: XpcConnectionRaw) -> Result<Self, OpheliaError> {
        if raw.is_null() {
            return Err(OpheliaError::Closed);
        }
        let retained = unsafe { xpc_retain(raw) };
        if retained.is_null() {
            return Err(OpheliaError::Closed);
        }
        Ok(Self {
            raw: retained,
            cancel_on_drop: false,
        })
    }

    pub(super) fn retain_remote_from_message(message: XpcObjectRaw) -> Result<Self, OpheliaError> {
        let raw = unsafe { xpc_dictionary_get_remote_connection(message) };
        Self::retain(raw)
    }

    pub(super) fn cancel(&self) {
        unsafe { xpc_connection_cancel(self.raw) };
    }

    pub(super) fn raw(&self) -> XpcConnectionRaw {
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
    use crate::service::wire::{OpheliaWireFrame, frame_to_payload};

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
