use gpui::{Context, div, prelude::*, px};
use rust_i18n::t;

use super::SettingsWindow;

pub(super) fn render(this: &SettingsWindow, _cx: &mut Context<SettingsWindow>) -> gpui::Div {
    div().flex_col().gap(px(20.0)).child(super::setting_row(
        t!("settings.general.download_folder_label").to_string(),
        t!("settings.general.download_folder_description").to_string(),
        super::setting_directory_input(this.download_dir_input.clone()),
    ))
}
