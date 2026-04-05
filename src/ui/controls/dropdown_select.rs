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

//! Dropdown select control adapted from the `Select` component in
//! [longbridge/gpui-component](https://github.com/longbridge/gpui-component),
//! which is Apache-2.0 licensed. Ophelia keeps a local adaptation for a small,
//! settings-oriented dropdown without depending on the full component library.

use gpui::{
    Context, Corner, ElementId, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, anchored, deferred, div, point, prelude::*, px,
};

use crate::ui::prelude::*;

#[derive(Clone)]
pub struct DropdownOption {
    value: SharedString,
    label: SharedString,
}

impl DropdownOption {
    pub fn new(value: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }
}

pub struct DropdownSelect {
    id: ElementId,
    options: Vec<DropdownOption>,
    selected_value: SharedString,
    open: bool,
}

pub struct DropdownSelectChanged;

impl gpui::EventEmitter<DropdownSelectChanged> for DropdownSelect {}

impl DropdownSelect {
    pub fn new(
        id: impl Into<ElementId>,
        options: impl IntoIterator<Item = DropdownOption>,
        selected_value: impl Into<SharedString>,
        _cx: &mut Context<Self>,
    ) -> Self {
        let options = options.into_iter().collect::<Vec<_>>();
        let selected_value = selected_value.into();
        let selected_value = options
            .iter()
            .find(|option| option.value == selected_value)
            .map(|option| option.value.clone())
            .or_else(|| options.first().map(|option| option.value.clone()))
            .unwrap_or(selected_value);

        Self {
            id: id.into(),
            options,
            selected_value,
            open: false,
        }
    }

    pub fn selected_value(&self) -> &str {
        self.selected_value.as_ref()
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.open = !self.open;
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            cx.notify();
        }
    }

    fn select(&mut self, value: SharedString, cx: &mut Context<Self>) {
        let changed = self.selected_value != value;
        self.selected_value = value;
        self.open = false;
        if changed {
            cx.emit(DropdownSelectChanged);
        }
        cx.notify();
    }

    fn selected_label(&self) -> SharedString {
        self.options
            .iter()
            .find(|option| option.value == self.selected_value)
            .map(|option| option.label.clone())
            .unwrap_or_else(|| self.selected_value.clone())
    }
}

impl Render for DropdownSelect {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_open = self.open;
        let trigger = div()
            .id(self.id.clone())
            .w_full()
            .min_w(px(220.0))
            .h(px(40.0))
            .px(px(12.0))
            .rounded(px(Chrome::BUTTON_RADIUS))
            .border_1()
            .border_color(if is_open {
                Colors::ring()
            } else {
                Colors::input_border()
            })
            .bg(Colors::background())
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(Spacing::CONTROL_GAP))
            .hover(|style| style.bg(Colors::muted()))
            .on_click(cx.listener(|this, _, _, cx| {
                this.toggle(cx);
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(Colors::foreground())
                    .truncate()
                    .child(self.selected_label()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(Colors::muted_foreground())
                    .child(if is_open { "▴" } else { "▾" }),
            );

        let popup = if self.open {
            Some(
                anchored()
                    .anchor(Corner::TopLeft)
                    .offset(point(px(0.0), px(Spacing::CONTROL_GAP)))
                    .child(deferred(render_popup(
                        cx.entity(),
                        self.options.clone(),
                        self.selected_value.clone(),
                        cx,
                    ))),
            )
        } else {
            None
        };

        div().relative().w_full().child(trigger).children(popup)
    }
}

fn render_popup(
    entity: Entity<DropdownSelect>,
    options: Vec<DropdownOption>,
    selected_value: SharedString,
    cx: &mut Context<DropdownSelect>,
) -> impl IntoElement {
    div()
        .id("dropdown-select-popup")
        .occlude()
        .w_full()
        .min_w(px(220.0))
        .p(px(Chrome::MENU_POPUP_PADDING))
        .rounded(px(Chrome::CARD_RADIUS))
        .border_1()
        .border_color(Colors::border())
        .bg(Colors::card())
        .shadow_lg()
        .flex()
        .flex_col()
        .gap(px(Chrome::MENU_POPUP_GAP))
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.close(cx);
        }))
        .children(options.into_iter().enumerate().map(|(index, option)| {
            let is_selected = option.value == selected_value;
            let entity = entity.clone();
            let value = option.value.clone();

            div()
                .id(("dropdown-select-option", index))
                .flex()
                .items_center()
                .gap(px(Spacing::CONTROL_GAP))
                .px(px(Chrome::MENU_ITEM_PADDING_X))
                .py(px(Chrome::MENU_ITEM_PADDING_Y))
                .rounded(px(Chrome::BUTTON_RADIUS))
                .cursor_pointer()
                .text_sm()
                .text_color(Colors::foreground())
                .bg(if is_selected {
                    Colors::muted()
                } else {
                    Colors::card()
                })
                .hover(|style| style.bg(Colors::muted()))
                .on_click(move |_, _, app| {
                    let value = value.clone();
                    let _ = entity.update(app, |this, cx| this.select(value.clone(), cx));
                })
                .child(
                    div()
                        .w(px(Chrome::MENU_ITEM_CHECK_WIDTH))
                        .text_xs()
                        .text_color(Colors::active())
                        .child(if is_selected { "✓" } else { "" }),
                )
                .child(div().flex_1().min_w_0().truncate().child(option.label))
                .into_any_element()
        }))
}
