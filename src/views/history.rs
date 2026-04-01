use gpui::{Context, Entity, IntoElement, Window, div, prelude::*, px, transparent_black};

use crate::app::Downloads;
use crate::engine::{DownloadStatus, HistoryFilter, HistoryRow};
use crate::ui::prelude::*;
use crate::theme::Spacing;

pub struct HistoryView {
    downloads: Entity<Downloads>,
}

impl HistoryView {
    pub fn new(downloads: Entity<Downloads>, cx: &mut Context<Self>) -> Self {
        cx.observe(&downloads, |_, _, cx| cx.notify()).detach();
        Self { downloads }
    }
}

impl Render for HistoryView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let d = self.downloads.read(cx);
        let rows = d.history.clone();
        let current_filter = d.history_filter;
        let entity = self.downloads.clone();

        v_flex()
            // Filter tabs
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .mb(px(16.))
                    .children([
                        (HistoryFilter::All,      rust_i18n::t!("history.filter_all").to_string()),
                        (HistoryFilter::Finished, rust_i18n::t!("history.filter_finished").to_string()),
                        (HistoryFilter::Error,    rust_i18n::t!("history.filter_failed").to_string()),
                        (HistoryFilter::Paused,   rust_i18n::t!("history.filter_paused").to_string()),
                    ].into_iter().enumerate().map(|(i, (filter, label))| {
                        let active = filter == current_filter;
                        let e = entity.clone();
                        div()
                            .id(i)
                            .px(px(12.))
                            .py(px(6.))
                            .rounded(px(6.))
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .cursor_pointer()
                            .bg(if active { Colors::muted().into() } else { transparent_black() })
                            .text_color(if active {
                                Colors::foreground()
                            } else {
                                Colors::muted_foreground()
                            })
                            .on_click(move |_, _, cx| {
                                e.update(cx, |d, cx| d.set_history_filter(filter, cx));
                            })
                            .child(label)
                    })),
            )
            // Rows or empty state
            .child(if rows.is_empty() {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(Colors::muted_foreground())
                    .child(rust_i18n::t!("history.empty").to_string())
                    .into_any_element()
            } else {
                v_flex()
                    .gap(px(Spacing::LIST_GAP))
                    .children(rows.iter().map(|row| history_row(row)))
                    .into_any_element()
            })
    }
}

fn history_row(row: &HistoryRow) -> gpui::Div {
    let (status_icon, status_color) = match row.status {
        DownloadStatus::Finished   => (IconName::CircleCheck, Colors::active()),
        DownloadStatus::Error      => (IconName::CircleX,     Colors::error()),
        DownloadStatus::Paused     => (IconName::CirclePause, Colors::queued()),
        DownloadStatus::Downloading => (IconName::ArrowDownToLine, Colors::finished()),
        DownloadStatus::Pending    => (IconName::ArrowDownToLine, Colors::muted_foreground()),
    };

    let size_str = format_bytes(row.total_bytes.unwrap_or(row.downloaded_bytes));
    let age_str  = format_age(row.added_at);

    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(Spacing::ROW_PADDING_X))
        .py(px(10.))
        .rounded(px(10.))
        .border_1()
        .border_color(Colors::border())
        .bg(Colors::card())
        // Status icon
        .child(
            div()
                .flex_shrink_0()
                .child(icon_sm(status_icon, status_color)),
        )
        // Filename + URL
        .child(
            v_flex()
                .flex_1()
                .overflow_hidden()
                .gap(px(2.))
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(Colors::foreground())
                        .truncate()
                        .child(row.filename().to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(Colors::muted_foreground())
                        .truncate()
                        .child(row.url.clone()),
                ),
        )
        // Size + age
        .child(
            v_flex()
                .flex_shrink_0()
                .items_end()
                .gap(px(2.))
                .child(
                    div()
                        .text_sm()
                        .text_color(Colors::foreground())
                        .child(size_str),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(Colors::muted_foreground())
                        .child(age_str),
                ),
        )
}

fn format_bytes(bytes: u64) -> String {
    const GB: u64 = 1_000_000_000;
    const MB: u64 = 1_000_000;
    const KB: u64 = 1_000;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Convert a unix-millisecond timestamp to a human-readable age string.
fn format_age(added_at_ms: i64) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let secs = ((now_ms - added_at_ms) / 1000).max(0) as u64;

    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        let m = secs / 60;
        if m == 1 { "1 minute ago".to_string() } else { format!("{m} minutes ago") }
    } else if secs < 86400 {
        let h = secs / 3600;
        if h == 1 { "1 hour ago".to_string() } else { format!("{h} hours ago") }
    } else {
        let d = secs / 86400;
        if d == 1 { "yesterday".to_string() } else { format!("{d} days ago") }
    }
}
