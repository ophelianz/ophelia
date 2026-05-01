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

//! App-shell ownership for the embedded backend session.

use gpui::{App, BorrowAppContext, Global};
use ophelia::session::{SessionClient, SessionError, SessionHost};
use tokio::runtime::Handle;

use crate::ipc::IpcServer;
use crate::runtime::Tokio;
use crate::settings::Settings;

pub struct SessionServices {
    _session_host: SessionHost,
    ipc: IpcServer,
}

impl Global for SessionServices {}

pub fn start(settings: &Settings, cx: &mut App) -> Result<SessionClient, SessionError> {
    let runtime = Tokio::handle(cx);
    let session_host = SessionHost::start(&runtime, settings.core_config(), settings.core_paths())?;
    let session_client = session_host.client();
    let ipc = IpcServer::start(settings.ipc_port, &runtime, session_client.clone());
    install(session_host, ipc, cx);
    Ok(session_client)
}

pub fn install<C: BorrowAppContext>(session_host: SessionHost, ipc: IpcServer, cx: &mut C) {
    cx.set_global(SessionServices {
        _session_host: session_host,
        ipc,
    });
}

pub fn restart_ipc<C: BorrowAppContext>(
    port: u16,
    runtime: &Handle,
    session_client: SessionClient,
    cx: &mut C,
) {
    cx.update_global::<SessionServices, _>(|services, _| {
        services.ipc = IpcServer::start(port, runtime, session_client);
    });
}
