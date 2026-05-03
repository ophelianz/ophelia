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

use std::process::ExitCode;

#[cfg(target_os = "macos")]
fn main() -> ExitCode {
    use ophelia::service::{OPHELIA_MACH_SERVICE_NAME, OpheliaService, run_mach_service};

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("OPHELIA_LOG")
                .unwrap_or_else(|_| "ophelia=info,ophelia_service=info".into()),
        )
        .init();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to start Tokio runtime: {error}");
            return ExitCode::FAILURE;
        }
    };

    let paths = ophelia::ProfilePaths::default_profile();
    let service = match OpheliaService::start(runtime.handle(), paths) {
        Ok(service) => service,
        Err(error) => {
            eprintln!("failed to start Ophelia service: {error}");
            return ExitCode::FAILURE;
        }
    };

    tracing::info!(service = OPHELIA_MACH_SERVICE_NAME, "Ophelia service ready");

    match run_mach_service(runtime.handle(), service.client()) {
        Ok(_listener) => {
            runtime.block_on(service.wait());
            ExitCode::SUCCESS
        }
        Err(error) => {
            drop(service);
            eprintln!("failed to run Mach service: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() -> ExitCode {
    eprintln!("ophelia-service currently supports macOS only");
    ExitCode::FAILURE
}
