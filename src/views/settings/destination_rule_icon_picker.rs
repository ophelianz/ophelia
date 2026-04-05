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

use gpui::{
    Context, Corner, IntoElement, ParentElement, Styled, anchored, deferred, div, point,
    prelude::*, px,
};

use crate::settings::suggested_destination_rule_icon_name;
use crate::ui::prelude::*;

use super::{DestinationRuleEditor, SettingsWindow};

pub(super) fn render(
    this: &SettingsWindow,
    index: usize,
    rule: &DestinationRuleEditor,
    cx: &mut Context<SettingsWindow>,
) -> impl IntoElement {
    let entity = cx.entity();
    let auto_icon_name = auto_rule_icon_name(rule, cx);
    let selected_icon_name = rule.icon_name.as_deref();
    let preview_icon_name = selected_icon_name.unwrap_or(auto_icon_name);
    let is_open = this.open_icon_picker_rule == Some(index);

    let button = div()
        .id(format!("destination-rule-icon-trigger-{index}"))
        .size(px(Chrome::SETTINGS_ICON_TRIGGER_SIZE))
        .rounded(px(Chrome::CARD_RADIUS))
        .border_1()
        .border_color(if is_open {
            Colors::ring()
        } else {
            Colors::input_border()
        })
        .bg(if is_open {
            Colors::muted()
        } else {
            Colors::background()
        })
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|style| style.bg(Colors::muted()))
        .on_click(move |_, _, app| {
            let _ = entity.update(app, |this, cx| {
                this.toggle_destination_rule_icon_picker(index, cx)
            });
        })
        .child(file_type_icon_sm(preview_icon_name, Colors::foreground()));

    let popup = if is_open {
        Some(
            anchored()
                .anchor(Corner::TopLeft)
                .offset(point(px(0.0), px(Spacing::CONTROL_GAP)))
                .child(deferred(render_popup(index, rule, cx))),
        )
    } else {
        None
    };

    div().relative().child(button).children(popup)
}

fn render_popup(
    index: usize,
    rule: &DestinationRuleEditor,
    cx: &mut Context<SettingsWindow>,
) -> impl IntoElement {
    let selected_icon_name = rule.icon_name.as_deref();
    let auto_preview_icon_name = auto_rule_icon_name(rule, cx).to_string();

    div()
        .id(format!("destination-rule-icon-popup-{index}"))
        .occlude()
        .w(px(Chrome::SETTINGS_ICON_PICKER_WIDTH))
        .p(px(Chrome::SETTINGS_ICON_PICKER_PADDING))
        .rounded(px(Chrome::PANEL_RADIUS))
        .border_1()
        .border_color(Colors::border())
        .bg(Colors::card())
        .shadow_lg()
        .flex()
        .flex_col()
        .gap(px(Spacing::SETTINGS_PANEL_GAP))
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.close_destination_rule_icon_picker(cx);
        }))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(Spacing::SETTINGS_INLINE_GAP))
                .child(icon_option(
                    index,
                    None,
                    &auto_preview_icon_name,
                    selected_icon_name.is_none(),
                    cx,
                ))
                .children(FILE_TYPE_ICON_NAMES.iter().map(|icon_name| {
                    icon_option(
                        index,
                        Some((*icon_name).to_string()),
                        icon_name,
                        selected_icon_name == Some(*icon_name),
                        cx,
                    )
                    .into_any_element()
                })),
        )
}

fn icon_option(
    index: usize,
    icon_override: Option<String>,
    icon_name: &str,
    selected: bool,
    cx: &mut Context<SettingsWindow>,
) -> impl IntoElement {
    let entity = cx.entity();
    let button_id = match &icon_override {
        Some(icon_name) => format!("destination-rule-icon-option-{index}-{icon_name}"),
        None => format!("destination-rule-icon-option-{index}-auto"),
    };

    div()
        .id(button_id)
        .size(px(Chrome::SETTINGS_ICON_TILE_SIZE))
        .rounded(px(Chrome::CARD_RADIUS))
        .border_1()
        .border_color(if selected {
            Colors::ring()
        } else {
            Colors::input_border()
        })
        .bg(if selected {
            Colors::muted()
        } else {
            Colors::background()
        })
        .cursor_pointer()
        .hover(|style| style.bg(Colors::muted()))
        .flex()
        .items_center()
        .justify_center()
        .relative()
        .on_click(move |_, _, app| {
            let icon_override = icon_override.clone();
            let _ = entity.update(app, |this, cx| {
                this.set_destination_rule_icon(index, icon_override, cx)
            });
        })
        .child(file_type_icon_sm(icon_name, Colors::foreground()))
        .when(selected, |this| {
            this.child(
                div()
                    .absolute()
                    .top(px(3.0))
                    .right(px(4.0))
                    .text_xs()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(Colors::active())
                    .child("•"),
            )
        })
}

fn auto_rule_icon_name(rule: &DestinationRuleEditor, cx: &Context<SettingsWindow>) -> &'static str {
    let label = rule.label_input.read(cx).text().to_string();
    let extensions = super::parse_extensions_input(rule.extensions_input.read(cx).text());
    suggested_destination_rule_icon_name(&label, &extensions)
}
