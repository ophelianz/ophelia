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

//! Settings window.
//!
//! Settings are held
//! in memory and written atomically when the user clicks Done. A
//! `SettingsClosed` event is emitted so the main window can update its
//! in-memory settings copy immediately.

use std::path::Path;

use gpui::{
    Context, Entity, EventEmitter, FontWeight, Pixels, SharedString,
    StatefulInteractiveElement as _, Window, div, prelude::*, px,
};
use rust_i18n::t;

use crate::settings::{
    CollisionStrategy, DestinationRule, Settings, canonical_language, default_destination_rules,
};
use crate::theme::APP_FONT_FAMILY;
use crate::ui::prelude::*;

mod destination_rule_icon_picker;
mod destinations;
mod general;
mod network;

const SETTINGS_SIDEBAR_MIN_WIDTH: f32 = 140.0;
const SETTINGS_SIDEBAR_MAX_WIDTH: f32 = 240.0;
const SETTINGS_CONTENT_MIN_WIDTH: f32 = 520.0;

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
    Destinations,
    Network,
}

impl Section {
    fn icon(self) -> IconName {
        match self {
            Section::General => IconName::GeneralSettings,
            Section::Destinations => IconName::Folder,
            Section::Network => IconName::Network,
        }
    }

    fn icon_color(self) -> gpui::Rgba {
        match self {
            Section::General => Colors::muted_foreground(),
            Section::Destinations => Colors::muted_foreground(),
            Section::Network => Colors::active(),
        }
    }
}

// ---------------------------------------------------------------------------
// Entity
// ---------------------------------------------------------------------------

pub struct SettingsWindow {
    initial_settings: Settings,
    pub settings: Settings,
    active: Section,
    pub(super) language_select: Entity<DropdownSelect>,
    pub(super) download_dir_input: Entity<DirectoryInput>,
    pub(super) destination_rule_editors: Vec<DestinationRuleEditor>,
    pub(super) global_speed_limit_input: Entity<NumberInput>,
    pub(super) ipc_port_input: Entity<NumberInput>,
    pub(super) concurrent_downloads_input: Entity<NumberInput>,
    pub(super) connections_per_download_input: Entity<NumberInput>,
    pub(super) connections_per_server_input: Entity<NumberInput>,
    next_destination_rule_index: usize,
    pub(super) open_icon_picker_rule: Option<usize>,
}

impl EventEmitter<SettingsClosed> for SettingsWindow {}

