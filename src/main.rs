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
mod engine;
mod ipc;
mod logging;
mod platform;
mod settings;
mod theme;
mod tray;
mod ui;
mod views;

use assets::Assets;
use gpui::{App, Application, ApplicationActivationPolicy, QuitMode, prelude::*};

fn run() {
    let app = Application::new()
        .with_assets(Assets::new())
        .with_quit_mode(QuitMode::Explicit)
        .with_activation_policy(ApplicationActivationPolicy::Regular);
    app.on_reopen(|cx| {
        let _ = app_actions::ensure_main_window(cx);
    });
    app.run(|cx: &mut App| {
        let initial_settings = settings::Settings::load();
        rust_i18n::set_locale(initial_settings.resolved_language());

        let downloads = cx.new(|cx| app::Downloads::new(cx));

        app_menu::init(cx);
        app_actions::init(downloads, cx);
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
                    std::borrow::Cow::Owned(
                        std::fs::read(format!(
                            "{}/assets/fonts/{filename}",
                            env!("CARGO_MANIFEST_DIR")
                        ))
                        .unwrap(),
                    )
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
