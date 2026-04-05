/***************************************************
** This file is part of Ophelia, distributed under the
** terms of the GPL License, version 3 or later.
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs, do no evil and behave plz )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

use gpui::{ParentElement, Styled, div, px};
use rust_i18n::t;

use crate::theme::Spacing;

use super::SettingsWindow;

pub(super) fn render(this: &SettingsWindow) -> gpui::Div {
    div()
        .flex_col()
        .gap(px(Spacing::SETTINGS_SECTION_GAP))
        .child(super::setting_row(
            t!("settings.general.language_label").to_string(),
            t!("settings.general.language_description").to_string(),
            super::setting_dropdown_select(this.language_select.clone()),
        ))
}
