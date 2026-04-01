mod app;
mod assets;
mod engine;
mod logging;
mod platform;
mod settings;
mod theme;
mod ui;
mod views;

use std::borrow::Cow;

use assets::Assets;
use gpui::{App, Application, Bounds, WindowBounds, WindowOptions, prelude::*, px, size};
use views::main_window::MainWindow;

fn run() {
    Application::new()
        .with_assets(Assets::new())
        .run(|cx: &mut App| {
            cx.text_system()
                .add_fonts(vec![
                    Cow::Owned(std::fs::read(
                        concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/Inter-VariableFont_opsz,wght.ttf")
                    ).unwrap()),
                ])
                .unwrap();

            let bounds = Bounds::centered(None, size(px(1120.), px(700.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: platform::titlebar_options(),
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| MainWindow::new(cx)),
            )
            .unwrap();
            cx.activate(true);
        });
}

fn main() {
    let _log_guard = logging::init();
    run();
}
