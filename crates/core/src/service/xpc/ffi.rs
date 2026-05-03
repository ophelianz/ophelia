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

use crate::service::codec::{
    OpheliaCommandEnvelope, OpheliaFrameEnvelope, command_from_body, frame_from_body, frame_to_body,
};
use crate::service::{OPHELIA_MACH_SERVICE_NAME, OpheliaError};
use block2::{Block, RcBlock};
use std::ffi::{CString, c_char, c_void};
use std::ptr;
use std::slice;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

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
    fn xpc_connection_send_barrier(connection: XpcConnectionRaw, barrier: &Block<dyn Fn()>);
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

pub(super) fn send_reply(
    peer: &XpcConnectionHandle,
    reply: XpcObject,
    frame: OpheliaFrameEnvelope,
) {
    if set_frame_body(reply.raw(), &frame).is_ok() {
        unsafe { xpc_connection_send_message(peer.raw(), reply.raw()) };
    }
}

pub(super) fn send_message(peer: &XpcConnectionHandle, frame: OpheliaFrameEnvelope) {
    let _ = send_frame_message(peer, frame);
}

pub(super) fn send_message_then_cancel_after_barrier(
    peer: &XpcConnectionHandle,
    closer: PeerCloser,
    frame: OpheliaFrameEnvelope,
) {
    if send_frame_message(peer, frame).is_err() {
        closer.cancel();
        return;
    }

    // SAFETY: xpc_connection_send_barrier copies the block before returning and
    // runs it after previously submitted sends have been handed to the XPC
    // transport. The barrier is a local transport flush point, not a remote ACK.
    let barrier = RcBlock::new(move || {
        closer.cancel();
    });
    unsafe { xpc_connection_send_barrier(peer.raw(), &barrier) };
}

fn send_frame_message(
    peer: &XpcConnectionHandle,
    frame: OpheliaFrameEnvelope,
) -> Result<(), OpheliaError> {
    let body = frame_to_body(&frame)?;
    let message = message_from_body(&body)?;
    unsafe { xpc_connection_send_message(peer.raw(), message.raw()) };
    Ok(())
}

pub(super) fn command_from_xpc_event(
    event: XpcObjectRaw,
) -> Result<OpheliaCommandEnvelope, (u64, OpheliaError)> {
    let body = body_from_xpc_object(event).map_err(|error| (0, error))?;
    command_from_body(&body).map_err(|error| (id_from_bad_command_body(&body), error))
}

pub(super) fn frame_from_xpc_event(
    event: XpcObjectRaw,
) -> Result<OpheliaFrameEnvelope, OpheliaError> {
    if event.is_null() || xpc_object_is_error(event) {
        return Err(OpheliaError::Closed);
    }
    let body = body_from_xpc_object(event)?;
    frame_from_body(&body)
}

pub(super) fn message_from_body(body: &[u8]) -> Result<XpcObject, OpheliaError> {
    let object = unsafe { xpc_dictionary_create(ptr::null(), ptr::null(), 0) };
    let object = XpcObject::from_owned(object)?;
    set_body(object.raw(), body);
    Ok(object)
}

fn set_frame_body(object: XpcObjectRaw, frame: &OpheliaFrameEnvelope) -> Result<(), OpheliaError> {
    let body = frame_to_body(frame)?;
    set_body(object, &body);
    Ok(())
}

fn set_body(object: XpcObjectRaw, body: &[u8]) {
    unsafe {
        xpc_dictionary_set_data(
            object,
            body_key(),
            body.as_ptr().cast::<c_void>(),
            body.len(),
        );
    }
}

fn body_from_xpc_object(object: XpcObjectRaw) -> Result<Vec<u8>, OpheliaError> {
    let mut len = 0usize;
    let bytes = unsafe { xpc_dictionary_get_data(object, body_key(), &mut len) };
    if bytes.is_null() {
        return Err(OpheliaError::Transport {
            message: "service XPC message did not include a body".into(),
        });
    }
    // SAFETY: xpc_dictionary_get_data returns memory borrowed from the XPC
    // dictionary. The slice is copied immediately, so no borrowed XPC payload
    // crosses into Tokio tasks or Rust-owned service state.
    Ok(unsafe { slice::from_raw_parts(bytes.cast::<u8>(), len).to_vec() })
}

