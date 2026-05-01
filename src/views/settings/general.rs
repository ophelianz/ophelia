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

use crate::build_info::{BuildInfo, updater_controls_enabled};
use crate::settings::UpdateChannel;
use crate::theme::Spacing;
use crate::ui::prelude::{SegmentedControl, SegmentedControlOption, Switch};

use super::SettingsWindow;

pub(super) fn render(this: &SettingsWindow, cx: &mut Context<SettingsWindow>) -> gpui::Div {
    let updater_controls_enabled = updater_controls_enabled();
    let content = div()
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
        .child(super::setting_row(
            t!("settings.general.auto_update_label").to_string(),
            t!("settings.general.auto_update_description").to_string(),
            render_auto_update_switch(this, cx, updater_controls_enabled),
        ))
        .child(super::setting_row(
            t!("settings.general.update_channel_label").to_string(),
            t!("settings.general.update_channel_description").to_string(),
            render_update_channel(this, cx, updater_controls_enabled),
        ));

    if updater_controls_enabled {
        content
    } else {
        content.child(
            div()
                .text_xs()
                .text_color(crate::theme::Colors::muted_foreground())
                .child(if BuildInfo::current().channel.is_dev() {
                    t!("settings.general.auto_update_dev_hint").to_string()
                } else {
                    t!("settings.general.auto_update_platform_hint").to_string()
                }),
        )
    }
}

fn render_notifications_switch(
    this: &SettingsWindow,
    cx: &mut Context<SettingsWindow>,
) -> impl IntoElement {
    let entity = cx.entity();

    Switch::new("notifications-enabled")
        .checked(this.settings.notifications_enabled)
        .on_click(move |checked, _, app| {
            entity.update(app, |this, cx| this.set_notifications_enabled(checked, cx));
        })
}

fn render_auto_update_switch(
    this: &SettingsWindow,
    cx: &mut Context<SettingsWindow>,
    enabled: bool,
) -> impl IntoElement {
    let entity = cx.entity();
    let switch = Switch::new("auto-update-enabled").checked(this.settings.auto_update_enabled);

    div()
        .opacity(if enabled { 1.0 } else { 0.6 })
        .child(if enabled {
            switch.on_click(move |checked, _, app| {
                entity.update(app, |this, cx| this.set_auto_update_enabled(checked, cx));
            })
        } else {
            switch
        })
}

fn render_update_channel(
    this: &SettingsWindow,
    cx: &mut Context<SettingsWindow>,
    enabled: bool,
) -> impl IntoElement {
    let entity = cx.entity();
    let stable = SegmentedControlOption::new(
        "update-channel-stable",
        t!("settings.general.update_channel_stable").to_string(),
    )
    .selected(this.settings.update_channel == UpdateChannel::Stable)
    .min_width(96.0);
    let nightly = SegmentedControlOption::new(
        "update-channel-nightly",
        t!("settings.general.update_channel_nightly").to_string(),
    )
    .selected(this.settings.update_channel == UpdateChannel::Nightly)
    .min_width(96.0);

    let stable = if enabled {
        let entity = entity.clone();
        stable.on_click(move |_, _, app| {
            entity.update(app, |this, cx| {
                this.set_update_channel(UpdateChannel::Stable, cx);
            });
        })
    } else {
        stable
    };
    let nightly = if enabled {
        nightly.on_click(move |_, _, app| {
            entity.update(app, |this, cx| {
                this.set_update_channel(UpdateChannel::Nightly, cx);
            });
        })
    } else {
        nightly
    };

    div().opacity(if enabled { 1.0 } else { 0.6 }).child(
        SegmentedControl::new("update-channel")
            .option(stable)
            .option(nightly),
    )
}
