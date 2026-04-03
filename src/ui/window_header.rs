use gpui::{
    AnyElement, App, Context, IntoElement, MouseButton, Render, RenderOnce, SharedString, Window,
    WindowControlArea, div, prelude::*, px,
};

use crate::platform;
use crate::ui::prelude::*;

#[derive(IntoElement)]
pub struct WindowHeader {
    title: Option<SharedString>,
    trailing: Option<AnyElement>,
}

impl WindowHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: Some(title.into()),
            trailing: None,
        }
    }

    pub fn empty() -> Self {
        Self {
            title: None,
            trailing: None,
        }
    }

    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
        self
    }
}

struct WindowHeaderDragState {
    should_move: bool,
}

impl Render for WindowHeaderDragState {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl RenderOnce for WindowHeader {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let chrome = platform::window_chrome();
        let title = self.title.take().filter(|title| !title.is_empty());
        let drag_state = window.use_state(cx, |_, _| WindowHeaderDragState { should_move: false });

        div().flex_shrink_0().child(
            div()
                .id("window-header")
                .flex()
                .items_center()
                .h(px(chrome.height))
                .border_b_1()
                .border_color(Colors::border())
                .bg(Colors::background())
                .on_mouse_down_out(window.listener_for(&drag_state, |state, _, _, _| {
                    state.should_move = false;
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    window.listener_for(&drag_state, |state, _, _, _| {
                        state.should_move = true;
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    window.listener_for(&drag_state, |state, _, _, _| {
                        state.should_move = false;
                    }),
                )
                .on_mouse_move(window.listener_for(&drag_state, |state, _, window, _| {
                    if state.should_move {
                        state.should_move = false;
                        window.start_window_move();
                    }
                }))
                .child(
                    h_flex()
                        .h_full()
                        .w_full()
                        .items_center()
                        .gap(px(12.0))
                        .pl(px(chrome.leading_padding))
                        .pr(px(chrome.horizontal_padding))
                        .child(
                            div()
                                .flex_1()
                                .h_full()
                                .flex()
                                .items_center()
                                .window_control_area(WindowControlArea::Drag)
                                .when_some(title, |this, title| {
                                    this.child(
                                        div()
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(Colors::muted_foreground())
                                            .child(title),
                                    )
                                }),
                        )
                        .when_some(self.trailing.take(), |this, trailing| this.child(trailing)),
                ),
        )
    }
}