fn body_key() -> *const c_char {
    c"body".as_ptr()
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

fn id_from_bad_command_body(body: &[u8]) -> u64 {
    body.get(2..10)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_le_bytes)
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
pub(super) struct XpcConnectionHandle {
    raw: XpcConnectionRaw,
}

// XPC connections are thread-safe OS handles; this type only retains/releases.
unsafe impl Send for XpcConnectionHandle {}
unsafe impl Sync for XpcConnectionHandle {}

impl XpcConnectionHandle {
    pub(super) fn from_owned(raw: XpcConnectionRaw) -> Result<Self, OpheliaError> {
        if raw.is_null() {
            return Err(OpheliaError::Closed);
        }
        Ok(Self { raw })
    }

    pub(super) fn retain(raw: XpcConnectionRaw) -> Result<Self, OpheliaError> {
        if raw.is_null() {
            return Err(OpheliaError::Closed);
        }
        let retained = unsafe { xpc_retain(raw) };
        if retained.is_null() {
            return Err(OpheliaError::Closed);
        }
        Ok(Self { raw: retained })
    }

    pub(super) fn raw(&self) -> XpcConnectionRaw {
        self.raw
    }
}

impl Clone for XpcConnectionHandle {
    fn clone(&self) -> Self {
        Self::retain(self.raw).expect("retaining a live XPC connection")
    }
}

impl Drop for XpcConnectionHandle {
    fn drop(&mut self) {
        unsafe { xpc_release(self.raw) };
    }
}

#[derive(Debug)]
pub(super) struct ManagedXpcConnection {
    handle: XpcConnectionHandle,
}

// Managed connections are normal client/listener handles; drop cancels then releases.
unsafe impl Send for ManagedXpcConnection {}
unsafe impl Sync for ManagedXpcConnection {}

impl ManagedXpcConnection {
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
        Ok(Self {
            handle: XpcConnectionHandle::from_owned(raw)?,
        })
    }

    pub(super) fn handle(&self) -> &XpcConnectionHandle {
        &self.handle
    }

    pub(super) fn cancel(&self) {
        unsafe { xpc_connection_cancel(self.handle.raw()) };
    }
}

impl Drop for ManagedXpcConnection {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Debug)]
pub(super) struct ManagedPeerConnection {
    handle: XpcConnectionHandle,
    cancelled: Arc<AtomicBool>,
}

// Accepted peers are retained XPC handles owned by the installed event handler.
unsafe impl Send for ManagedPeerConnection {}
unsafe impl Sync for ManagedPeerConnection {}

impl ManagedPeerConnection {
    pub(super) fn retain(raw: XpcConnectionRaw) -> Result<Self, OpheliaError> {
        Ok(Self {
            handle: XpcConnectionHandle::retain(raw)?,
            cancelled: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(super) fn handle(&self) -> XpcConnectionHandle {
        self.handle.clone()
    }

    pub(super) fn closer(&self) -> PeerCloser {
        PeerCloser {
            handle: self.handle.clone(),
            cancelled: self.cancelled.clone(),
        }
    }

    pub(super) fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            unsafe { xpc_connection_cancel(self.handle.raw()) };
        }
    }
}

impl Drop for ManagedPeerConnection {
    fn drop(&mut self) {
        // SAFETY: canceling the accepted peer is the retain-cycle breaker. XPC
        // owns a copied event-handler block, and that block owns this managed
        // peer. Cancel releases the installed handler asynchronously, allowing
        // the captured Rust state to drop.
        self.cancel();
    }
}

#[derive(Clone, Debug)]
pub(super) struct PeerCloser {
    handle: XpcConnectionHandle,
    cancelled: Arc<AtomicBool>,
}

impl PeerCloser {
    pub(super) fn cancel(&self) {
        if !self.cancelled.swap(true, Ordering::AcqRel) {
            unsafe { xpc_connection_cancel(self.handle.raw()) };
        }
    }
}

#[derive(Debug)]
pub(super) struct InstalledEventHandler;

pub(super) fn install_event_handler(
    connection: &XpcConnectionHandle,
    handler: &Block<dyn Fn(XpcObjectRaw)>,
) -> InstalledEventHandler {
    // SAFETY: XPC copies the Objective-C block for the connection. The marker
    // makes that C-level ownership visible in Rust; cancellation releases the
    // installed handler.
    unsafe { xpc_connection_set_event_handler(connection.raw(), handler) };
    InstalledEventHandler
}

pub(super) fn activate_connection(connection: &XpcConnectionHandle) {
    unsafe { xpc_connection_activate(connection.raw()) };
}

#[cfg(test)]
mod connection_tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn connection_roles_are_send_sync() {
        assert_send_sync::<XpcConnectionHandle>();
        assert_send_sync::<ManagedXpcConnection>();
        assert_send_sync::<ManagedPeerConnection>();
        assert_send_sync::<PeerCloser>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::codec::{OpheliaFrameEnvelope, frame_to_body};

    #[test]
    fn xpc_body_roundtrip_keeps_frame() {
        let frame = OpheliaFrameEnvelope::Error {
            id: 42,
            error: OpheliaError::BadRequest {
                message: "bad command".into(),
            },
        };
        let body = frame_to_body(&frame).unwrap();
        let object = message_from_body(&body).unwrap();
        let decoded = frame_from_xpc_event(object.raw()).unwrap();

        assert!(matches!(
            decoded,
            OpheliaFrameEnvelope::Error {
                id: 42,
                error: OpheliaError::BadRequest { .. }
            }
        ));
    }
}
