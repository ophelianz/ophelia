use gpui::{ParentElement, Styled, div, px};

use super::SettingsWindow;

pub(super) fn render(this: &SettingsWindow) -> gpui::Div {
    div()
        .flex_col()
        .gap(px(20.0))
        .child(super::setting_row(
            "Global Speed Limit",
            "Caps total download bandwidth across all active downloads. Enter KB/s, or 0 for unlimited.",
            super::setting_text_input(this.global_speed_limit_input.clone()),
        ))
        .child(super::setting_row(
            "Concurrent Downloads",
            "Maximum number of downloads running at the same time",
            super::setting_text_input(this.concurrent_downloads_input.clone()),
        ))
        .child(super::setting_row(
            "Connections per Download",
            "Parallel chunk connections used for a single download",
            super::setting_text_input(this.connections_per_download_input.clone()),
        ))
        .child(super::setting_row(
            "Connections per Server",
            "Maximum simultaneous connections to a single hostname",
            super::setting_text_input(this.connections_per_server_input.clone()),
        ))
}
