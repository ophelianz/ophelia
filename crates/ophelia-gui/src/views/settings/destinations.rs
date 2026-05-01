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

#[cfg(debug_assertions)]
use std::sync::{
    OnceLock,
    atomic::{AtomicUsize, Ordering},
};

use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::*, px};
use rust_i18n::t;

use crate::settings::{
    CollisionStrategy, HttpDownloadOrderingMode, suggested_destination_rule_icon_name,
};
use crate::ui::prelude::*;

use super::{DestinationRuleEditor, SettingsWindow};

pub(super) fn render(this: &SettingsWindow, cx: &mut Context<SettingsWindow>) -> gpui::Div {
    debug_destinations_render(this.destination_rule_editors.len());

    div()
        .flex_col()
        .gap(px(Spacing::SETTINGS_SECTION_GAP))
        .child(super::setting_row(
            t!("settings.destinations.download_folder_label").to_string(),
            t!("settings.destinations.download_folder_description").to_string(),
            super::setting_directory_input(this.download_dir_input.clone()),
        ))
        .child(super::setting_row(
            t!("settings.destinations.collision_strategy_label").to_string(),
            t!("settings.destinations.collision_strategy_description").to_string(),
            render_collision_strategy(this, cx),
        ))
        .child(super::setting_row(
            t!("settings.destinations.http_download_ordering_mode_label").to_string(),
            t!("settings.destinations.http_download_ordering_mode_description").to_string(),
            render_http_download_ordering_mode(this, cx),
        ))
        .child(super::setting_row(
            t!("settings.destinations.sequential_download_extensions_label").to_string(),
            t!("settings.destinations.sequential_download_extensions_description").to_string(),
            render_sequential_download_extensions(this),
        ))
        .child(super::setting_row(
            t!("settings.destinations.destination_rules_enabled_label").to_string(),
            t!("settings.destinations.destination_rules_enabled_description").to_string(),
            render_destination_rules_switch(this, cx),
        ))
        .child(render_destination_rules_section(this, cx))
}

fn render_collision_strategy(
    this: &SettingsWindow,
    cx: &mut Context<SettingsWindow>,
) -> impl IntoElement {
    let entity = cx.entity();

    SegmentedControl::new("collision-strategy")
        .option(
            SegmentedControlOption::new(
                "collision-strategy-rename",
                t!("settings.destinations.collision_strategy_rename").to_string(),
            )
            .selected(this.settings.collision_strategy == CollisionStrategy::Rename)
            .min_width(88.0)
            .on_click({
                let entity = entity.clone();
                move |_, _, app| {
                    entity.update(app, |this, cx| {
                        this.set_collision_strategy(CollisionStrategy::Rename, cx);
                    });
                }
            }),
        )
        .option(
            SegmentedControlOption::new(
                "collision-strategy-replace",
                t!("settings.destinations.collision_strategy_replace").to_string(),
            )
            .selected(this.settings.collision_strategy == CollisionStrategy::Replace)
            .min_width(88.0)
            .on_click(move |_, _, app| {
                entity.update(app, |this, cx| {
                    this.set_collision_strategy(CollisionStrategy::Replace, cx);
                });
            }),
        )
}

fn render_destination_rules_switch(
    this: &SettingsWindow,
    cx: &mut Context<SettingsWindow>,
) -> impl IntoElement {
    let entity = cx.entity();

    Switch::new("destination-rules-enabled")
        .checked(this.settings.destination_rules_enabled)
        .on_click(move |checked, _, app| {
            entity.update(app, |this, cx| {
                this.set_destination_rules_enabled(checked, cx)
            });
        })
}

