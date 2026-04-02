use gpui::{div, prelude::*, px, Context};

use crate::settings::Settings;
use super::SettingsWindow;

const SPEED_PRESETS: &[u64] = &[
    0,
    256 * 1024,
    512 * 1024,
    1 * 1024 * 1024,
    2 * 1024 * 1024,
    5 * 1024 * 1024,
    10 * 1024 * 1024,
    25 * 1024 * 1024,
    50 * 1024 * 1024,
];

fn speed_preset_idx(bps: u64) -> usize {
    SPEED_PRESETS.iter().rposition(|&p| p <= bps).unwrap_or(0)
}

fn format_speed(bps: u64) -> gpui::SharedString {
    if bps == 0 {
        "Unlimited".into()
    } else if bps < 1024 * 1024 {
        format!("{} KB/s", bps / 1024).into()
    } else {
        format!("{} MB/s", bps / (1024 * 1024)).into()
    }
}

pub(super) fn render(settings: &Settings, cx: &mut Context<SettingsWindow>) -> gpui::Div {
    let speed_idx = speed_preset_idx(settings.global_speed_limit_bps);
    let speed_label = format_speed(settings.global_speed_limit_bps);
    let concurrent = settings.max_concurrent_downloads;
    let per_dl = settings.max_connections_per_download;
    let per_srv = settings.max_connections_per_server;

    div()
        .flex_col()
        .gap(px(20.0))
        .child(super::setting_row(
            "Global Speed Limit",
            "Caps total download bandwidth across all active downloads",
            super::stepper(
                speed_label,
                "speed-dec",
                "speed-inc",
                speed_idx > 0,
                speed_idx < SPEED_PRESETS.len() - 1,
                cx.listener(move |this, _, _, cx| {
                    let i = speed_preset_idx(this.settings.global_speed_limit_bps);
                    if i > 0 {
                        this.settings.global_speed_limit_bps = SPEED_PRESETS[i - 1];
                    }
                    cx.notify();
                }),
                cx.listener(move |this, _, _, cx| {
                    let i = speed_preset_idx(this.settings.global_speed_limit_bps);
                    if i < SPEED_PRESETS.len() - 1 {
                        this.settings.global_speed_limit_bps = SPEED_PRESETS[i + 1];
                    }
                    cx.notify();
                }),
            ),
        ))
        .child(super::setting_row(
            "Concurrent Downloads",
            "Maximum number of downloads running at the same time",
            super::stepper(
                format!("{concurrent}").into(),
                "concurrent-dec",
                "concurrent-inc",
                concurrent > 1,
                concurrent < 10,
                cx.listener(|this, _, _, cx| {
                    if this.settings.max_concurrent_downloads > 1 {
                        this.settings.max_concurrent_downloads -= 1;
                    }
                    cx.notify();
                }),
                cx.listener(|this, _, _, cx| {
                    if this.settings.max_concurrent_downloads < 10 {
                        this.settings.max_concurrent_downloads += 1;
                    }
                    cx.notify();
                }),
            ),
        ))
        .child(super::setting_row(
            "Connections per Download",
            "Parallel chunk connections used for a single download",
            super::stepper(
                format!("{per_dl}").into(),
                "per-dl-dec",
                "per-dl-inc",
                per_dl > 1,
                per_dl < 16,
                cx.listener(|this, _, _, cx| {
                    if this.settings.max_connections_per_download > 1 {
                        this.settings.max_connections_per_download -= 1;
                    }
                    cx.notify();
                }),
                cx.listener(|this, _, _, cx| {
                    if this.settings.max_connections_per_download < 16 {
                        this.settings.max_connections_per_download += 1;
                    }
                    cx.notify();
                }),
            ),
        ))
        .child(super::setting_row(
            "Connections per Server",
            "Maximum simultaneous connections to a single hostname",
            super::stepper(
                format!("{per_srv}").into(),
                "per-srv-dec",
                "per-srv-inc",
                per_srv > 1,
                per_srv < 16,
                cx.listener(|this, _, _, cx| {
                    if this.settings.max_connections_per_server > 1 {
                        this.settings.max_connections_per_server -= 1;
                    }
                    cx.notify();
                }),
                cx.listener(|this, _, _, cx| {
                    if this.settings.max_connections_per_server < 16 {
                        this.settings.max_connections_per_server += 1;
                    }
                    cx.notify();
                }),
            ),
        ))
}
