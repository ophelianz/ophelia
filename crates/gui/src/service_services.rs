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

//! App-shell ownership for backend-side helpers.

use gpui::{App, BorrowAppContext, Global};
use ophelia::service::{OpheliaClient, OpheliaError};
use tokio::runtime::Handle;

use crate::ipc::IpcServer;
use crate::runtime::Tokio;
use crate::settings::Settings;

pub struct BackendServices {
    ipc: IpcServer,
}

impl Global for BackendServices {}

pub fn start(settings: &Settings, cx: &mut App) -> Result<OpheliaClient, OpheliaError> {
    let runtime = Tokio::handle(cx);
    let client = OpheliaClient::connect_local()?;
    let ipc = IpcServer::start(settings.ipc_port, &runtime, client.clone());
    install(ipc, cx);
    Ok(client)
}

pub fn install<C: BorrowAppContext>(ipc: IpcServer, cx: &mut C) {
    cx.set_global(BackendServices { ipc });
}

pub fn restart_ipc<C: BorrowAppContext>(
    port: u16,
    runtime: &Handle,
    client: OpheliaClient,
    cx: &mut C,
) {
    cx.update_global::<BackendServices, _>(|services, _| {
        services.ipc = IpcServer::start(port, runtime, client);
    });
}
