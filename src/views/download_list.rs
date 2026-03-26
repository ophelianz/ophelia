use gpui::{div, prelude::*, px, App, Window};
use crate::ui::prelude::*;
use crate::views::download_row::DownloadRow;

#[derive(IntoElement)]
pub struct DownloadList {
    pub rows: Vec<DownloadRow>,
}

impl DownloadList {
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }
}

impl RenderOnce for DownloadList {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .text_sm()
                    .text_color(Colors::muted_foreground())
                    .font_weight(gpui::FontWeight::EXTRA_BOLD)
                    .mb(px(14.0))
                    .child("RECENT"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(Spacing::LIST_GAP))
                    .children(self.rows),
            )
    }
}
