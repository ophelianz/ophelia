use gpui::{App, ElementId, Hsla, SharedString, Window, div, prelude::*, px, relative};

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
            Self::Active => Colors::active().into(),
            Self::Paused | Self::Queued => Colors::queued().into(),
            Self::Finished => Colors::finished().into(),
            Self::Error => Colors::error().into(),
        }
    }

    fn status_icon(self) -> IconName {
        match self {
            Self::Active => IconName::ArrowDownToLine,
            Self::Paused | Self::Queued => IconName::CirclePause,
            Self::Finished => IconName::CircleCheck,
            Self::Error => IconName::CircleX,
        }
    }

    fn pause_icon(self) -> Option<IconName> {
        match self {
            Self::Paused => Some(IconName::CirclePlay),
            Self::Active | Self::Queued => Some(IconName::CirclePause),
            Self::Finished | Self::Error => None,
        }
    }

    fn remove_icon(self) -> IconName {
        match self {
            Self::Active | Self::Queued => IconName::CircleX,
            Self::Paused | Self::Finished | Self::Error => IconName::Trash2,
        }
    }

    fn shows_speed(self) -> bool {
        matches!(self, Self::Active)
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

        h_flex()
            .items_center()
            .gap(px(Spacing::ROW_GAP))
            .px(px(Spacing::ROW_PADDING_X))
            .py(px(Spacing::ROW_PADDING_Y))
            .rounded(px(Chrome::CARD_RADIUS))
            .border_1()
            .border_color(Colors::border())
            .bg(Colors::card())
            .hover(|style| style.bg(Colors::card_hover()))
            .child(icon_sm(self.state.status_icon(), color))
            .child(DownloadRowDetails::new(
                self.filename,
                self.destination,
                self.progress,
                color,
            ))
            .child(DownloadRowActions::new(
                self.id,
                self.state,
                self.speed,
                self.on_pause_resume,
                self.on_remove,
            ))
    }
}

#[derive(IntoElement)]
struct DownloadRowDetails {
    filename: SharedString,
    destination: SharedString,
    progress: f32,
    color: Hsla,
}

impl DownloadRowDetails {
    fn new(filename: SharedString, destination: SharedString, progress: f32, color: Hsla) -> Self {
        Self {
            filename,
            destination,
            progress,
            color,
        }
    }
}

impl RenderOnce for DownloadRowDetails {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        v_flex()
            .flex_1()
            .gap(px(6.0))
            .min_w_0()
            .child(
                div()
                    .text_base()
                    .font_weight(gpui::FontWeight::BOLD)
                    .truncate()
                    .child(self.filename),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(Colors::muted_foreground())
                    .truncate()
                    .child(self.destination),
            )
            .child(progress_bar(self.progress, self.color))
    }
}

#[derive(IntoElement)]
struct DownloadRowActions {
    id: DownloadId,
    state: DownloadState,
    speed: SharedString,
    on_pause_resume: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
    on_remove: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
}

impl DownloadRowActions {
    fn new(
        id: DownloadId,
        state: DownloadState,
        speed: SharedString,
        on_pause_resume: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
        on_remove: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
    ) -> Self {
        Self {
            id,
            state,
            speed,
            on_pause_resume,
            on_remove,
        }
    }
}

impl RenderOnce for DownloadRowActions {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let pause_button = self.on_pause_resume.zip(self.state.pause_icon()).map(
            |(on_pause_resume, icon_name)| {
                row_action_button(
                    ElementId::NamedInteger("pr".into(), self.id.0),
                    icon_name,
                    on_pause_resume,
                )
            },
        );
        let remove_button = self.on_remove.map(|on_remove| {
            row_action_button(
                ElementId::NamedInteger("rm".into(), self.id.0),
                self.state.remove_icon(),
                on_remove,
            )
        });

        h_flex()
            .items_center()
            .gap(px(Spacing::LIST_GAP))
            .when(self.state.shows_speed(), |this| {
                this.child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Colors::muted_foreground())
                        .w(px(80.0))
                        .child(self.speed),
                )
            })
            .children(pause_button)
            .children(remove_button)
    }
}

fn row_action_button(
    id: ElementId,
    icon_name: IconName,
    on_click: Box<dyn Fn(&mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    div()
        .id(id)
        .p(px(4.0))
        .rounded(px(Chrome::CONTROL_RADIUS))
        .cursor_pointer()
        .hover(|style| style.bg(Colors::muted()))
        .on_click(move |_, window, cx| {
            on_click(window, cx);
        })
        .child(icon_sm(icon_name, Colors::muted_foreground()))
}

fn progress_bar(progress: f32, color: Hsla) -> gpui::Div {
    div()
        .w_full()
        .h(px(Chrome::PROGRESS_BAR_HEIGHT))
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
