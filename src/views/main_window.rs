use gpui::{div, prelude::*, px, Context, Window};

use crate::ui::prelude::*;
use crate::theme::Spacing;
use crate::views::sidebar::Sidebar;
use crate::views::stats_bar::StatsBar;
use crate::views::download_list::DownloadList;
use crate::platform;

/// Root view
/// owns the full window layout
///
/// This is the only `Render` (stateful) view at the top level.
/// It composes the sidebar and main content area side by side.
pub struct MainWindow;

impl MainWindow {
    pub fn new() -> Self {
        Self
    }
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(Colors::background())
            .text_color(Colors::foreground())
            .font_family("Inter")
            // Full-width titlebar
            .child(
                div()
                    .h(px(platform::TITLEBAR_HEIGHT))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .px(px(16.0))
                    .border_b_1()
                    .border_color(Colors::border())
                    .child(icon_sm(IconName::Settings, Colors::muted_foreground()))
            )
            // Sidebar + content below
            .child(
                div()
                    .flex()
                    .flex_1()
                    .overflow_hidden()
                    .child(Sidebar)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .overflow_hidden()
                            .child(
                                div()
                                    .id("main-content")
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .overflow_y_scroll()
                                    .px(px(Spacing::CONTENT_PADDING_X))
                                    .py(px(Spacing::CONTENT_PADDING_Y))
                                    .child(StatsBar)
                                    .child(DownloadList),
                            ),
                    )
            )
    }
}
