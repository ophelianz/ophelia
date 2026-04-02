//! Settings window.
//!
//! Settings are held
//! in memory and written atomically when the user clicks Done. A
//! `SettingsClosed` event is emitted so the main window can update its
//! in-memory settings copy immediately.

use gpui::{
    div, prelude::*, px, App, ClickEvent, Context, EventEmitter, FontWeight,
    SharedString, Window,
};

use crate::platform;
use crate::settings::Settings;
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
}

impl EventEmitter<SettingsClosed> for SettingsWindow {}

impl SettingsWindow {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self { settings: Settings::load(), active: Section::General }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        let _ = self.settings.save();
        cx.emit(SettingsClosed { settings: self.settings.clone() });
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active;

        // Build section content first; borrows self.settings and cx briefly.
        let content = match active {
            Section::General => general::render(&self.settings, cx),
            Section::Network => network::render(&self.settings, cx),
        };

        let sections: &[(&str, Section)] = &[
            ("General", Section::General),
            ("Network", Section::Network),
        ];

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(Colors::background())
            .text_color(Colors::foreground())
            .font_family("Inter")
            // Titlebar strip
            // TODO: make this cross-platform
            .child(
                div()
                    .h(px(platform::TITLEBAR_HEIGHT))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .pl(px(platform::TRAFFIC_LIGHT_AREA))
                    .pr(px(24.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(Colors::muted_foreground())
                            .child("Settings"),
                    ),
            )
            // Sidebar + scrollable content
            .child(
                div()
                    .flex()
                    .flex_1()
                    .border_t_1()
                    .border_color(Colors::border())
                    .overflow_hidden()
                    // Left nav
                    .child(
                        div()
                            .w(px(160.0))
                            .flex_shrink_0()
                            .border_r_1()
                            .border_color(Colors::border())
                            .p(px(12.0))
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .children(sections.iter().map(|&(label, section)| {
                                let is_active = active == section;
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
                            }))
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
                            ),
                    )
                    // Scrollable content pane
                    .child(
                        div()
                            .id("settings-content")
                            .flex_1()
                            .overflow_y_scroll()
                            .p(px(32.0))
                            .child(content),
                    ),
            )
    }
}

// ---------------------------------------------------------------------------
// Shared UI helpers - accessible to general and network submodules via super::
// ---------------------------------------------------------------------------

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
