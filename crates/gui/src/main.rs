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

rust_i18n::i18n!("locales", fallback = "en");

mod app;
mod app_actions;
mod app_menu;
mod assets;
mod build_info;
mod engine;
mod format;
mod ipc;
mod logging;
mod platform;
mod runtime;
mod service_services;
mod settings;
mod theme;
mod tray;
mod ui;
mod updater;
mod views;

use assets::Assets;
use gpui::{App, ApplicationActivationPolicy, QuitMode, prelude::*};
use gpui_platform::application;
use std::process::ExitCode;

#[cfg(target_os = "macos")]
fn run_development_service_if_requested() -> Option<ExitCode> {
    std::env::var_os(ophelia::service::OPHELIA_RUN_SERVICE_ENV)?;
    Some(run_development_service())
}

#[cfg(not(target_os = "macos"))]
fn run_development_service_if_requested() -> Option<ExitCode> {
    None
}

#[cfg(target_os = "macos")]
fn run_development_service() -> ExitCode {
    use ophelia::service::{OPHELIA_MACH_SERVICE_NAME, OpheliaService, run_mach_service};

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("OPHELIA_LOG").unwrap_or_else(|_| "ophelia=info,ophelia_gui=info".into()),
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

fn run() {
    let assets = Assets::new();
    let app = application()
        .with_assets(assets.clone())
        .with_quit_mode(QuitMode::Explicit)
        .with_activation_policy(ApplicationActivationPolicy::Regular);
    app.on_reopen(|cx| {
        let _ = app_actions::ensure_main_window(cx);
    });
    app.run(move |cx: &mut App| {
        runtime::init(cx);

        let initial_settings = settings::Settings::load();
        rust_i18n::set_locale(initial_settings.resolved_language());

        let service_client = match service_services::start(&initial_settings, cx) {
            Ok(client) => client,
            Err(error) => {
                tracing::error!(?error, "failed to start backend service");
                cx.quit();
                return;
            }
        };
        let downloads =
            cx.new(|cx| app::Downloads::new(service_client.clone(), initial_settings.clone(), cx));

        app_menu::init(cx);
        app_actions::init(downloads, cx);
        updater::init(initial_settings.clone(), cx);
        tray::init(cx);
        ui::chrome::modal::bind_actions(cx);
        ui::controls::number_input::init(cx);
        ui::controls::text_field::init(cx);

        cx.text_system()
            .add_fonts(
                [
                    "Inter-VariableFont_opsz,wght.ttf",
                    "IBMPlexSans-Light.ttf",
                    "IBMPlexSans-Regular.ttf",
                    "IBMPlexSans-Medium.ttf",
                    "IBMPlexSans-SemiBold.ttf",
                    "IBMPlexSans-Bold.ttf",
                ]
                .into_iter()
                .map(|filename| {
                    std::borrow::Cow::Owned(assets.read(format!("fonts/{filename}")).unwrap())
                })
                .collect(),
            )
            .unwrap();

        let _ = app_actions::ensure_main_window(cx);
        app_menu::refresh(cx);
        tray::refresh(cx, true);

        cx.activate(true);
    });
}

fn main() -> ExitCode {
    if let Some(exit_code) = run_development_service_if_requested() {
        return exit_code;
    }

    let _log_guard = logging::init();
    run();
    ExitCode::SUCCESS
}
