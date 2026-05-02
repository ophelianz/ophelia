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
    App, Context, IntoElement, Render, SharedString, Window, div, linear_color_stop,
    linear_gradient, prelude::*, px, rgba,
};

use crate::ui::prelude::*;

const TOAST_TITLE_LINE_HEIGHT: f32 = 21.0;
const TOAST_DETAIL_LINE_HEIGHT: f32 = 25.0;

type ToastActionHandler = dyn Fn(&mut Window, &mut App);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    #[allow(dead_code)]
    // the primitive supports neutral toasts before the first production caller
    Info,
    Warning,
    Error,
}

#[derive(Clone)]
pub struct ToastAction {
    label: SharedString,
    handler: Rc<ToastActionHandler>,
}

impl ToastAction {
    fn new(
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            handler: Rc::new(handler),
        }
    }
}

#[derive(Clone)]
pub struct Toast {
    kind: ToastKind,
    title: SharedString,
    detail: Option<SharedString>,
    action: Option<ToastAction>,
}

impl Toast {
    #[allow(dead_code)] // the primitive supports neutral toasts before the first production caller
    pub fn info(title: impl Into<SharedString>) -> Self {
        Self::new(ToastKind::Info, title)
    }

    pub fn warning(title: impl Into<SharedString>) -> Self {
        Self::new(ToastKind::Warning, title)
    }

    #[allow(dead_code)] // available for future service and updater failures
    pub fn error(title: impl Into<SharedString>) -> Self {
        Self::new(ToastKind::Error, title)
    }

    pub fn new(kind: ToastKind, title: impl Into<SharedString>) -> Self {
        Self {
            kind,
            title: title.into(),
            detail: None,
            action: None,
        }
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    #[allow(dead_code)] // the primitive supports it before service handoff UI needs it
    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.action = Some(ToastAction::new(label, handler));
        self
    }

    #[cfg(test)]
    fn title(&self) -> &str {
        self.title.as_ref()
    }

    #[cfg(test)]
    fn detail_text(&self) -> Option<&str> {
        self.detail.as_ref().map(|detail| detail.as_ref())
    }
}

#[derive(Clone)]
pub struct ToastLayer {
    active: Option<Toast>,
}

impl ToastLayer {
    pub fn new() -> Self {
        Self { active: None }
    }

    pub fn show(&mut self, toast: Toast, cx: &mut Context<Self>) {
        self.active = Some(toast);
        cx.notify();
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.active = None;
        cx.notify();
    }

    #[cfg(test)]
    fn active_toast(&self) -> Option<&Toast> {
        self.active.as_ref()
    }
}

impl Render for ToastLayer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(toast) = self.active.clone() else {
            return div().into_any_element();
        };

        let layer = cx.entity();
        div()
            .absolute()
            .size_full()
            .bottom_0()
            .left_0()
            .child(
                div()
                    .absolute()
                    .w_full()
                    .bottom(px(24.0))
                    .left_0()
                    .flex()
                    .justify_center()
                    .child(render_toast(toast, layer)),
            )
            .into_any_element()
    }
}

fn render_toast(toast: Toast, layer: gpui::Entity<ToastLayer>) -> impl IntoElement {
    let style = toast_style(toast.kind);
    let action = toast.action.clone();

    h_flex()
        .id("toast")
        .occlude()
        .max_w(px(560.0))
        .min_w(px(360.0))
        .mx(px(24.0))
        .px(px(16.0))
        .py(px(14.0))
        .gap(px(14.0))
        .items_start()
        .rounded(px(Chrome::CARD_RADIUS))
        .border_1()
        .border_color(style.border)
        .bg(style.background)
        .shadow_lg()
        .child(IconBox::medium(style.icon, style.accent))
        .child(
            v_flex()
                .min_w_0()
                .flex_1()
                .gap(px(3.0))
                .child(
                    div()
                        .text_sm()
                        .line_height(px(TOAST_TITLE_LINE_HEIGHT))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(Colors::foreground())
                        .child(toast.title),
                )
                .when_some(toast.detail, |this, detail| {
                    this.child(
                        div()
                            .text_xs()
                            .line_height(px(TOAST_DETAIL_LINE_HEIGHT))
                            .text_color(Colors::muted_foreground())
                            .child(detail),
                    )
                }),
        )
        .when_some(action, |this, action| {
            let handler = action.handler.clone();
            let layer = layer.clone();
            this.child(
                Button::new("toast-action", action.label)
                    .compact()
                    .on_click(move |_, window, cx| {
                        handler(window, cx);
                        layer.update(cx, |layer, cx| {
                            layer.dismiss(cx);
                        });
                    }),
            )
        })
        .child(
            IconButton::new("toast-dismiss", IconName::X)
                .stop_propagation()
                .on_click(move |_, _window, cx| {
                    layer.update(cx, |layer, cx| {
                        layer.dismiss(cx);
                    });
                }),
        )
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ToastStyle {
    icon: IconName,
    accent: gpui::Rgba,
    background: gpui::Background,
    border: gpui::Rgba,
}

fn toast_style(kind: ToastKind) -> ToastStyle {
    match kind {
        ToastKind::Info => ToastStyle {
            icon: IconName::Info,
            accent: Colors::finished(),
            background: Colors::card().into(),
            border: Colors::border(),
        },
        ToastKind::Warning => ToastStyle {
            icon: IconName::TriangleAlert,
            accent: Colors::warning(),
            background: red_toast_background(),
            border: rgba(0xe5634d4a),
        },
        ToastKind::Error => ToastStyle {
            icon: IconName::CircleX,
            accent: Colors::error(),
            background: red_toast_background(),
            border: rgba(0xe5634d5c),
        },
    }
}

fn red_toast_background() -> gpui::Background {
    linear_gradient(
        180.0,
        linear_color_stop(rgba(0x2d1113f4), 0.0),
        linear_color_stop(rgba(0x240e10f7), 1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_builders_keep_short_title_and_optional_detail() {
        let toast = Toast::warning("Ophelia is using another service")
            .detail("Service owner: homebrew formula. Binary: /opt/homebrew/bin/ophelia-service");

        assert_eq!(toast.kind, ToastKind::Warning);
        assert_eq!(toast.title(), "Ophelia is using another service");
        assert_eq!(
            toast.detail_text(),
            Some("Service owner: homebrew formula. Binary: /opt/homebrew/bin/ophelia-service")
        );
    }

    #[test]
    fn toast_kind_maps_to_icons_and_accent_colors() {
        assert_eq!(toast_style(ToastKind::Info).icon, IconName::Info);
        assert_eq!(
            toast_style(ToastKind::Warning).icon,
            IconName::TriangleAlert
        );
        assert_eq!(toast_style(ToastKind::Error).icon, IconName::CircleX);
    }

    #[test]
    fn toast_layer_replaces_the_active_toast() {
        let mut app = gpui::TestApp::new();

        app.update(|cx| {
            let layer = cx.new(|_| ToastLayer::new());
            layer.update(cx, |layer, cx| {
                layer.show(Toast::info("First"), cx);
                layer.show(Toast::error("Second"), cx);
            });

            let toast = layer.read(cx).active_toast().unwrap();
            assert_eq!(toast.title(), "Second");
            assert_eq!(toast.kind, ToastKind::Error);
        });
    }
}
