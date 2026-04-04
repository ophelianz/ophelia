use gpui::{
    AnyElement, App, Context, InteractiveElement as _, IntoElement, MouseButton, Render,
    RenderOnce, SharedString, Window, WindowControlArea, div, prelude::*, px,
};

use crate::platform;
use crate::ui::prelude::*;

#[derive(IntoElement)]
pub struct WindowHeader {
    title: Option<SharedString>,
    leading: Option<AnyElement>,
    show_window_controls: bool,
}

impl WindowHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: Some(title.into()),
            leading: None,
            show_window_controls: !cfg!(target_os = "macos"),
        }
    }

    pub fn empty() -> Self {
        Self {
            title: None,
            leading: None,
            show_window_controls: !cfg!(target_os = "macos"),
        }
    }

    pub fn leading(mut self, element: impl IntoElement) -> Self {
        self.leading = Some(element.into_any_element());
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
                .child(
                    h_flex()
                        .h_full()
                        .w_full()
                        .items_center()
                        .gap(px(Chrome::HEADER_GAP))
                        .pl(px(chrome.leading_padding))
                        .pr(px(chrome.horizontal_padding))
                        .when_some(self.leading.take(), |this, leading| this.child(leading))
                        .child(
                            div()
                                .flex_1()
                                .h_full()
                                .flex()
                                .items_center()
                                .on_mouse_down_out(window.listener_for(
                                    &drag_state,
                                    |state, _, _, _| {
                                        state.should_move = false;
                                    },
                                ))
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
                                .on_mouse_move(window.listener_for(
                                    &drag_state,
                                    |state, _, window, _| {
                                        if state.should_move {
                                            state.should_move = false;
                                            window.start_window_move();
                                        }
                                    },
                                ))
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
                        .when(self.show_window_controls, |this| {
                            this.child(window_controls())
                        }),
                ),
        )
    }
}

fn window_controls() -> impl IntoElement {
    h_flex()
        .items_center()
        .gap(px(Chrome::MENU_BAR_GAP))
        .child(window_control_button(0, "—", false, |window| {
            window.minimize_window();
        }))
        .child(window_control_button(1, "□", false, |window| {
            window.zoom_window();
        }))
        .child(window_control_button(2, "×", true, |window| {
            window.remove_window();
        }))
}

fn window_control_button(
    id: usize,
    label: &'static str,
    destructive: bool,
    on_click: impl Fn(&mut Window) + 'static,
) -> impl IntoElement {
    div()
        .id(("window-control", id))
        .flex()
        .items_center()
        .justify_center()
        .w(px(Chrome::WINDOW_CONTROL_WIDTH))
        .h(px(Chrome::WINDOW_CONTROL_HEIGHT))
        .rounded(px(Chrome::CONTROL_RADIUS))
        .text_sm()
        .text_color(Colors::muted_foreground())
        .cursor_pointer()
        .hover(move |style| {
            if destructive {
                style.bg(Colors::error()).text_color(Colors::background())
            } else {
                style.bg(Colors::muted()).text_color(Colors::foreground())
            }
        })
        .on_click(move |_, window, _| on_click(window))
        .child(label)
}