fn render_http_download_ordering_mode(
    this: &SettingsWindow,
    cx: &mut Context<SettingsWindow>,
) -> impl IntoElement {
    let entity = cx.entity();

    SegmentedControl::new("http-download-ordering-mode")
        .option(
            SegmentedControlOption::new(
                "http-download-ordering-balanced",
                t!("settings.destinations.http_download_ordering_mode_balanced").to_string(),
            )
            .selected(
                this.settings.http_download_ordering_mode == HttpDownloadOrderingMode::Balanced,
            )
            .min_width(98.0)
            .on_click({
                let entity = entity.clone();
                move |_, _, app| {
                    entity.update(app, |this, cx| {
                        this.set_http_download_ordering_mode(
                            HttpDownloadOrderingMode::Balanced,
                            cx,
                        );
                    });
                }
            }),
        )
        .option(
            SegmentedControlOption::new(
                "http-download-ordering-file-specific",
                t!("settings.destinations.http_download_ordering_mode_file_specific").to_string(),
            )
            .selected(
                this.settings.http_download_ordering_mode == HttpDownloadOrderingMode::FileSpecific,
            )
            .min_width(122.0)
            .on_click({
                let entity = entity.clone();
                move |_, _, app| {
                    entity.update(app, |this, cx| {
                        this.set_http_download_ordering_mode(
                            HttpDownloadOrderingMode::FileSpecific,
                            cx,
                        );
                    });
                }
            }),
        )
        .option(
            SegmentedControlOption::new(
                "http-download-ordering-sequential",
                t!("settings.destinations.http_download_ordering_mode_sequential").to_string(),
            )
            .selected(
                this.settings.http_download_ordering_mode == HttpDownloadOrderingMode::Sequential,
            )
            .min_width(108.0)
            .on_click(move |_, _, app| {
                entity.update(app, |this, cx| {
                    this.set_http_download_ordering_mode(HttpDownloadOrderingMode::Sequential, cx);
                });
            }),
        )
}

fn render_sequential_download_extensions(this: &SettingsWindow) -> impl IntoElement {
    let is_file_specific =
        this.settings.http_download_ordering_mode == HttpDownloadOrderingMode::FileSpecific;

    div()
        .when(!is_file_specific, |this| this.opacity(0.6))
        .child(super::setting_text_input(
            this.sequential_download_extensions_input.clone(),
        ))
}

fn render_destination_rules_section(
    this: &SettingsWindow,
    cx: &mut Context<SettingsWindow>,
) -> impl IntoElement {
    let entity = cx.entity();
    let restore_entity = entity.clone();

    div()
        .flex_col()
        .gap(px(Spacing::SETTINGS_PANEL_GAP))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(Spacing::SECTION_GAP))
                .child(
                    div()
                        .flex_1()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::LIGHT)
                                .text_color(Colors::foreground())
                                .child(
                                    t!("settings.destinations.destination_rules_title").to_string(),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(Colors::muted_foreground())
                                .child(
                                    t!("settings.destinations.destination_rules_description")
                                        .to_string(),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(Spacing::SETTINGS_INLINE_GAP))
                        .child(
                            Button::new(
                                "restore-default-destination-rules",
                                t!("settings.destinations.destination_rules_restore_defaults")
                                    .to_string(),
                            )
                            .on_click(move |_, _, app| {
                                restore_entity.update(app, |this, cx| {
                                    this.restore_default_destination_rules(cx)
                                });
                            }),
                        ),
                ),
        )
        .child(
            div()
                .flex_col()
                .gap(px(Spacing::SETTINGS_PANEL_GAP))
                .p(px(Chrome::SETTINGS_SECTION_PANEL_PADDING))
                .rounded(px(Chrome::PANEL_RADIUS))
                .border_1()
                .border_color(if this.settings.destination_rules_enabled {
                    Colors::border()
                } else {
                    Colors::input_border()
                })
                .when(this.destination_rule_editors.is_empty(), |this| {
                    this.child(
                        div()
                            .py(px(Chrome::MENU_ITEM_PADDING_Y))
                            .text_sm()
                            .text_color(Colors::muted_foreground())
                            .child(t!("settings.destinations.destination_rules_empty").to_string()),
                    )
                })
                .when(!this.settings.destination_rules_enabled, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(Colors::muted_foreground())
                            .child(
                                t!("settings.destinations.destination_rules_disabled_hint")
                                    .to_string(),
                            ),
                    )
                })
                .children(
                    this.destination_rule_editors
                        .iter()
                        .enumerate()
                        .map(|(index, rule)| {
                            render_destination_rule_row(this, index, index == 0, rule, cx)
                                .into_any_element()
                        }),
                ),
        )
}

