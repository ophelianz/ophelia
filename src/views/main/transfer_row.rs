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

use gpui::{App, ElementId, Hsla, RenderOnce, SharedString, Window, div, prelude::*, px, relative};

use crate::app::TransferDisplayState;
use crate::engine::DownloadId;
use crate::format::{DataQuantity, data};
use crate::ui::prelude::*;

impl TransferDisplayState {
    fn accent_color(self) -> Hsla {
        match self {
            Self::Active => Colors::active().into(),
            Self::Paused => Colors::muted_foreground().into(),
            Self::Queued => Colors::queued().into(),
            Self::Finished => Colors::finished().into(),
            Self::Error => Colors::error().into(),
        }
    }

    fn badge_border_color(self) -> Hsla {
        match self {
            Self::Active => Colors::active().into(),
            Self::Paused => Colors::border().into(),
            Self::Queued => Colors::queued().into(),
            Self::Finished => Colors::finished().into(),
            Self::Error => Colors::error().into(),
        }
    }

    fn badge_label(self) -> &'static str {
        match self {
            Self::Active => "Downloading",
            Self::Paused => "Paused",
            Self::Queued => "Queued",
            Self::Finished => "Finished",
            Self::Error => "Error",
        }
    }

    fn action_icon(self) -> Option<IconName> {
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
}

#[derive(IntoElement)]
pub struct TransferRow {
    pub id: DownloadId,
    pub filename: SharedString,
    pub destination: SharedString,
    pub icon_name: SharedString,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub progress: f32,
    pub state: TransferDisplayState,
    pub selected: bool,
    pub on_select: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
    pub on_open_destination: Box<dyn Fn(&mut Window, &mut App) + 'static>,
    pub on_pause_resume: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
    pub on_remove: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
}

impl RenderOnce for TransferRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let border_color = if self.selected {
            Colors::ring()
        } else {
            Colors::border()
        };
        let meta_size = format_size_label(self.downloaded_bytes, self.total_bytes);
        let progress_label = progress_percentage_label(self.progress);

        let row = h_flex()
            .id(("transfer-row", self.id.0))
            .w_full()
            .items_center()
            .gap(px(Spacing::ROW_GAP))
            .px(px(16.0))
            .py(px(14.0))
            .rounded(px(Chrome::PANEL_RADIUS))
            .border_1()
            .border_color(border_color)
            .bg(Colors::card())
            .cursor_pointer()
            .hover(|style| style.border_color(Colors::input_border()))
            .child(transfer_icon_tile(&self.icon_name))
            .child(
                TransferRowDetails::new(
                    self.filename,
                    self.destination,
                    meta_size.into(),
                    progress_label.into(),
                    self.progress,
                    self.state,
                )
                .into_any_element(),
            )
            .child(
                TransferRowActions::new(
                    self.id,
                    self.state,
                    self.on_open_destination,
                    self.on_pause_resume,
                    self.on_remove,
                )
                .into_any_element(),
            );

        if let Some(on_select) = self.on_select {
            row.on_click(move |_, window, cx| on_select(window, cx))
        } else {
            row
        }
    }
}

#[derive(IntoElement)]
struct TransferRowDetails {
    filename: SharedString,
    destination: SharedString,
    size_label: SharedString,
    progress_label: SharedString,
    progress: f32,
    state: TransferDisplayState,
}

impl TransferRowDetails {
    fn new(
        filename: SharedString,
        destination: SharedString,
        size_label: SharedString,
        progress_label: SharedString,
        progress: f32,
        state: TransferDisplayState,
    ) -> Self {
        Self {
            filename,
            destination,
            size_label,
            progress_label,
            progress,
            state,
        }
    }
}

impl RenderOnce for TransferRowDetails {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap(px(6.0))
            .child(
                h_flex()
                    .items_start()
                    .justify_between()
                    .gap(px(Spacing::LIST_GAP))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_base()
                            .font_weight(gpui::FontWeight::NORMAL)
                            .truncate()
                            .child(self.filename),
                    )
                    .child(status_badge(self.state)),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_sm()
                    .text_color(Colors::muted_foreground())
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_weight(gpui::FontWeight::LIGHT)
                            .child(self.size_label),
                    )
                    .child(div().flex_shrink_0().child("•"))
                    .child(div().flex_1().min_w_0().truncate().child(self.destination)),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(Spacing::LIST_GAP))
                    .child(progress_bar(self.progress, self.state.accent_color()).flex_1())
                    .child(
                        div()
                            .w(px(38.0))
                            .text_right()
                            .text_sm()
                            .font_weight(gpui::FontWeight::LIGHT)
                            .text_color(Colors::foreground())
                            .child(self.progress_label),
                    ),
            )
    }
}

#[derive(IntoElement)]
struct TransferRowActions {
    id: DownloadId,
    state: TransferDisplayState,
    on_open_destination: Box<dyn Fn(&mut Window, &mut App) + 'static>,
    on_pause_resume: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
    on_remove: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
}