impl SettingsWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::from_settings(Settings::load(), cx)
    }

    fn from_settings(settings: Settings, cx: &mut Context<Self>) -> Self {
        let initial_settings = settings.clone();
        let selected_language = settings.resolved_language().to_string();
        let fallback_download_dir = settings.download_dir().to_string_lossy().to_string();
        let destination_rule_editors = settings
            .destination_rules
            .iter()
            .map(|rule| DestinationRuleEditor::from_rule(rule, cx))
            .collect::<Vec<_>>();
        let next_destination_rule_index = next_destination_rule_index(&settings.destination_rules);
        let language_select = cx.new(|cx| {
            DropdownSelect::new(
                "settings-language",
                language_options(),
                selected_language.clone(),
                cx,
            )
        });

        cx.subscribe(
            &language_select,
            |_this: &mut Self, _, _: &DropdownSelectChanged, cx| {
                cx.notify();
            },
        )
        .detach();

        Self {
            initial_settings,
            language_select,
            download_dir_input: cx.new(|cx| {
                DirectoryInput::new(
                    fallback_download_dir.clone(),
                    t!("settings.destinations.download_folder_placeholder").to_string(),
                    cx,
                )
            }),
            destination_rule_editors,
            global_speed_limit_input: cx.new(|cx| {
                NumberInput::new(
                    format!("{}", settings.global_speed_limit_bps / 1024),
                    t!("settings.network.global_speed_limit_placeholder").to_string(),
                    0,
                    1_000_000,
                    64,
                    cx,
                )
            }),
            ipc_port_input: cx.new(|cx| {
                NumberInput::new(
                    format!("{}", settings.ipc_port),
                    t!("settings.network.ipc_port_placeholder").to_string(),
                    1,
                    u16::MAX as u64,
                    1,
                    cx,
                )
            }),
            concurrent_downloads_input: cx.new(|cx| {
                NumberInput::new(
                    format!("{}", settings.max_concurrent_downloads),
                    t!("settings.network.concurrent_downloads_placeholder").to_string(),
                    1,
                    10,
                    1,
                    cx,
                )
            }),
            connections_per_download_input: cx.new(|cx| {
                NumberInput::new(
                    format!("{}", settings.max_connections_per_download),
                    t!("settings.network.connections_per_download_placeholder").to_string(),
                    1,
                    16,
                    1,
                    cx,
                )
            }),
            connections_per_server_input: cx.new(|cx| {
                NumberInput::new(
                    format!("{}", settings.max_connections_per_server),
                    t!("settings.network.connections_per_server_placeholder").to_string(),
                    1,
                    16,
                    1,
                    cx,
                )
            }),
            settings,
            active: Section::General,
            next_destination_rule_index,
            open_icon_picker_rule: None,
        }
    }

    #[cfg(test)]
    fn new_with_settings(settings: Settings, cx: &mut Context<Self>) -> Self {
        Self::from_settings(settings, cx)
    }

    fn draft_settings(&self, cx: &mut Context<Self>) -> Settings {
        let mut settings = self.settings.clone();
        settings.language =
            canonical_language(self.language_select.read(cx).selected_value()).to_string();
        settings.default_download_dir =
            parse_path_input(self.download_dir_input.read(cx).text(cx).as_ref());
        settings.global_speed_limit_bps = parse_speed_limit_input(
            self.global_speed_limit_input.read(cx).text(),
            settings.global_speed_limit_bps,
        );
        settings.ipc_port =
            parse_port_input(self.ipc_port_input.read(cx).text(), settings.ipc_port);
        let fallback_download_dir = settings.download_dir();
        settings.destination_rules = self
            .destination_rule_editors
            .iter()
            .enumerate()
            .map(|(index, rule)| rule.to_rule(index, &fallback_download_dir, cx))
            .collect();
        settings.max_concurrent_downloads = parse_bounded_usize_input(
            self.concurrent_downloads_input.read(cx).text(),
            settings.max_concurrent_downloads,
            1,
            10,
        );
        settings.max_connections_per_download = parse_bounded_usize_input(
            self.connections_per_download_input.read(cx).text(),
            settings.max_connections_per_download,
            1,
            16,
        );
        settings.max_connections_per_server = parse_bounded_usize_input(
            self.connections_per_server_input.read(cx).text(),
            settings.max_connections_per_server,
            1,
            16,
        );
        settings
    }

    fn needs_restart(&self, cx: &mut Context<Self>) -> bool {
        let draft = self.draft_settings(cx);
        draft.resolved_language() != self.initial_settings.resolved_language()
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        self.settings = self.draft_settings(cx);
        let _ = self.settings.save();
        cx.emit(SettingsClosed {
            settings: self.settings.clone(),
        });
    }

    fn save_and_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
        window.remove_window();
    }

    fn save_and_restart(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.save(cx);
        cx.restart();
    }

    pub(super) fn set_collision_strategy(
        &mut self,
        strategy: CollisionStrategy,
        cx: &mut Context<Self>,
    ) {
        if self.settings.collision_strategy != strategy {
            self.settings.collision_strategy = strategy;
            cx.notify();
        }
    }

    pub(super) fn set_destination_rules_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.settings.destination_rules_enabled != enabled {
            self.settings.destination_rules_enabled = enabled;
            cx.notify();
        }
    }

    pub(super) fn set_destination_rule_enabled(
        &mut self,
        index: usize,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(rule) = self.destination_rule_editors.get_mut(index)
            && rule.enabled != enabled
        {
            rule.enabled = enabled;
            cx.notify();
        }
    }

    pub(super) fn add_destination_rule(&mut self, cx: &mut Context<Self>) {
        let id = format!("destination-rule-{}", self.next_destination_rule_index);
        self.next_destination_rule_index += 1;
        let target_dir = self.download_dir_input.read(cx).text(cx).to_string();
        self.destination_rule_editors
            .push(DestinationRuleEditor::empty(id, target_dir, cx));
        cx.notify();
    }

    pub(super) fn restore_default_destination_rules(&mut self, cx: &mut Context<Self>) {
        let fallback_download_dir =
            parse_path_input(self.download_dir_input.read(cx).text(cx).as_ref())
                .unwrap_or_else(|| self.settings.download_dir());
        let default_rules = default_destination_rules(&fallback_download_dir);
        self.destination_rule_editors = default_rules
            .iter()
            .map(|rule| DestinationRuleEditor::from_rule(rule, cx))
            .collect();
        self.next_destination_rule_index = next_destination_rule_index(&default_rules);
        self.open_icon_picker_rule = None;
        cx.notify();
    }

    pub(super) fn remove_destination_rule(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.destination_rule_editors.len() {
            self.destination_rule_editors.remove(index);
            self.open_icon_picker_rule = match self.open_icon_picker_rule {
                Some(open_index) if open_index == index => None,
                Some(open_index) if open_index > index => Some(open_index - 1),
                current => current,
            };
            cx.notify();
        }
    }

    pub(super) fn toggle_destination_rule_icon_picker(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        self.open_icon_picker_rule = if self.open_icon_picker_rule == Some(index) {
            None
        } else {
            Some(index)
        };
        cx.notify();
    }

    pub(super) fn close_destination_rule_icon_picker(&mut self, cx: &mut Context<Self>) {
        if self.open_icon_picker_rule.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn set_destination_rule_icon(
        &mut self,
        index: usize,
        icon_name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(rule) = self.destination_rule_editors.get_mut(index) {
            rule.icon_name = icon_name;
            self.open_icon_picker_rule = None;
            cx.notify();
        }
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let needs_restart = self.needs_restart(cx);
        let header = if cfg!(target_os = "macos") {
            WindowHeader::new(t!("settings.title").to_string())
                .leading(div().w(px(24.0)))
                .when(needs_restart, |this| {
                    this.trailing(self.restart_required_indicator(cx))
                })
        } else {
            WindowHeader::new(t!("settings.title").to_string()).when(needs_restart, |this| {
                this.trailing(self.restart_required_indicator(cx))
            })
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(Colors::background())
            .text_color(Colors::foreground())
            .font_family(APP_FONT_FAMILY)
            .child(header)
            .child(
                div()
                    .flex_1()
                    .border_t_1()
                    .border_color(Colors::border())
                    .overflow_hidden()
                    .child(
                        h_resizable("settings-layout")
                            .child(
                                resizable_panel()
                                    .size(px(Spacing::SETTINGS_SIDEBAR_WIDTH))
                                    .size_range(
                                        px(SETTINGS_SIDEBAR_MIN_WIDTH)
                                            ..px(SETTINGS_SIDEBAR_MAX_WIDTH),
                                    )
                                    .child(self.render_sidebar(cx)),
                            )
                            .child(
                                resizable_panel()
                                    .size_range(px(SETTINGS_CONTENT_MIN_WIDTH)..Pixels::MAX)
                                    .child(
                                        div()
                                            .id("settings-content")
                                            .size_full()
                                            .min_w_0()
                                            .overflow_y_scroll()
                                            .p(px(Spacing::SETTINGS_CONTENT_PADDING))
                                            .child(self.render_content(cx)),
                                    ),
                            ),
                    ),
            )
    }
}

impl SettingsWindow {
    fn restart_required_indicator(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .items_center()
            .gap(px(Spacing::SETTINGS_INLINE_GAP))
            .child(
                Button::new(
                    "settings-restart-required",
                    t!("settings.restart_required_button").to_string(),
                )
                .compact()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.save_and_restart(window, cx);
                })),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(Colors::muted_foreground())
                    .child(t!("settings.restart_required_suffix").to_string()),
            )
    }
}

