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
    InstalledEventHandler, ManagedPeerConnection, ManagedXpcConnection, PeerCloser,
    XpcConnectionHandle, XpcObject, XpcObjectRaw, activate_connection, cancel_raw_connection,
    command_from_xpc_event, install_event_handler, peer_is_same_user, send_message,
    send_message_then_cancel_after_barrier, send_reply, xpc_dictionary_create_reply,
    xpc_object_is_error,
};
use crate::service::codec::{OpheliaCommandEnvelope, OpheliaFrameEnvelope};
use crate::service::{OpheliaClient, OpheliaCommand, OpheliaError, OpheliaResponse};
use block2::RcBlock;
use tokio::runtime::Handle;
use tokio::sync::watch;

pub struct MachServiceListener {
    _listener: ManagedXpcConnection,
    _handler: InstalledEventHandler,
}

pub fn run_mach_service(
    runtime: &Handle,
    client: OpheliaClient,
) -> Result<MachServiceListener, OpheliaError> {
    let listener = ManagedXpcConnection::connect_listener()?;
    let runtime = runtime.clone();
    let handler = RcBlock::new(move |peer: XpcObjectRaw| {
        if peer.is_null() {
            return;
        }
        if !peer_is_same_user(peer) {
            cancel_raw_connection(peer);
            return;
        }
        if let Ok(peer) = ManagedPeerConnection::retain(peer) {
            accept_peer_connection(peer, runtime.clone(), client.clone());
        }
    });
    let installed = install_event_handler(listener.handle(), &handler);
    activate_connection(listener.handle());
    Ok(MachServiceListener {
        _listener: listener,
        _handler: installed,
    })
}

fn accept_peer_connection(peer: ManagedPeerConnection, runtime: Handle, client: OpheliaClient) {
    let install_handle = peer.handle();
    let (disconnect_tx, disconnect_rx) = watch::channel(false);
    let handler = RcBlock::new(move |event: XpcObjectRaw| {
        if event.is_null() || xpc_object_is_error(event) {
            let _ = disconnect_tx.send(true);
            peer.cancel();
            return;
        }
        let reply = match unsafe { xpc_dictionary_create_reply(event) } {
            reply if !reply.is_null() => match XpcObject::from_owned(reply) {
                Ok(reply) => reply,
                Err(_) => return,
            },
            _ => return,
        };
        let peer_handle = peer.handle();

        let command = match command_from_xpc_event(event) {
            Ok(command) => command,
            Err((id, error)) => {
                send_reply(
                    &peer_handle,
                    reply,
                    OpheliaFrameEnvelope::Error { id, error },
                );
                return;
            }
        };

        let client = client.clone();
        let closer = peer.closer();
        let disconnected = disconnect_rx.clone();
        runtime.spawn(async move {
            handle_peer_command(client, peer_handle, closer, reply, command, disconnected).await;
        });
    });

    let _installed = install_event_handler(&install_handle, &handler);
    activate_connection(&install_handle);
}

async fn handle_peer_command(
    client: OpheliaClient,
    peer: XpcConnectionHandle,
    closer: PeerCloser,
    reply: XpcObject,
    command: OpheliaCommandEnvelope,
    disconnected: watch::Receiver<bool>,
) {
    if matches!(command.command, OpheliaCommand::Subscribe) {
        handle_subscribe_command(client, peer, closer, reply, command.id, disconnected).await;
        return;
    }

    let frame = match client.dispatch(command.command).await {
        Ok(response) => OpheliaFrameEnvelope::Response {
            id: command.id,
            response: Box::new(response),
        },
        Err(error) => OpheliaFrameEnvelope::Error {
            id: command.id,
            error,
        },
    };
    send_reply(&peer, reply, frame);
}

async fn handle_subscribe_command(
    client: OpheliaClient,
    peer: XpcConnectionHandle,
    closer: PeerCloser,
    reply: XpcObject,
    id: u64,
    mut disconnected: watch::Receiver<bool>,
) {
    let mut subscription = match client.subscribe().await {
        Ok(subscription) => subscription,
        Err(error) => {
            send_reply(&peer, reply, OpheliaFrameEnvelope::Error { id, error });
            return;
        }
    };

    send_reply(
        &peer,
        reply,
        OpheliaFrameEnvelope::Response {
            id,
            response: Box::new(OpheliaResponse::Snapshot {
                snapshot: Box::new(subscription.snapshot.clone()),
            }),
        },
    );

    loop {
        tokio::select! {
            changed = disconnected.changed() => {
                if changed.is_err() || *disconnected.borrow() {
                    closer.cancel();
                    break;
                }
            }
            update = subscription.next_update() => {
                match update {
                    Ok(update) => {
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
                            "sending Mach update batch"
                        );
                        send_message(&peer, OpheliaFrameEnvelope::Update {
                            update: Box::new(update),
                        });
                    }
                    Err(error @ OpheliaError::Closed) => {
                        send_terminal_error_then_close(&peer, closer, id, error);
                        return;
                    }
                    Err(error) => {
                        send_terminal_error_then_close(&peer, closer, id, error);
                        return;
                    }
                }
            }
        }
    }
}

fn send_terminal_error_then_close(
    peer: &XpcConnectionHandle,
    closer: PeerCloser,
    id: u64,
    error: OpheliaError,
) {
    send_message_then_cancel_after_barrier(peer, closer, OpheliaFrameEnvelope::Error { id, error });
}