fn render_destination_rule_row(
    _this: &SettingsWindow,
    index: usize,
    is_first: bool,
    rule: &DestinationRuleEditor,
    cx: &mut Context<SettingsWindow>,
) -> impl IntoElement {
    debug_destination_rule_row_render();

    let entity = cx.entity();
    let toggle_entity = entity.clone();
    let icon_name = destination_rule_icon_name(rule, cx);
    let label = destination_rule_label(rule, cx);
    let extensions = destination_rule_extensions_preview(rule, cx);

    div()
        .flex_col()
        .gap(px(Spacing::SETTINGS_INLINE_GAP))
        .py(px(Chrome::SETTINGS_RULE_CARD_PADDING))
        .when(!is_first, |this| {
            this.border_t_1().border_color(Colors::border())
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(Spacing::SETTINGS_INLINE_GAP))
                .child(
                    div()
                        .size(px(Chrome::SETTINGS_ICON_TRIGGER_SIZE))
                        .rounded(px(Chrome::CARD_RADIUS))
                        .border_1()
                        .border_color(Colors::input_border())
                        .bg(Colors::background())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(file_type_icon_sm(&icon_name, Colors::foreground())),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex_col()
                        .gap(px(Spacing::SETTINGS_LABEL_GAP))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::LIGHT)
                                .text_color(Colors::foreground())
                                .truncate()
                                .child(label),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(Colors::muted_foreground())
                                .truncate()
                                .child(extensions),
                        ),
                )
                .child(
                    Switch::new(format!("destination-rule-enabled-{}", rule.id))
                        .checked(rule.enabled)
                        .on_click(move |checked, _, app| {
                            toggle_entity.update(app, |this, cx| {
                                this.set_destination_rule_enabled(index, checked, cx)
                            });
                        }),
                ),
        )
        .child(rule.target_dir_input.clone())
}

fn destination_rule_icon_name(
    rule: &DestinationRuleEditor,
    cx: &Context<SettingsWindow>,
) -> String {
    if let Some(icon_name) = rule.icon_name.as_ref() {
        return icon_name.clone();
    }

    let label = rule.label_input.read(cx).text().to_string();
    let extensions = super::parse_extensions_input(rule.extensions_input.read(cx).text());
    suggested_destination_rule_icon_name(&label, &extensions).to_string()
}

fn destination_rule_label(rule: &DestinationRuleEditor, cx: &Context<SettingsWindow>) -> String {
    let label = rule.label_input.read(cx).text().trim().to_string();
    if label.is_empty() {
        "Custom Rule".to_string()
    } else {
        label
    }
}

fn destination_rule_extensions_preview(
    rule: &DestinationRuleEditor,
    cx: &Context<SettingsWindow>,
) -> String {
    let extensions = rule.extensions_input.read(cx).text().trim().to_string();
    if extensions.is_empty() {
        "Extensions are editable in settings.json".to_string()
    } else {
        extensions
    }
}

#[cfg(debug_assertions)]
fn debug_destinations_render(rule_count: usize) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    static COUNT: AtomicUsize = AtomicUsize::new(0);

    if *ENABLED.get_or_init(|| std::env::var_os("OPHELIA_DEBUG_DESTINATIONS_RENDER").is_some()) {
        let count = COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if count <= 10 || count.is_multiple_of(25) {
            eprintln!("[settings/destinations] render #{count} (rules={rule_count})");
        }
    }
}

#[cfg(not(debug_assertions))]
fn debug_destinations_render(_rule_count: usize) {}

#[cfg(debug_assertions)]
fn debug_destination_rule_row_render() {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    static COUNT: AtomicUsize = AtomicUsize::new(0);

    if *ENABLED.get_or_init(|| std::env::var_os("OPHELIA_DEBUG_DESTINATIONS_RENDER").is_some()) {
        let count = COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if count <= 20 || count.is_multiple_of(50) {
            eprintln!("[settings/destination-rule-row] render #{count}");
        }
    }
}

#[cfg(not(debug_assertions))]
fn debug_destination_rule_row_render() {}
