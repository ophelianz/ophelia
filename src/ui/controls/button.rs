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

//! Button control adapted from the `Button` component in
//! [longbridge/gpui-component](https://github.com/longbridge/gpui-component),
//! which is Apache-2.0 licensed. Ophelia keeps a focused local adaptation for
//! its own UI instead of depending on the full component library.

use std::rc::Rc;

use gpui::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, prelude::*, px,
};

use crate::ui::prelude::*;

#[derive(Clone, Copy)]
pub enum ButtonVariant {
    Secondary,
    Primary,
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    compact: bool,
    icon: Option<IconName>,
    variant: ButtonVariant,
    disabled: bool,
    on_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            compact: false,
            icon: None,
            variant: ButtonVariant::Secondary,
            disabled: false,
            on_click: None,
        }
    }

    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    pub fn primary(mut self) -> Self {
        self.variant = ButtonVariant::Primary;
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (bg, border_color, text_color) = match (self.variant, self.disabled) {
            (ButtonVariant::Primary, false) => (
                Colors::active(),
                gpui::rgba(0x00000000),
                Colors::background(),
            ),
            (ButtonVariant::Primary, true) => (
                Colors::muted(),
                gpui::rgba(0x00000000),
                Colors::muted_foreground(),
            ),
            (ButtonVariant::Secondary, false) => (
                Colors::muted(),
                Colors::input_border(),
                Colors::foreground(),
            ),
            (ButtonVariant::Secondary, true) => (
                Colors::muted(),
                Colors::input_border(),
                Colors::muted_foreground(),
            ),
        };

        div()
            .id(self.id)
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .gap(px(Spacing::SETTINGS_INLINE_GAP))
            .h(px(Chrome::BUTTON_HEIGHT))
            .px(px(if self.compact {
                Chrome::BUTTON_COMPACT_PADDING_X
            } else {
                Chrome::BUTTON_PADDING_X
            }))
            .rounded(px(Chrome::BUTTON_RADIUS))
            .border_1()
            .border_color(border_color)
            .bg(bg)
            .text_sm()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(text_color)
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(Colors::card_hover()))
                    .active(|style| style.bg(Colors::card()))
            })
            .when_some(self.icon, |this, icon| {
                this.child(icon_sm(icon, text_color))
            })
            .child(div().child(self.label))
            .when_some(
                self.on_click.filter(|_| !self.disabled),
                |this, on_click| {
                    this.on_click(move |event, window, cx| on_click(event, window, cx))
                },
            )
    }
}
