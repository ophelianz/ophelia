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

use gpui::{Context, IntoElement, ParentElement, Styled, div, px};
use rust_i18n::t;

use crate::theme::Spacing;
use crate::ui::prelude::Switch;

use super::SettingsWindow;

pub(super) fn render(this: &SettingsWindow, cx: &mut Context<SettingsWindow>) -> gpui::Div {
    div()
        .flex_col()
        .gap(px(Spacing::SETTINGS_SECTION_GAP))
        .child(super::setting_row(
            t!("settings.general.language_label").to_string(),
            t!("settings.general.language_description").to_string(),
            super::setting_dropdown_select(this.language_select.clone()),
        ))
        .child(super::setting_row(
            t!("settings.general.notifications_label").to_string(),
            t!("settings.general.notifications_description").to_string(),
            render_notifications_switch(this, cx),
        ))
}

fn render_notifications_switch(
    this: &SettingsWindow,
    cx: &mut Context<SettingsWindow>,
) -> impl IntoElement {
    let entity = cx.entity();

    Switch::new("notifications-enabled")
        .checked(this.settings.notifications_enabled)
        .on_click(move |checked, _, app| {
            let _ = entity.update(app, |this, cx| this.set_notifications_enabled(checked, cx));
        })
}
