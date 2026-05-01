use super::ffi::{
    XpcConnection, XpcObject, XpcObjectRaw, cancel_raw_connection, command_from_xpc_event,
    peer_is_same_user, send_message, send_reply, xpc_connection_activate,
    xpc_connection_set_event_handler, xpc_dictionary_create_reply, xpc_object_is_error,
};
use crate::service::wire::{OpheliaWireCommand, OpheliaWireFrame};
use crate::service::{OpheliaClient, OpheliaCommand, OpheliaError, OpheliaResponse};
use block2::RcBlock;
use tokio::runtime::Handle;
use tokio::sync::watch;

pub fn run_mach_service(runtime: &Handle, client: OpheliaClient) -> Result<(), OpheliaError> {
    let listener = XpcConnection::connect_listener()?;
    let runtime = runtime.clone();
    let handler = RcBlock::new(move |peer: XpcObjectRaw| {
        if peer.is_null() {
            return;
        }
        if !peer_is_same_user(peer) {
            cancel_raw_connection(peer);
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

fn accept_peer_connection(peer: XpcConnection, runtime: Handle, client: OpheliaClient) {
    let peer_for_handler = peer.clone();
    let (disconnect_tx, disconnect_rx) = watch::channel(false);
    let handler = RcBlock::new(move |event: XpcObjectRaw| {
        if event.is_null() || xpc_object_is_error(event) {
            let _ = disconnect_tx.send(true);
            peer_for_handler.cancel();
            return;
        }
        let reply = match unsafe { xpc_dictionary_create_reply(event) } {
            reply if !reply.is_null() => match XpcObject::from_owned(reply) {
                Ok(reply) => reply,
                Err(_) => return,
            },
            _ => return,
        };
        let peer = match XpcConnection::retain_remote_from_message(event) {
            Ok(peer) => peer,
            Err(error) => {
                send_reply(
                    &peer_for_handler,
                    reply,
                    OpheliaWireFrame::Error { id: 0, error },
                );
                return;
            }
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
        let disconnected = disconnect_rx.clone();
        runtime.spawn(async move {
            handle_peer_command(client, peer, reply, command, disconnected).await;
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
    disconnected: watch::Receiver<bool>,
) {
    if matches!(command.command, OpheliaCommand::Subscribe) {
        handle_subscribe_command(client, peer, reply, command.id, disconnected).await;
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
    mut disconnected: watch::Receiver<bool>,
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
        tokio::select! {
            changed = disconnected.changed() => {
                if changed.is_err() || *disconnected.borrow() {
                    break;
                }
            }
            event = subscription.next_event() => {
                match event {
                    Ok(event) => send_message(&peer, OpheliaWireFrame::Event { event }),
                    Err(OpheliaError::Closed) => break,
                    Err(error) => {
                        send_message(&peer, OpheliaWireFrame::Error { id, error });
                        break;
                    }
                }
            }
        }
    }

    peer.cancel();
}
