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

use rust_i18n::t;
use std::time::Duration;

use gpui::{
    App, Bounds, Context, IntoElement, SharedString, Window, WindowBackgroundAppearance,
    WindowBounds, WindowKind, WindowOptions, div, point, prelude::*, px, size,
};

use crate::ui::prelude::*;

pub enum NotificationKind {
    Success,
    Error,
}

pub struct Notification {
    filename: SharedString,
    kind: NotificationKind,
}

impl Notification {
    pub fn new(filename: impl Into<SharedString>, kind: NotificationKind) -> Self {
        Self {
            filename: filename.into(),
            kind,
        }
    }
}

impl Render for Notification {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let (icon, accent) = match self.kind {
            NotificationKind::Success => (IconName::CircleCheck, Colors::active()),
            NotificationKind::Error => (IconName::CircleX, Colors::error()),
        };
        let label = match self.kind {
            NotificationKind::Success => t!("notifications.complete"),
            NotificationKind::Error => t!("notifications.failed"),
        };

        h_flex()
            .size_full()
            .px(px(16.))
            .gap(px(12.))
            .items_center()
            .rounded(px(10.))
            .border_1()
            .border_color(Colors::border())
            .bg(Colors::card())
            .child(icon_m(icon, accent))
            .child(
                v_flex()
                    .gap(px(2.))
                    .overflow_hidden()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(Colors::foreground())
                            .truncate()
                            .child(self.filename.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(Colors::muted_foreground())
                            .child(label.to_string()),
                    ),
            )
    }
}

pub fn show(cx: &mut App, filename: SharedString, kind: NotificationKind) {
    let options = build_options(cx);
    let Ok(handle) = cx.open_window(options, |_, cx| {
        cx.new(|_| Notification::new(filename, kind))
    }) else {
        return;
    };
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_secs(4)).await;
        handle
            .update(cx, |_, window, _| window.remove_window())
            .ok();
    })
    .detach();
}

fn build_options(cx: &App) -> WindowOptions {
    let w = px(320.);
    let h = px(72.);
    let margin_right = px(16.);
    let margin_top = px(48.);

    let (bounds, display_id) = if let Some(screen) = cx.primary_display() {
        let tr = screen.bounds().top_right();
        let b = Bounds {
            origin: point(tr.x - w - margin_right, tr.y + margin_top),
            size: size(w, h),
        };
        (b, Some(screen.id()))
    } else {
        (
            Bounds {
                origin: point(px(0.), px(0.)),
                size: size(w, h),
            },
            None,
        )
    };

    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        window_background: WindowBackgroundAppearance::Opaque,
        display_id,
        ..Default::default()
    }
}
