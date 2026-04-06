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

use std::rc::Rc;

use gpui::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div, px, transparent_black,
};

use crate::ui::prelude::*;

#[derive(IntoElement)]
pub struct FilterChip {
    id: ElementId,
    label: SharedString,
    active: bool,
    on_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}

impl FilterChip {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            active,
            on_click: None,
        }
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for FilterChip {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let chip = div()
            .id(self.id)
            .px(px(12.0))
            .py(px(6.0))
            .rounded(px(Chrome::CONTROL_RADIUS))
            .text_sm()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .cursor_pointer()
            .bg(if self.active {
                Colors::muted().into()
            } else {
                transparent_black()
            })
            .text_color(if self.active {
                Colors::foreground()
            } else {
                Colors::muted_foreground()
            })
            .child(self.label);

        if let Some(on_click) = self.on_click {
            chip.on_click(move |event, window, cx| on_click(event, window, cx))
        } else {
            chip
        }
    }
}