impl SettingsWindow {
    fn render_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        match self.active {
            Section::General => general::render(self).into_any_element(),
            Section::Destinations => destinations::render(self, cx).into_any_element(),
            Section::Network => network::render(self).into_any_element(),
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sections = [
            (t!("settings.general.section").to_string(), Section::General),
            (
                t!("settings.destinations.section").to_string(),
                Section::Destinations,
            ),
            (t!("settings.network.section").to_string(), Section::Network),
        ];
        let nav_items = sections
            .into_iter()
            .map(|(label, section)| {
                let is_active = self.active == section;

                div()
                    .id(SharedString::from(label.clone()))
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
                    .child(label)
            })
            .collect::<Vec<_>>();

        div()
            .w_full()
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
                    .on_click(cx.listener(|this, _, window, cx| this.save_and_close(window, cx)))
                    .child(t!("settings.done").to_string()),
            )
    }
}

pub(super) fn setting_directory_input(input: Entity<DirectoryInput>) -> gpui::Div {
    div().w(px(Spacing::SETTINGS_CONTROL_WIDTH)).child(input)
}

pub(super) fn setting_dropdown_select(input: Entity<DropdownSelect>) -> gpui::Div {
    div().w(px(Spacing::SETTINGS_CONTROL_WIDTH)).child(input)
}

