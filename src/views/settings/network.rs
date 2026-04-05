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
        .gap(px(Spacing::SETTINGS_GROUP_GAP))
        .child(super::setting_row(
            t!("settings.network.ipc_port_label").to_string(),
            t!("settings.network.ipc_port_description").to_string(),
            super::setting_number_input(this.ipc_port_input.clone()),
        ))
        .child(super::setting_row(
            t!("settings.network.global_speed_limit_label").to_string(),
            t!("settings.network.global_speed_limit_description").to_string(),
            super::setting_number_input(this.global_speed_limit_input.clone()),
        ))
        .child(super::setting_row(
            t!("settings.network.concurrent_downloads_label").to_string(),
            t!("settings.network.concurrent_downloads_description").to_string(),
            super::setting_number_input(this.concurrent_downloads_input.clone()),
        ))
        .child(super::setting_row(
            t!("settings.network.connections_per_download_label").to_string(),
            t!("settings.network.connections_per_download_description").to_string(),
            super::setting_number_input(this.connections_per_download_input.clone()),
        ))
        .child(super::setting_row(
            t!("settings.network.connections_per_server_label").to_string(),
            t!("settings.network.connections_per_server_description").to_string(),
            super::setting_number_input(this.connections_per_server_input.clone()),
        ))
}
