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

use gpui::{Context, IntoElement, ParentElement, Styled, div, prelude::*, px};
use rust_i18n::t;

use crate::settings::CollisionStrategy;
use crate::ui::prelude::*;

use super::{DestinationRuleEditor, SettingsWindow};

pub(super) fn render(this: &SettingsWindow, cx: &mut Context<SettingsWindow>) -> gpui::Div {
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
                    let _ = entity.update(app, |this, cx| {
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
                let _ = entity.update(app, |this, cx| {
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
            let _ = entity.update(app, |this, cx| {
                this.set_destination_rules_enabled(checked, cx)
            });
        })
}

fn render_destination_rules_section(
    this: &SettingsWindow,
    cx: &mut Context<SettingsWindow>,
) -> impl IntoElement {
    let entity = cx.entity();
    let restore_entity = entity.clone();
    let add_entity = entity.clone();

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
                                .font_weight(gpui::FontWeight::SEMIBOLD)
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
                                let _ = restore_entity.update(app, |this, cx| {
                                    this.restore_default_destination_rules(cx)
                                });
                            }),
                        )
                        .child(
                            Button::new(
                                "add-destination-rule",
                                t!("settings.destinations.destination_rules_add").to_string(),
                            )
                            .icon(IconName::Plus)
                            .on_click(move |_, _, app| {
                                let _ = add_entity
                                    .update(app, |this, cx| this.add_destination_rule(cx));
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
                .border_color(Colors::border())
                .bg(if this.settings.destination_rules_enabled {
                    Colors::card()
                } else {
                    Colors::muted()
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
                            render_destination_rule_row(this, index, rule, cx).into_any_element()
                        }),
                ),
        )
}

fn render_destination_rule_row(
    this: &SettingsWindow,
    index: usize,
    rule: &DestinationRuleEditor,
    cx: &mut Context<SettingsWindow>,
) -> impl IntoElement {
    let entity = cx.entity();
    let toggle_entity = entity.clone();
    let remove_entity = entity.clone();

    div()
        .flex_col()
        .gap(px(Spacing::SETTINGS_PANEL_GAP))
        .p(px(Chrome::SETTINGS_RULE_CARD_PADDING))
        .rounded(px(Chrome::CARD_RADIUS))
        .border_1()
        .border_color(if rule.enabled {
            Colors::border()
        } else {
            Colors::input_border()
        })
        .bg(Colors::background())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(Spacing::SETTINGS_PANEL_GAP))
                .child(super::destination_rule_icon_picker::render(
                    this, index, rule, cx,
                ))
                .child(div().w(px(148.0)).child(rule.label_input.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(rule.extensions_input.clone()),
                )
                .child(
                    Switch::new(format!("destination-rule-enabled-{}", rule.id))
                        .checked(rule.enabled)
                        .on_click(move |checked, _, app| {
                            let _ = toggle_entity.update(app, |this, cx| {
                                this.set_destination_rule_enabled(index, checked, cx)
                            });
                        }),
                )
                .child(
                    div()
                        .id(format!("remove-destination-rule-{index}"))
                        .size(px(32.0))
                        .rounded(px(8.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(Colors::muted()))
                        .on_click(move |_, _, app| {
                            let _ = remove_entity
                                .update(app, |this, cx| this.remove_destination_rule(index, cx));
                        })
                        .child(icon_sm(IconName::Trash2, Colors::muted_foreground())),
                ),
        )
        .child(rule.target_dir_input.clone())
}
