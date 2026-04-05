use gpui::{
    Context, Corner, IntoElement, ParentElement, Styled, anchored, deferred, div, point,
    prelude::*, px,
};
use rust_i18n::t;

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
        .size(px(36.0))
        .rounded(px(10.0))
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
    let auto_icon_name = auto_rule_icon_name(rule, cx).to_string();

    div()
        .id(format!("destination-rule-icon-popup-{index}"))
        .occlude()
        .w(px(280.0))
        .p(px(12.0))
        .rounded(px(12.0))
        .border_1()
        .border_color(Colors::border())
        .bg(Colors::card())
        .shadow_lg()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.close_destination_rule_icon_picker(cx);
        }))
        .child(option(
            index,
            None,
            &auto_icon_name,
            t!("settings.destinations.destination_rule_icon_auto").to_string(),
            selected_icon_name.is_none(),
            cx,
        ))
        .child(div().h(px(1.0)).bg(Colors::border()))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.0))
                .children(FILE_TYPE_ICON_NAMES.iter().map(|icon_name| {
                    option(
                        index,
                        Some((*icon_name).to_string()),
                        icon_name,
                        file_type_icon_label(icon_name),
                        selected_icon_name == Some(*icon_name),
                        cx,
                    )
                    .into_any_element()
                })),
        )
}

fn option(
    index: usize,
    icon_override: Option<String>,
    icon_name: &str,
    label: String,
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
        .w(px(58.0))
        .h(px(58.0))
        .p(px(6.0))
        .rounded(px(10.0))
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
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(4.0))
        .text_xs()
        .text_color(if selected {
            Colors::foreground()
        } else {
            Colors::muted_foreground()
        })
        .on_click(move |_, _, app| {
            let icon_override = icon_override.clone();
            let _ = entity.update(app, |this, cx| {
                this.set_destination_rule_icon(index, icon_override, cx)
            });
        })
        .child(file_type_icon_sm(icon_name, Colors::foreground()))
        .child(div().w_full().text_center().truncate().child(label))
}

fn auto_rule_icon_name(rule: &DestinationRuleEditor, cx: &Context<SettingsWindow>) -> &'static str {
    let label = rule.label_input.read(cx).text().to_string();
    let extensions = super::parse_extensions_input(rule.extensions_input.read(cx).text());
    suggested_destination_rule_icon_name(&label, &extensions)
}

fn file_type_icon_label(icon_name: &str) -> String {
    match icon_name {
        "archive" => t!("settings.destinations.destination_rule_icon_option_archive").to_string(),
        "audio" => t!("settings.destinations.destination_rule_icon_option_audio").to_string(),
        "book" => t!("settings.destinations.destination_rule_icon_option_book").to_string(),
        "code" => t!("settings.destinations.destination_rule_icon_option_code").to_string(),
        "default" => t!("settings.destinations.destination_rule_icon_option_default").to_string(),
        "document" => t!("settings.destinations.destination_rule_icon_option_document").to_string(),
        "executable" => {
            t!("settings.destinations.destination_rule_icon_option_executable").to_string()
        }
        "font" => t!("settings.destinations.destination_rule_icon_option_font").to_string(),
        "image" => t!("settings.destinations.destination_rule_icon_option_image").to_string(),
        "key" => t!("settings.destinations.destination_rule_icon_option_key").to_string(),
        "mail" => t!("settings.destinations.destination_rule_icon_option_mail").to_string(),
        "presentation" => {
            t!("settings.destinations.destination_rule_icon_option_presentation").to_string()
        }
        "spreadsheet" => {
            t!("settings.destinations.destination_rule_icon_option_spreadsheet").to_string()
        }
        "vector" => t!("settings.destinations.destination_rule_icon_option_vector").to_string(),
        "video" => t!("settings.destinations.destination_rule_icon_option_video").to_string(),
        "web" => t!("settings.destinations.destination_rule_icon_option_web").to_string(),
        _ => t!("settings.destinations.destination_rule_icon_option_default").to_string(),
    }
}
