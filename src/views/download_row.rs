use gpui::{div, prelude::*, px, relative, App, Hsla, SharedString, Window};
use crate::ui::prelude::*;

#[derive(Clone, Copy)]
pub enum DownloadState {
    Active,
    Queued,
    Finished,
}

impl DownloadState {
    fn color(self) -> Hsla {
        match self {
            Self::Active   => Colors::active().into(),
            Self::Queued   => Colors::queued().into(),
            Self::Finished => Colors::finished().into(),
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Active   => IconName::ArrowDownToLine,
            Self::Queued   => IconName::CirclePause,
            Self::Finished => IconName::CircleCheck,
        }
    }
}

#[derive(IntoElement)]
pub struct DownloadRow {
    pub filename: SharedString,
    pub url: SharedString,
    pub progress: f32,
    pub speed: SharedString,
    pub state: DownloadState,
}

impl RenderOnce for DownloadRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let color = self.state.color();

        div()
            .flex()
            .items_center()
            .gap(px(14.0))
            .px(px(Spacing::ROW_PADDING_X))
            .py(px(Spacing::ROW_PADDING_Y))
            .rounded(px(10.0))
            .border_1()
            .border_color(Colors::border())
            .bg(Colors::card())
            .child(icon_sm(IconName::ArrowDownToLine, color))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child(self.filename),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(Colors::muted_foreground())
                            .child(self.url),
                    )
                    .child(progress_bar(self.progress, color)),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(Colors::muted_foreground())
                    .w(px(80.0))
                    .child(self.speed),
            )
            .child(icon_sm(self.state.icon(), color))
            .child(icon_sm(IconName::CirclePause, Colors::muted_foreground()))
            .child(
                div()
                    .text_base()
                    .text_color(Colors::muted_foreground())
                    .child("×"),
            )
    }
}

fn progress_bar(progress: f32, color: Hsla) -> gpui::Div {
    div()
        .w_full()
        .h(px(4.0))
        .rounded_full()
        .bg(Colors::muted())
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(color)
                .w(relative(progress.clamp(0.0, 1.0))),
        )
}