impl TransferRowActions {
    fn new(
        id: DownloadId,
        state: TransferDisplayState,
        on_open_destination: Box<dyn Fn(&mut Window, &mut App) + 'static>,
        on_pause_resume: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
        on_remove: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>>,
    ) -> Self {
        Self {
            id,
            state,
            on_open_destination,
            on_pause_resume,
            on_remove,
        }
    }
}

impl RenderOnce for TransferRowActions {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let open_button = row_action_button(
            ElementId::NamedInteger("transfer-row-action-open".into(), self.id.0),
            Some("transfer-row-open-action"),
            IconName::Folder,
            self.on_open_destination,
        );
        let pause_button = self.on_pause_resume.zip(self.state.action_icon()).map(
            |(on_pause_resume, icon_name)| {
                row_action_button(
                    ElementId::NamedInteger("transfer-row-action-primary".into(), self.id.0),
                    Some("transfer-row-primary-action"),
                    icon_name,
                    on_pause_resume,
                )
            },
        );
        let remove_button = self.on_remove.map(|on_remove| {
            row_action_button(
                ElementId::NamedInteger("transfer-row-action-remove".into(), self.id.0),
                Some("transfer-row-remove-action"),
                self.state.remove_icon(),
                on_remove,
            )
        });

        h_flex()
            .items_center()
            .gap(px(Spacing::LIST_GAP))
            .flex_shrink_0()
            .child(open_button)
            .children(pause_button)
            .children(remove_button)
    }
}

fn transfer_icon_tile(icon_name: &str) -> impl IntoElement {
    div()
        .size(px(52.0))
        .rounded(px(Chrome::PANEL_RADIUS))
        .border_1()
        .border_color(Colors::border())
        .bg(Colors::background())
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .child(file_type_icon(icon_name, px(26.0), Colors::foreground()))
}

fn status_badge(state: TransferDisplayState) -> impl IntoElement {
    div()
        .px(px(8.0))
        .py(px(3.0))
        .rounded_full()
        .border_1()
        .border_color(state.badge_border_color())
        .bg(Colors::background())
        .text_xs()
        .font_weight(gpui::FontWeight::LIGHT)
        .text_color(state.accent_color())
        .child(state.badge_label())
}

fn row_action_button(
    id: ElementId,
    debug_selector: Option<&'static str>,
    icon_name: IconName,
    on_click: Box<dyn Fn(&mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    let button = IconButton::new(id, icon_name)
        .stop_propagation()
        .on_click(move |_, window, cx| on_click(window, cx));

    if let Some(debug_selector) = debug_selector {
        button.debug_selector(debug_selector)
    } else {
        button
    }
}

fn progress_bar(progress: f32, color: Hsla) -> gpui::Div {
    div()
        .w_full()
        .h(px(Chrome::PROGRESS_BAR_HEIGHT))
        .rounded_full()
        .bg(Colors::border())
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(color)
                .w(relative(progress.clamp(0.0, 1.0))),
        )
}

fn format_size_label(downloaded_bytes: u64, total_bytes: Option<u64>) -> String {
    data(DataQuantity::Bytes(total_bytes.unwrap_or(downloaded_bytes))).to_string()
}

fn progress_percentage_label(progress: f32) -> String {
    format!("{:.0}%", (progress.clamp(0.0, 1.0) * 100.0).round())
}

