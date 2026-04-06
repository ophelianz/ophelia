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
mod ui;
mod views;

use assets::Assets;
use gpui::{App, Application, Bounds, prelude::*, px, size};
use views::main::main_window::MainWindow;

const MAIN_WINDOW_MIN_WIDTH: f32 = 960.0;
const MAIN_WINDOW_MIN_HEIGHT: f32 = 620.0;

fn run() {
    Application::new()
        .with_assets(Assets::new())
        .run(|cx: &mut App| {
            let initial_settings = settings::Settings::load();
            rust_i18n::set_locale(initial_settings.resolved_language());

            app_menu::init(cx);
            app_actions::init(cx);
            ui::chrome::modal::bind_actions(cx);
            ui::controls::number_input::init(cx);
            ui::controls::text_field::init(cx);

            cx.text_system()
                .add_fonts(vec![std::borrow::Cow::Owned(
                    std::fs::read(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/assets/fonts/Inter-VariableFont_opsz,wght.ttf"
                    ))
                    .unwrap(),
                )])
                .unwrap();

            let bounds = Bounds::centered(None, size(px(1280.), px(720.)), cx);
            let main_window = cx
                .open_window(
                    platform::window_options(
                        bounds,
                        size(px(MAIN_WINDOW_MIN_WIDTH), px(MAIN_WINDOW_MIN_HEIGHT)),
                    ),
                    |_, cx| cx.new(|cx| MainWindow::new(cx)),
                )
                .unwrap();

            app_actions::set_main_window(main_window, cx);

            app_menu::refresh(cx);

            cx.activate(true);
        });
}

fn main() {
    let _log_guard = logging::init();
    run();
}
