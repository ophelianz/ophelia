//! Settings panel modal.
//!
//! Full-window overlay with a left section nav and a content pane.
//! Changes are held in memory and written atomically on Done/close.
//! Engine-level changes (speed limit, connection counts) take effect
//! on the next launch; the in-memory Settings on Downloads is updated
//! immediately so the rest of the UI reflects the new values.

use gpui::{div, prelude::*, px, rgba, App, ClickEvent, Context, EventEmitter, FontWeight,
           SharedString, Window};

use crate::settings::Settings;
use crate::ui::prelude::*;

// Speed limit presets in bytes/sec. 0 = unlimited.
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

fn format_speed(bps: u64) -> SharedString {
    if bps == 0 {
        "Unlimited".into()
    } else if bps < 1024 * 1024 {
        format!("{} KB/s", bps / 1024).into()
    } else {
        format!("{} MB/s", bps / (1024 * 1024)).into()
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

pub struct SettingsClosed {
    pub settings: Settings,
}

// ---------------------------------------------------------------------------
// Entity
// ---------------------------------------------------------------------------

pub struct SettingsModal {
    pub settings: Settings,
    active_section: usize,
}

impl EventEmitter<SettingsClosed> for SettingsModal {}

impl SettingsModal {
    pub fn new() -> Self {
        Self { settings: Settings::load(), active_section: 0 }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        let _ = self.settings.save();
        cx.emit(SettingsClosed { settings: self.settings.clone() });
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

impl Render for SettingsModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let section = self.active_section;

        // Pre-compute section content first so that self and cx are free
        // for the nav and button listeners below.
        let section_content = if section == 0 {
            div().flex_col().gap(px(20.0)).children(self.general_rows(cx))
        } else {
            div().flex_col().gap(px(20.0)).children(self.network_rows(cx))
        };

        let sections: &[&str] = &["General", "Network"];

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000099))
            .child(
                div()
                    .w(px(660.0))
                    .rounded(px(14.0))
                    .border_1()
                    .border_color(Colors::border())
                    .bg(Colors::card())
                    .flex()
                    .flex_col()
                    // Header
                    .child(
                        div()
                            .px(px(24.0))
                            .py(px(18.0))
                            .border_b_1()
                            .border_color(Colors::border())
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(Colors::foreground())
                                    .child("Settings"),
                            )
                            .child(
                                div()
                                    .id("settings-close")
                                    .w(px(28.0))
                                    .h(px(28.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(6.0))
                                    .text_base()
                                    .text_color(Colors::muted_foreground())
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
                                    .child("✕"),
                            ),
                    )
                    // Body: nav + content
                    .child(
                        div()
                            .flex()
                            .min_h(px(360.0))
                            .overflow_hidden()
                            // Left nav
                            .child(
                                div()
                                    .w(px(156.0))
                                    .flex_shrink_0()
                                    .border_r_1()
                                    .border_color(Colors::border())
                                    .p(px(12.0))
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .children(sections.iter().enumerate().map(|(i, &label)| {
                                        let active = i == section;
                                        div()
                                            .id(i)
                                            .px(px(12.0))
                                            .py(px(8.0))
                                            .rounded(px(6.0))
                                            .text_sm()
                                            .font_weight(if active {
                                                FontWeight::SEMIBOLD
                                            } else {
                                                FontWeight::NORMAL
                                            })
                                            .text_color(if active {
                                                Colors::foreground()
                                            } else {
                                                Colors::muted_foreground()
                                            })
                                            .bg(if active {
                                                Colors::muted().into()
                                            } else {
                                                gpui::transparent_black()
                                            })
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.active_section = i;
                                                cx.notify();
                                            }))
                                            .child(SharedString::from(label))
                                    })),
                            )
                            // Content area
                            .child(
                                div()
                                    .id("settings-content")
                                    .flex_1()
                                    .overflow_y_scroll()
                                    .p(px(24.0))
                                    .child(section_content),
                            ),
                    )
                    // Footer
                    .child(
                        div()
                            .px(px(24.0))
                            .py(px(16.0))
                            .border_t_1()
                            .border_color(Colors::border())
                            .flex()
                            .justify_end()
                            .child(
                                div()
                                    .id("settings-done")
                                    .px(px(20.0))
                                    .py(px(9.0))
                                    .rounded(px(8.0))
                                    .bg(Colors::active())
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(Colors::background())
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
                                    .child("Done"),
                            ),
                    ),
            )
    }
}

// ---------------------------------------------------------------------------
// Section content builders
// ---------------------------------------------------------------------------

impl SettingsModal {
    fn general_rows(&self, _cx: &mut Context<Self>) -> Vec<gpui::Div> {
        let dir: SharedString = self.settings.download_dir().to_string_lossy().to_string().into();

        vec![setting_row(
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
        )]
    }

    fn network_rows(&self, cx: &mut Context<Self>) -> Vec<gpui::Div> {
        let speed_idx = speed_preset_idx(self.settings.global_speed_limit_bps);
        let speed_label = format_speed(self.settings.global_speed_limit_bps);
        let concurrent = self.settings.max_concurrent_downloads;
        let per_dl = self.settings.max_connections_per_download;
        let per_srv = self.settings.max_connections_per_server;

        vec![
            setting_row(
                "Global Speed Limit",
                "Caps total download bandwidth across all active downloads",
                stepper(
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
            ),
            setting_row(
                "Concurrent Downloads",
                "Maximum number of downloads running at the same time",
                stepper(
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
            ),
            setting_row(
                "Connections per Download",
                "Parallel chunk connections used for a single download",
                stepper(
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
            ),
            setting_row(
                "Connections per Server",
                "Maximum simultaneous connections to a single hostname",
                stepper(
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
            ),
        ]
    }
}

// ---------------------------------------------------------------------------
// UI helpers
// ---------------------------------------------------------------------------

/// Label + description on the left, a control on the right.
fn setting_row(label: &'static str, description: &'static str, control: impl IntoElement) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(24.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(Colors::foreground())
                        .child(label),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(Colors::muted_foreground())
                        .child(description),
                ),
        )
        .child(div().flex_shrink_0().child(control))
}

/// `−  value  +` stepper.
fn stepper<F1, F2>(
    value: SharedString,
    dec_id: &'static str,
    inc_id: &'static str,
    can_dec: bool,
    can_inc: bool,
    on_dec: F1,
    on_inc: F2,
) -> gpui::Div
where
    F1: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F2: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    div()
        .flex()
        .items_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(Colors::border())
        .overflow_hidden()
        .child(stepper_btn(dec_id, "−", can_dec, on_dec))
        .child(
            div()
                .px(px(10.0))
                .py(px(7.0))
                .min_w(px(72.0))
                .flex()
                .items_center()
                .justify_center()
                .border_l_1()
                .border_r_1()
                .border_color(Colors::border())
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(Colors::foreground())
                .child(value),
        )
        .child(stepper_btn(inc_id, "+", can_inc, on_inc))
}

fn stepper_btn<F>(
    id: &'static str,
    symbol: &'static str,
    enabled: bool,
    on_click: F,
) -> impl IntoElement
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    div()
        .id(id)
        .w(px(32.0))
        .h(px(32.0))
        .flex()
        .items_center()
        .justify_center()
        .text_base()
        .text_color(if enabled { Colors::foreground() } else { Colors::muted_foreground() })
        .when(enabled, move |el| el.cursor_pointer().on_click(on_click))
        .child(symbol)
}
