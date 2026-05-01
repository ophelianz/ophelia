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

//! GPUI-owned Tokio runtime for app services.

use gpui::{App, Global, ReadGlobal};
use tokio::runtime::{Handle, Runtime};

pub fn init(cx: &mut App) {
    // Zed uses a small GPUI-owned Tokio runtime for app services.
    // This keeps the same shape, but the worker count is not tuned for Ophelia yet.
    let worker_threads = std::thread::available_parallelism()
        .map(|threads| threads.get().clamp(2, 4))
        .unwrap_or(2);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()
        .expect("failed to initialize Tokio runtime");

    let handle = runtime.handle().clone();
    cx.set_global(GlobalTokio {
        runtime: Some(runtime),
        handle,
    });
}

struct GlobalTokio {
    runtime: Option<Runtime>,
    handle: Handle,
}

impl Global for GlobalTokio {}

impl Drop for GlobalTokio {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

pub struct Tokio;

impl Tokio {
    pub fn handle(cx: &App) -> Handle {
        GlobalTokio::global(cx).handle.clone()
    }
}
