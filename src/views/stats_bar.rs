use gpui::{div, prelude::*, px, App, Hsla, Window};
use crate::ui::prelude::*;

/// 4-column stats grid: speed, active, finished, total
#[derive(IntoElement)]
pub struct StatsBar;

impl RenderOnce for StatsBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .gap(px(Spacing::CARD_GAP))
            .mb(px(32.0))
            .child(stat_card("Speed",    "0 B/s", Colors::foreground()))
            .child(stat_card("Active",   "0",     Colors::active()))
            .child(stat_card("Finished", "0",     Colors::finished()))
            .child(stat_card("Total",    "0",     Colors::queued()))

    }
}

/// A single stat card: label + large value.
/// The color tints the label to communicate state at a glance.
fn stat_card(label: &str, value: &str, color: impl Into<Hsla>) -> gpui::Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .py(px(16.0))
        .px(px(16.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(Colors::border())
        .bg(Colors::card())
        .child(
            div()
                .text_xs()
                .text_color(color)
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(label.to_string()),
        )
        .child(
            div()
                .text_2xl()
                .font_weight(gpui::FontWeight::BOLD)
                .child(value.to_string()),
        )
}
