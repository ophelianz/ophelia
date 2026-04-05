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

//! Switch control adapted from the `Switch` component in
//! [longbridge/gpui-component](https://github.com/longbridge/gpui-component),
//! which is Apache-2.0 licensed. Ophelia keeps a local copy so it can tailor
//! behavior and styling without taking a direct dependency on that component
//! library.

use std::rc::Rc;

use gpui::{
    App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    StatefulInteractiveElement, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use crate::ui::prelude::*;

#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    checked: bool,
    on_click: Option<Rc<dyn Fn(bool, &mut Window, &mut App)>>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            checked: false,
            on_click: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn on_click<F>(mut self, on_click: F) -> Self
    where
        F: Fn(bool, &mut Window, &mut App) + 'static,
    {
        self.on_click = Some(Rc::new(on_click));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let bar_width = px(36.0);
        let bar_height = px(20.0);
        let thumb_size = px(16.0);
        let inset = px(2.0);
        let travel = bar_width - thumb_size - inset * 2.0;
        let bar_color = if self.checked {
            Colors::active()
        } else {
            Colors::muted()
        };
        let thumb_color = Colors::background();

        div()
            .id(self.id.clone())
            .w(bar_width)
            .h(bar_height)
            .rounded(bar_height)
            .border_1()
            .border_color(if self.checked {
                Colors::active()
            } else {
                Colors::input_border()
            })
            .bg(bar_color)
            .cursor_pointer()
            .child(
                div()
                    .mt(inset)
                    .ml(if self.checked { inset + travel } else { inset })
                    .size(thumb_size)
                    .rounded(thumb_size)
                    .bg(thumb_color),
            )
            .when_some(
                self.on_click.as_ref().map(Rc::clone),
                |this: gpui::Stateful<gpui::Div>, on_click| {
                    this.on_click(move |_, window, cx| on_click(!self.checked, window, cx))
                },
            )
    }
}