pub(crate) fn default_transfer_icon_name_for_filename(filename: &str) -> &'static str {
    let extension = std::path::Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{}", extension.to_ascii_lowercase()));

    match extension.as_deref() {
        Some(".zip" | ".rar" | ".7z" | ".tar" | ".gz" | ".bz2" | ".xz" | ".tgz") => "archive",
        Some(".mp3" | ".flac" | ".wav" | ".aac" | ".ogg" | ".m4a" | ".opus") => "audio",
        Some(".epub" | ".mobi" | ".azw3" | ".fb2") => "book",
        Some(
            ".rs" | ".js" | ".ts" | ".tsx" | ".jsx" | ".py" | ".go" | ".java" | ".c" | ".cpp"
            | ".h" | ".hpp" | ".json" | ".yaml" | ".yml" | ".toml" | ".sh" | ".css",
        ) => "code",
        Some(".pdf" | ".doc" | ".docx" | ".txt" | ".rtf" | ".md") => "document",
        Some(".exe" | ".msi" | ".dmg" | ".pkg" | ".appimage" | ".deb" | ".rpm" | ".apk") => {
            "executable"
        }
        Some(".ttf" | ".otf" | ".woff" | ".woff2") => "font",
        Some(
            ".png" | ".jpg" | ".jpeg" | ".gif" | ".webp" | ".heic" | ".avif" | ".bmp" | ".tiff",
        ) => "image",
        Some(".pem" | ".pub" | ".p12" | ".pfx" | ".crt" | ".cer" | ".asc") => "key",
        Some(".eml" | ".mbox" | ".msg") => "mail",
        Some(".ppt" | ".pptx" | ".odp") => "presentation",
        Some(".csv" | ".tsv" | ".xls" | ".xlsx" | ".ods") => "spreadsheet",
        Some(".svg" | ".ai" | ".eps") => "vector",
        Some(".mp4" | ".mkv" | ".mov" | ".avi" | ".webm" | ".m4v" | ".wmv") => "video",
        Some(".html" | ".htm" | ".mhtml" | ".webloc" | ".url") => "web",
        _ => "default",
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Deref;

    use gpui::{
        Context, Modifiers, MouseButton, Render, TestApp, TestAppContext, VisualTestContext,
        Window, point, px, size,
    };

    use super::*;

    #[test]
    fn percentage_label_is_clamped_and_rounded() {
        assert_eq!(progress_percentage_label(0.643), "64%");
        assert_eq!(progress_percentage_label(5.0), "100%");
        assert_eq!(progress_percentage_label(-1.0), "0%");
    }

    #[test]
    fn icon_heuristics_follow_filename_extension() {
        assert_eq!(
            default_transfer_icon_name_for_filename("movie.mkv"),
            "video"
        );
        assert_eq!(
            default_transfer_icon_name_for_filename("report.pdf"),
            "document"
        );
        assert_eq!(
            default_transfer_icon_name_for_filename("archive.tar.gz"),
            "archive"
        );
        assert_eq!(default_transfer_icon_name_for_filename("README"), "default");
    }

    struct TransferRowHost {
        selected: bool,
        selection_events: usize,
        open_action_events: usize,
        primary_action_events: usize,
    }

    impl TransferRowHost {
        fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
            Self {
                selected: false,
                selection_events: 0,
                open_action_events: 0,
                primary_action_events: 0,
            }
        }
    }

    impl Render for TransferRowHost {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let entity = cx.entity();
            div()
                .size_full()
                .p(px(20.0))
                .child(div().w(px(720.0)).child(TransferRow {
                    id: DownloadId(7),
                    filename: "movie.mkv".into(),
                    destination: "/tmp/Videos/movie.mkv".into(),
                    icon_name: "video".into(),
                    downloaded_bytes: 64_000_000,
                    total_bytes: Some(100_000_000),
                    progress: 0.64,
                    state: TransferDisplayState::Active,
                    selected: self.selected,
                    on_select: Some(Box::new({
                        let entity = entity.clone();
                        move |_window, app| {
                            entity.update(app, |this, cx| {
                                this.selected = true;
                                this.selection_events += 1;
                                cx.notify();
                            });
                        }
                    })),
                    on_open_destination: Box::new({
                        let entity = entity.clone();
                        move |_window, app| {
                            entity.update(app, |this, cx| {
                                this.open_action_events += 1;
                                cx.notify();
                            });
                        }
                    }),
                    on_pause_resume: Some(Box::new({
                        let entity = entity.clone();
                        move |_window, app| {
                            entity.update(app, |this, cx| {
                                this.primary_action_events += 1;
                                cx.notify();
                            });
                        }
                    })),
                    on_remove: Some(Box::new(|_, _| {})),
                }))
        }
    }

    #[test]
    fn clicking_the_row_selects_it() {
        let mut app = TestApp::new();
        let mut window = app.open_window(TransferRowHost::new);

        window.draw();
        window.simulate_click(point(px(180.0), px(56.0)), MouseButton::Left);

        window.read(|host, _| {
            assert!(host.selected);
            assert_eq!(host.selection_events, 1);
        });
    }

    #[test]
    fn clicking_the_primary_action_does_not_select_the_row() {
        let mut app = TestAppContext::single();
        let window = app.open_window(size(px(800.0), px(240.0)), TransferRowHost::new);
        let view = window.root(&mut app).unwrap();
        let cx = VisualTestContext::from_window(*window.deref(), &app).into_mut();

        cx.run_until_parked();
        let button_bounds = cx
            .debug_bounds("transfer-row-primary-action")
            .expect("primary action button should render debug bounds");
        cx.simulate_click(button_bounds.center(), Modifiers::none());

        cx.update(|_window, app| {
            view.update(app, |host: &mut TransferRowHost, _cx| {
                assert!(!host.selected);
                assert_eq!(host.selection_events, 0);
                assert_eq!(host.open_action_events, 0);
                assert_eq!(host.primary_action_events, 1);
            });
        });
    }

    #[test]
    fn clicking_the_open_action_does_not_select_the_row() {
        let mut app = TestAppContext::single();
        let window = app.open_window(size(px(800.0), px(240.0)), TransferRowHost::new);
        let view = window.root(&mut app).unwrap();
        let cx = VisualTestContext::from_window(*window.deref(), &app).into_mut();

        cx.run_until_parked();
        let button_bounds = cx
            .debug_bounds("transfer-row-open-action")
            .expect("open action button should render debug bounds");
        cx.simulate_click(button_bounds.center(), Modifiers::none());

        cx.update(|_window, app| {
            view.update(app, |host: &mut TransferRowHost, _cx| {
                assert!(!host.selected);
                assert_eq!(host.selection_events, 0);
                assert_eq!(host.open_action_events, 1);
                assert_eq!(host.primary_action_events, 0);
            });
        });
    }
}
