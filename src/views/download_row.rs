use gpui::{div, prelude::*, px, relative, App, ElementId, Hsla, SharedString, Window};

use crate::engine::DownloadId;
use crate::ui::prelude::*;

#[derive(Clone, Copy)]
pub enum DownloadState {
    Active,
    Paused,
    Queued,
    Finished,
    Error,
}

impl DownloadState {
    fn color(self) -> Hsla {
        match self {
            Self::Active   => Colors::active().into(),
            Self::Paused   => Colors::queued().into(),
            Self::Queued   => Colors::queued().into(),
            Self::Finished => Colors::finished().into(),
            Self::Error    => Colors::error().into(),
        }
    }

    fn status_icon(self) -> IconName {
        match self {
            Self::Active   => IconName::ArrowDownToLine,
            Self::Paused   => IconName::CirclePause,
            Self::Queued   => IconName::CirclePause,
            Self::Finished => IconName::CircleCheck,
            Self::Error    => IconName::CircleCheck,
        }
    }
}

#[derive(IntoElement)]
pub struct DownloadRow {
    pub id: DownloadId,
    pub filename: SharedString,
    pub destination: SharedString,
    pub progress: f32,
    pub speed: SharedString,
    pub state: DownloadState,
    pub on_pause_resume: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
    pub on_remove: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
}

impl RenderOnce for DownloadRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let color = self.state.color();
        let id = self.id;
        let state = self.state;
        let on_pause_resume = self.on_pause_resume;
        let on_remove = self.on_remove;

        let pause_icon = match state {
            DownloadState::Active | DownloadState::Queued => Some(IconName::CirclePause),
            DownloadState::Paused => Some(IconName::CirclePlay),
            _ => None,
        };

        let pause_btn = pause_icon.map(|icon| {
            div()
                .id(ElementId::NamedInteger("pr".into(), id.0))
                .p(px(4.0))
                .rounded(px(4.0))
                .hover(|s| s.bg(Colors::muted()))
                .on_click(move |_, window, cx| {
                    if let Some(ref cb) = on_pause_resume {
                        cb(window, cx);
                    }
                })
                .child(icon_sm(icon, Colors::muted_foreground()))
        });

        let remove_btn = div()
            .id(ElementId::NamedInteger("rm".into(), id.0))
            .p(px(4.0))
            .rounded(px(4.0))
            .hover(|s| s.bg(Colors::muted()))
            .on_click(move |_, window, cx| {
                if let Some(ref cb) = on_remove {
                    cb(window, cx);
                }
            })
            .child(icon_sm(IconName::Trash2, Colors::muted_foreground()));

        h_flex()
            .items_center()
            .gap(px(14.0))
            .px(px(Spacing::ROW_PADDING_X))
            .py(px(Spacing::ROW_PADDING_Y))
            .rounded(px(10.0))
            .border_1()
            .border_color(Colors::border())
            .bg(Colors::card())
            .hover(|s| s.bg(Colors::card_hover()))
            .child(icon_sm(state.status_icon(), color))
            .child(
                v_flex()
                    .flex_1()
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
                            .child(self.destination),
                    )
                    .child(progress_bar(self.progress, color)),
            )
            .when(matches!(state, DownloadState::Active), |el| {
                el.child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Colors::muted_foreground())
                        .w(px(80.0))
                        .child(self.speed),
                )
            })
            .children(pause_btn)
            .child(remove_btn)
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