pub(super) fn setting_number_input(input: Entity<NumberInput>) -> gpui::Div {
    div().w(px(Spacing::SETTINGS_CONTROL_WIDTH)).child(input)
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

fn parse_port_input(input: &str, fallback: u16) -> u16 {
    input
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .unwrap_or(fallback)
}

fn language_options() -> Vec<DropdownOption> {
    vec![
        DropdownOption::new(
            "en",
            t!("settings.general.language_option_english").to_string(),
        ),
        DropdownOption::new(
            "zh-CN",
            t!("settings.general.language_option_simplified_chinese").to_string(),
        ),
    ]
}

pub(super) fn parse_extensions_input(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn format_extensions_input(extensions: &[String]) -> String {
    extensions.join(", ")
}

fn next_destination_rule_index(rules: &[DestinationRule]) -> usize {
    rules
        .iter()
        .filter_map(|rule| {
            rule.id
                .strip_prefix("destination-rule-")
                .and_then(|suffix| suffix.parse::<usize>().ok())
        })
        .max()
        .map(|index| index + 1)
        .unwrap_or_else(|| rules.len() + 1)
}

pub(super) struct DestinationRuleEditor {
    pub(super) id: String,
    pub(super) enabled: bool,
    pub(super) icon_name: Option<String>,
    pub(super) label_input: Entity<TextField>,
    pub(super) extensions_input: Entity<TextField>,
    pub(super) target_dir_input: Entity<DirectoryInput>,
}

impl DestinationRuleEditor {
    fn from_rule(rule: &DestinationRule, cx: &mut Context<SettingsWindow>) -> Self {
        Self {
            id: rule.id.clone(),
            enabled: rule.enabled,
            icon_name: rule.icon_name.clone(),
            label_input: cx.new(|cx| {
                TextField::new(
                    rule.label.clone(),
                    t!("settings.destinations.destination_rule_label_placeholder").to_string(),
                    cx,
                )
            }),
            extensions_input: cx.new(|cx| {
                TextField::new(
                    format_extensions_input(&rule.extensions),
                    t!("settings.destinations.destination_rule_extensions_placeholder").to_string(),
                    cx,
                )
            }),
            target_dir_input: cx.new(|cx| {
                DirectoryInput::new(
                    rule.target_dir.to_string_lossy().to_string(),
                    t!("settings.destinations.destination_rule_directory_placeholder").to_string(),
                    cx,
                )
            }),
        }
    }

    fn empty(
        id: String,
        fallback_target_dir: impl Into<SharedString>,
        cx: &mut Context<SettingsWindow>,
    ) -> Self {
        Self {
            id,
            enabled: true,
            icon_name: None,
            label_input: cx.new(|cx| {
                TextField::new(
                    "",
                    t!("settings.destinations.destination_rule_label_placeholder").to_string(),
                    cx,
                )
            }),
            extensions_input: cx.new(|cx| {
                TextField::new(
                    "",
                    t!("settings.destinations.destination_rule_extensions_placeholder").to_string(),
                    cx,
                )
            }),
            target_dir_input: cx.new(|cx| {
                DirectoryInput::new(
                    fallback_target_dir,
                    t!("settings.destinations.destination_rule_directory_placeholder").to_string(),
                    cx,
                )
            }),
        }
    }

    fn to_rule(
        &self,
        index: usize,
        fallback_target_dir: &Path,
        cx: &Context<SettingsWindow>,
    ) -> DestinationRule {
        let label = self.label_input.read(cx).text().trim().to_string();
        let target_dir = parse_path_input(self.target_dir_input.read(cx).text(cx).as_ref())
            .unwrap_or_else(|| fallback_target_dir.to_path_buf());

        DestinationRule {
            id: self.id.clone(),
            label: if label.is_empty() {
                format!("Rule {}", index + 1)
            } else {
                label
            },
            enabled: self.enabled,
            target_dir,
            extensions: parse_extensions_input(self.extensions_input.read(cx).text()),
            icon_name: self.icon_name.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared UI helpers - accessible to general and network submodules via super::
// ---------------------------------------------------------------------------

fn setting_row(
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    control: impl IntoElement,
) -> gpui::Div {
    let label = label.into();
    let description = description.into();
    div()
        .flex()
        .flex_wrap()
        .items_start()
        .gap(px(Spacing::SETTINGS_ROW_GAP))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(Spacing::SETTINGS_LABEL_GAP))
                .flex_1()
                .min_w_0()
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
                        .truncate()
                        .child(description),
                ),
        )
        .child(div().flex_shrink_0().ml_auto().child(control))
}

#[cfg(test)]
mod tests {
    use std::ops::Deref;

    use gpui::{
        Bounds, MouseButton, TestApp, TestAppContext, Window, WindowBounds, WindowOptions, point,
        px,
    };

    use super::*;

    #[test]
    fn parses_comma_separated_extensions() {
        assert_eq!(
            parse_extensions_input(".mp3, flac,  .wav "),
            vec![".mp3", "flac", ".wav"]
        );
    }

    #[test]
    fn computes_next_destination_rule_index_from_existing_rules() {
        let rules = vec![
            DestinationRule {
                id: "destination-rule-2".into(),
                label: "Videos".into(),
                enabled: true,
                target_dir: std::path::PathBuf::from("/tmp/videos"),
                extensions: vec![".mp4".into()],
                icon_name: Some("video".into()),
            },
            DestinationRule {
                id: "music".into(),
                label: "Music".into(),
                enabled: true,
                target_dir: std::path::PathBuf::from("/tmp/music"),
                extensions: vec![".mp3".into()],
                icon_name: Some("audio".into()),
            },
        ];

        assert_eq!(next_destination_rule_index(&rules), 3);
    }

    #[test]
    fn language_dropdown_updates_draft_settings_via_real_ui_interaction() {
        let mut app = TestApp::new();
        let mut settings = Settings::default();
        settings.language = "en".into();

        let bounds = Bounds::from_corners(point(px(0.0), px(0.0)), point(px(800.0), px(600.0)));
        let mut window = app.open_window_with_options(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |_window: &mut Window, cx| SettingsWindow::new_with_settings(settings.clone(), cx),
        );
        window.draw();

        window.simulate_click(point(px(660.0), px(100.0)), MouseButton::Left);
        window.draw();
        window.simulate_click(point(px(660.0), px(198.0)), MouseButton::Left);

        window.update(|settings: &mut SettingsWindow, _window, cx| {
            assert_eq!(settings.draft_settings(cx).resolved_language(), "zh-CN");
            assert!(settings.needs_restart(cx));
        });
    }

    #[test]
    fn restart_button_requests_an_app_restart() {
        let mut app = TestAppContext::single();
        let mut settings = Settings::default();
        settings.language = "en".into();
        let mut restart = app.expect_restart();

        let bounds = Bounds::from_corners(point(px(0.0), px(0.0)), point(px(800.0), px(600.0)));
        let window = app.update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                move |_window: &mut Window, cx| {
                    cx.new(|cx| SettingsWindow::new_with_settings(settings.clone(), cx))
                },
            )
            .unwrap()
        });
        let view = window.root(&mut app).unwrap();
        let cx = gpui::VisualTestContext::from_window(*window.deref(), &app).into_mut();
        cx.run_until_parked();

        cx.simulate_click(point(px(660.0), px(100.0)), gpui::Modifiers::none());
        cx.simulate_click(point(px(660.0), px(198.0)), gpui::Modifiers::none());

        cx.update(|window, app| {
            view.update(app, |settings: &mut SettingsWindow, cx| {
                assert!(settings.needs_restart(cx));
                settings.save_and_restart(window, cx);
            });
        });

        assert!(restart.try_recv().ok().flatten().is_some());
    }
}
