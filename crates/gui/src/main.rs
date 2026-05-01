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

        let service_client = service_services::start(&initial_settings, cx)
            .expect("failed to start backend service");
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

fn main() {
    let _log_guard = logging::init();
    run();
}
