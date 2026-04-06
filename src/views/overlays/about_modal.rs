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
    App, Context, Entity, FontWeight, IntoElement, Render, RenderOnce, Window, div, prelude::*, px,
};
use rust_i18n::t;

use crate::ui::prelude::*;

type OnExitHandler = dyn Fn(&mut Window, &mut App);

pub struct AboutLayer {
    show: Entity<bool>,
}

impl AboutLayer {
    pub fn new(show: Entity<bool>, cx: &mut Context<Self>) -> Self {
        cx.observe(&show, |_, _, cx| {
            cx.notify();
        })
        .detach();

        Self { show }
    }

    fn hide(&mut self, cx: &mut Context<Self>) {
        self.show.update(cx, |show, cx| {
            *show = false;
            cx.notify();
        });
    }
}

impl Render for AboutLayer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if *self.show.read(cx) {
            let weak = cx.weak_entity();
            div()
                .child(AboutModal::new(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.hide(cx);
                    });
                }))
                .into_any_element()
        } else {
            div().into_any_element()
        }
    }
}

#[derive(IntoElement)]
pub struct AboutModal {
    on_exit: Rc<OnExitHandler>,
}

impl AboutModal {
    fn new(on_exit: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            on_exit: Rc::new(on_exit),
        }
    }
}

impl RenderOnce for AboutModal {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_exit = Rc::clone(&self.on_exit);
        let on_exit_button = Rc::clone(&self.on_exit);

        modal()
            .on_exit(move |window, cx| {
                on_exit(window, cx);
            })
            .child(
                div()
                    .w(px(Chrome::ABOUT_MODAL_WIDTH))
                    .p(px(Chrome::MODAL_PADDING))
                    .flex()
                    .flex_col()
                    .gap(px(Chrome::MODAL_STACK_GAP))
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(Spacing::SECTION_GAP))
                            .child(OpheliaLogo::new(52.0))
                            .child(
                                v_flex()
                                    .gap(px(4.0))
                                    .child(
                                        div()
                                            .text_xl()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(Colors::foreground())
                                            .child(t!("app.name").to_string()),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(Colors::muted_foreground())
                                            .child(
                                                t!(
                                                    "about.version",
                                                    version = env!("CARGO_PKG_VERSION")
                                                )
                                                .to_string(),
                                            ),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .line_height(px(22.0))
                            .text_color(Colors::muted_foreground())
                            .child(t!("app.tagline").to_string()),
                    )
                    .child(
                        h_flex().justify_end().child(
                            div()
                                .id("about-close")
                                .px(px(18.0))
                                .py(px(10.0))
                                .rounded(px(Chrome::BUTTON_RADIUS))
                                .bg(Colors::active())
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(Colors::background())
                                .cursor_pointer()
                                .on_click(move |_, window, cx| {
                                    on_exit_button(window, cx);
                                })
                                .child(t!("about.close").to_string()),
                        ),
                    ),
            )
    }
}
