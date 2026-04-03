//! Settings window.
//!
//! Settings are held
//! in memory and written atomically when the user clicks Done. A
//! `SettingsClosed` event is emitted so the main window can update its
//! in-memory settings copy immediately.

use gpui::{Context, Entity, EventEmitter, FontWeight, SharedString, Window, div, prelude::*, px};

use crate::settings::Settings;
use crate::theme::APP_FONT_FAMILY;
use crate::ui::prelude::*;

mod general;
mod network;

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

pub struct SettingsClosed {
    pub settings: Settings,
}

// ---------------------------------------------------------------------------
// Section routing
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    General,
    Network,
}

impl Section {
    fn icon(self) -> IconName {
        match self {
            Section::General => IconName::GeneralSettings,
            Section::Network => IconName::Network,
        }
    }

    fn icon_color(self) -> gpui::Rgba {
        match self {
            Section::General => Colors::muted_foreground(),
            Section::Network => Colors::active(),
        }
    }
}

// ---------------------------------------------------------------------------
// Entity
// ---------------------------------------------------------------------------

pub struct SettingsWindow {
    pub settings: Settings,
    active: Section,
    pub(super) download_dir_input: Entity<TextField>,
    pub(super) global_speed_limit_input: Entity<TextField>,
    pub(super) concurrent_downloads_input: Entity<TextField>,
    pub(super) connections_per_download_input: Entity<TextField>,
    pub(super) connections_per_server_input: Entity<TextField>,
}

impl EventEmitter<SettingsClosed> for SettingsWindow {}

impl SettingsWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let settings = Settings::load();

        Self {
            download_dir_input: cx.new(|cx| {
                TextField::new(
                    settings.download_dir().to_string_lossy().to_string(),
                    "Downloads folder",
                    cx,
                )
            }),
            global_speed_limit_input: cx.new(|cx| {
                TextField::new(
                    format!("{}", settings.global_speed_limit_bps / 1024),
                    "0",
                    cx,
                )
            }),
            concurrent_downloads_input: cx.new(|cx| {
                TextField::new(format!("{}", settings.max_concurrent_downloads), "3", cx)
            }),
            connections_per_download_input: cx.new(|cx| {
                TextField::new(
                    format!("{}", settings.max_connections_per_download),
                    "8",
                    cx,
                )
            }),
            connections_per_server_input: cx.new(|cx| {
                TextField::new(format!("{}", settings.max_connections_per_server), "4", cx)
            }),
            settings,
            active: Section::General,
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.settings.default_download_dir =
            parse_path_input(self.download_dir_input.read(cx).text());
        self.settings.global_speed_limit_bps = parse_speed_limit_input(
            self.global_speed_limit_input.read(cx).text(),
            self.settings.global_speed_limit_bps,
        );
        self.settings.max_concurrent_downloads = parse_bounded_usize_input(
            self.concurrent_downloads_input.read(cx).text(),
            self.settings.max_concurrent_downloads,
            1,
            10,
        );
        self.settings.max_connections_per_download = parse_bounded_usize_input(
            self.connections_per_download_input.read(cx).text(),
            self.settings.max_connections_per_download,
            1,
            16,
        );
        self.settings.max_connections_per_server = parse_bounded_usize_input(
            self.connections_per_server_input.read(cx).text(),
            self.settings.max_connections_per_server,
            1,
            16,
        );
        let _ = self.settings.save();
        cx.emit(SettingsClosed {
            settings: self.settings.clone(),
        });
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(Colors::background())
            .text_color(Colors::foreground())
            .font_family(APP_FONT_FAMILY)
            .child(WindowHeader::new("Settings"))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .border_t_1()
                    .border_color(Colors::border())
                    .overflow_hidden()
                    .child(self.render_sidebar(cx))
                    .child(
                        div()
                            .id("settings-content")
                            .flex_1()
                            .overflow_y_scroll()
                            .p(px(32.0))
                            .child(self.render_content(cx)),
                    ),
            )
    }
}

impl SettingsWindow {
    fn render_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        match self.active {
            Section::General => general::render(self, cx).into_any_element(),
            Section::Network => network::render(self).into_any_element(),
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sections = [("General", Section::General), ("Network", Section::Network)];
        let nav_items = sections
            .into_iter()
            .map(|(label, section)| {
                let is_active = self.active == section;

                div()
                    .id(SharedString::from(label))
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_sm()
                    .font_weight(if is_active {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if is_active {
                        Colors::foreground()
                    } else {
                        Colors::muted_foreground()
                    })
                    .bg(if is_active {
                        Colors::muted().into()
                    } else {
                        gpui::transparent_black()
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active = section;
                        cx.notify();
                    }))
                    .child(icon_m(section.icon(), section.icon_color()))
                    .child(SharedString::from(label))
            })
            .collect::<Vec<_>>();

        div()
            .w(px(160.0))
            .flex_shrink_0()
            .border_r_1()
            .border_color(Colors::border())
            .p(px(12.0))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(nav_items)
            .child(div().flex_1())
            .child(
                div()
                    .id("settings-done")
                    .mx(px(4.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(6.0))
                    .bg(Colors::active())
                    .flex()
                    .justify_center()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(Colors::background())
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
                    .child("Done"),
            )
    }
}

pub(super) fn setting_text_input(input: Entity<TextField>) -> gpui::Div {
    div().w(px(220.0)).child(input)
}

fn parse_path_input(input: &str) -> Option<std::path::PathBuf> {
    let trimmed = input.trim();
    (!trimmed.is_empty()).then(|| std::path::PathBuf::from(trimmed))
}

fn parse_speed_limit_input(input: &str, fallback_bps: u64) -> u64 {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return fallback_bps;
    }

    trimmed
        .parse::<u64>()
        .map(|kbps| kbps.saturating_mul(1024))
        .unwrap_or(fallback_bps)
}

fn parse_bounded_usize_input(input: &str, fallback: usize, min: usize, max: usize) -> usize {
    input
        .trim()
        .parse::<usize>()
        .map(|value| value.clamp(min, max))
        .unwrap_or(fallback)
}

// ---------------------------------------------------------------------------
// Shared UI helpers - accessible to general and network submodules via super::
// ---------------------------------------------------------------------------

fn setting_row(
    label: &'static str,
    description: &'static str,
    control: impl IntoElement,
) -> gpui::Div {
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
