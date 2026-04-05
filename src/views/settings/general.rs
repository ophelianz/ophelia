use gpui::{ParentElement, Styled, div, px};
use rust_i18n::t;

use super::SettingsWindow;

pub(super) fn render(this: &SettingsWindow) -> gpui::Div {
    div().flex_col().gap(px(24.0)).child(super::setting_row(
        t!("settings.general.language_label").to_string(),
        t!("settings.general.language_description").to_string(),
        super::setting_dropdown_select(this.language_select.clone()),
    ))
}
