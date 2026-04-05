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

use std::rc::Rc;

use gpui::{
    AnyElement, App, Div, InteractiveElement, IntoElement, KeyBinding, ParentElement, RenderOnce,
    Stateful, StatefulInteractiveElement, StyleRefinement, Styled, Window, anchored, deferred, div,
    point, px,
};
use gpui::{actions, prelude::FluentBuilder};

use crate::theme::{Chrome, Colors};

pub type OnExitHandler = dyn Fn(&mut Window, &mut App);

#[derive(IntoElement)]
pub struct Modal {
    div: Stateful<Div>,
    on_exit: Option<Rc<OnExitHandler>>,
}

impl Modal {
    fn new() -> Self {
        Self {
            div: div().id("modal-fg"),
            on_exit: None,
        }
    }

    pub fn on_exit(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_exit = Some(Rc::new(handler));
        self
    }
}

actions!(modal, [CloseModal]);

pub fn bind_actions(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseModal, None)]);
}

impl ParentElement for Modal {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.div.extend(elements);
    }
}

impl Styled for Modal {
    fn style(&mut self) -> &mut StyleRefinement {
        self.div.style()
    }
}

impl StatefulInteractiveElement for Modal {}

impl InteractiveElement for Modal {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.div.interactivity()
    }
}

impl RenderOnce for Modal {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let decorations = window.window_decorations();
        let size = window.viewport_size();
        let rounding = px(Chrome::PANEL_RADIUS);

        anchored().position(point(px(0.0), px(0.0))).child(deferred(
            div()
                .occlude()
                .flex()
                .w(size.width)
                .h(size.height)
                .bg(gpui::rgba(0x00000088))
                .id("modal-bg")
                .map(|div| match decorations {
                    gpui::Decorations::Server => div,
                    gpui::Decorations::Client { tiling } => div
                        .when(!(tiling.top || tiling.right), |div| {
                            div.rounded_tr(rounding)
                        })
                        .when(!(tiling.top || tiling.left), |div| div.rounded_tl(rounding))
                        .when(!(tiling.bottom || tiling.right), |div| {
                            div.rounded_br(rounding)
                        })
                        .when(!(tiling.bottom || tiling.left), |div| {
                            div.rounded_bl(rounding)
                        }),
                })
                .when_some(self.on_exit, |this, on_exit| {
                    let on_exit_clone = Rc::clone(&on_exit);
                    this.on_any_mouse_down(move |_, window, cx| {
                        on_exit_clone(window, cx);
                    })
                    .on_action(move |_: &CloseModal, window, cx| {
                        on_exit(window, cx);
                    })
                })
                .child(
                    self.div
                        .occlude()
                        .m_auto()
                        .border_1()
                        .border_color(Colors::border())
                        .bg(Colors::card())
                        .rounded(px(Chrome::MODAL_RADIUS))
                        .flex_col()
                        .on_any_mouse_down(|_, _, cx| {
                            cx.stop_propagation();
                        }),
                ),
        ))
    }
}

pub fn modal() -> Modal {
    Modal::new()
}
