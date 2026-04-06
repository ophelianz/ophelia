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
    AnyElement, App, Corner, ElementId, InteractiveElement, IntoElement, ParentElement, Pixels,
    Point, RenderOnce, Styled, Window, anchored, deferred, div, point, px,
};

use crate::ui::prelude::*;

type PopupCloseHandler = dyn Fn(&mut Window, &mut App);

#[derive(IntoElement)]
pub struct PopupSurface {
    id: ElementId,
    width: Option<Pixels>,
    min_width: Option<Pixels>,
    match_trigger_width: bool,
    offset: Point<Pixels>,
    on_close: Option<Rc<PopupCloseHandler>>,
    children: Vec<AnyElement>,
}

impl PopupSurface {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            width: None,
            min_width: None,
            match_trigger_width: false,
            offset: point(px(0.0), px(Spacing::CONTROL_GAP)),
            on_close: None,
            children: Vec::new(),
        }
    }

    pub fn width(mut self, width: Pixels) -> Self {
        self.width = Some(width);
        self
    }

    pub fn min_width(mut self, min_width: Pixels) -> Self {
        self.min_width = Some(min_width);
        self
    }

    pub fn match_trigger_width(mut self) -> Self {
        self.match_trigger_width = true;
        self
    }

    #[allow(dead_code)] // callers currently use the default offset, but custom popups will need this hook
    pub fn offset(mut self, offset: Point<Pixels>) -> Self {
        self.offset = offset;
        self
    }

    pub fn on_close(mut self, on_close: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(on_close));
        self
    }
}

impl ParentElement for PopupSurface {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for PopupSurface {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let panel = div()
            .id(self.id)
            .occlude()
            .p(px(Chrome::MENU_POPUP_PADDING))
            .rounded(px(Chrome::CARD_RADIUS))
            .border_1()
            .border_color(Colors::border())
            .bg(Colors::card())
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(Chrome::MENU_POPUP_GAP));

        let panel = if self.match_trigger_width {
            panel.w_full()
        } else {
            panel
        };

        let panel = if let Some(width) = self.width {
            panel.w(width)
        } else {
            panel
        };

        let panel = if let Some(min_width) = self.min_width {
            panel.min_w(min_width)
        } else {
            panel
        };

        let panel = if let Some(on_close) = self.on_close {
            panel.on_mouse_down_out(move |_, window, cx| {
                cx.stop_propagation();
                on_close(window, cx);
            })
        } else {
            panel
        };

        anchored()
            .anchor(Corner::TopLeft)
            .offset(self.offset)
            .child(deferred(panel.children(self.children)))
    }
}

pub fn popup_surface(id: impl Into<ElementId>) -> PopupSurface {
    PopupSurface::new(id)
}
