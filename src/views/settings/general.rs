use gpui::{div, prelude::*, px, Context};

use crate::settings::Settings;
use crate::ui::prelude::*;

use super::SettingsWindow;

pub(super) fn render(settings: &Settings, _cx: &mut Context<SettingsWindow>) -> gpui::Div {
    let dir: gpui::SharedString =
        settings.download_dir().to_string_lossy().to_string().into();

    div()
        .flex_col()
        .gap(px(20.0))
        .child(super::setting_row(
            "Download Folder",
            "Where files are saved when no destination is specified",
            div()
                .px(px(12.0))
                .py(px(7.0))
                .max_w(px(220.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(Colors::border())
                .text_sm()
                .text_color(Colors::muted_foreground())
                .overflow_hidden()
                .child(dir),
        ))
}
