use std::rc::Rc;

use gpui::{
    App, Context, Entity, Hsla, IntoElement, Render, RenderOnce, SharedString, Window, div,
    prelude::*, px, transparent_black,
};

use crate::app::Downloads;
use crate::engine::{DownloadId, DownloadStatus, HistoryFilter, HistoryRow};
use crate::ui::prelude::*;

use rust_i18n::t;

type ClickHandler = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct HistoryView {
    downloads: Entity<Downloads>,
}

impl HistoryView {
    pub fn new(downloads: Entity<Downloads>, cx: &mut Context<Self>) -> Self {
        cx.observe(&downloads, |_, _, cx| cx.notify()).detach();
        Self { downloads }
    }

    fn view_model(&self, cx: &App) -> HistoryViewModel {
        let downloads = self.downloads.read(cx);

        HistoryViewModel {
            filters: vec![
                HistoryFilterChipModel::new(
                    0,
                    HistoryFilter::All,
                    t!("history.filter_all").to_string(),
                    downloads.history_filter == HistoryFilter::All,
                ),
                HistoryFilterChipModel::new(
                    1,
                    HistoryFilter::Finished,
                    t!("history.filter_finished").to_string(),
                    downloads.history_filter == HistoryFilter::Finished,
                ),
                HistoryFilterChipModel::new(
                    2,
                    HistoryFilter::Error,
                    t!("history.filter_failed").to_string(),
                    downloads.history_filter == HistoryFilter::Error,
                ),
                HistoryFilterChipModel::new(
                    3,
                    HistoryFilter::Paused,
                    t!("history.filter_paused").to_string(),
                    downloads.history_filter == HistoryFilter::Paused,
                ),
            ],
            rows: downloads
                .history
                .iter()
                .map(HistoryRowModel::from_row)
                .collect(),
        }
    }
}

impl Render for HistoryView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view_model = self.view_model(cx);
        let downloads = self.downloads.clone();

        v_flex()
            .child(
                h_flex()
                    .items_center()
                    .gap(px(Chrome::MENU_BAR_GAP))
                    .mb(px(Spacing::SECTION_GAP))
                    .children(view_model.filters.into_iter().map(|filter_model| {
                        let filter = filter_model.filter;
                        let on_click: ClickHandler = Rc::new({
                            let downloads = downloads.clone();
                            move |_, cx| {
                                downloads.update(cx, |downloads, cx| {
                                    downloads.set_history_filter(filter, cx);
                                });
                            }
                        });
                        HistoryFilterChip::new(filter_model, on_click)
                    })),
            )
            .child(if view_model.rows.is_empty() {
                HistoryEmptyState.into_any_element()
            } else {
                v_flex()
                    .gap(px(Spacing::LIST_GAP))
                    .children(view_model.rows.into_iter().map(HistoryItemRow::new))
                    .into_any_element()
            })
    }
}

struct HistoryViewModel {
    filters: Vec<HistoryFilterChipModel>,
    rows: Vec<HistoryRowModel>,
}

#[derive(Clone)]
struct HistoryFilterChipModel {
    id: usize,
    filter: HistoryFilter,
    label: SharedString,
    active: bool,
}

impl HistoryFilterChipModel {
    fn new(id: usize, filter: HistoryFilter, label: impl Into<SharedString>, active: bool) -> Self {
        Self {
            id,
            filter,
            label: label.into(),
            active,
        }
    }
}

struct HistoryRowModel {
    id: DownloadId,
    status_icon: IconName,
    status_color: Hsla,
    filename: SharedString,
    subtitle: SharedString,
    size_label: SharedString,
    age_label: SharedString,
}

impl HistoryRowModel {
    fn from_row(row: &HistoryRow) -> Self {
        let (status_icon, status_color) = match row.status {
            DownloadStatus::Finished => (IconName::CircleCheck, Colors::active().into()),
            DownloadStatus::Error => (IconName::CircleX, Colors::error().into()),
            DownloadStatus::Paused => (IconName::CirclePause, Colors::queued().into()),
            DownloadStatus::Downloading => (IconName::ArrowDownToLine, Colors::finished().into()),
            DownloadStatus::Pending => {
                (IconName::ArrowDownToLine, Colors::muted_foreground().into())
            }
        };

        Self {
            id: row.id,
            status_icon,
            status_color,
            filename: row.filename().to_string().into(),
            subtitle: format_source_label(row).into(),
            size_label: format_bytes(row.total_bytes.unwrap_or(row.downloaded_bytes)).into(),
            age_label: format_age(row.finished_at.unwrap_or(row.added_at)).into(),
        }
    }
}

#[derive(IntoElement)]
struct HistoryFilterChip {
    model: HistoryFilterChipModel,
    on_click: ClickHandler,
}

impl HistoryFilterChip {
    fn new(model: HistoryFilterChipModel, on_click: ClickHandler) -> Self {
        Self { model, on_click }
    }
}

impl RenderOnce for HistoryFilterChip {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_click = Rc::clone(&self.on_click);

        div()
            .id(("history-filter", self.model.id))
            .px(px(12.0))
            .py(px(6.0))
            .rounded(px(Chrome::CONTROL_RADIUS))
            .text_sm()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .cursor_pointer()
            .bg(if self.model.active {
                Colors::muted().into()
            } else {
                transparent_black()
            })
            .text_color(if self.model.active {
                Colors::foreground()
            } else {
                Colors::muted_foreground()
            })
            .on_click(move |_, window, cx| {
                on_click(window, cx);
            })
            .child(self.model.label)
    }
}

#[derive(IntoElement)]
struct HistoryItemRow {
    model: HistoryRowModel,
}

impl HistoryItemRow {
    fn new(model: HistoryRowModel) -> Self {
        Self { model }
    }
}

impl RenderOnce for HistoryItemRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        h_flex()
            .id(("history-row", self.model.id.0))
            .items_center()
            .gap(px(Spacing::ROW_GAP))
            .px(px(Spacing::ROW_PADDING_X))
            .py(px(10.0))
            .rounded(px(Chrome::CARD_RADIUS))
            .border_1()
            .border_color(Colors::border())
            .bg(Colors::card())
            .child(
                div()
                    .flex_shrink_0()
                    .child(icon_sm(self.model.status_icon, self.model.status_color)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(Colors::foreground())
                            .truncate()
                            .child(self.model.filename),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(Colors::muted_foreground())
                            .truncate()
                            .child(self.model.subtitle),
                    ),
            )
            .child(
                v_flex()
                    .flex_shrink_0()
                    .items_end()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_sm()
                            .text_color(Colors::foreground())
                            .child(self.model.size_label),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(Colors::muted_foreground())
                            .child(self.model.age_label),
                    ),
            )
    }
}

#[derive(IntoElement)]
struct HistoryEmptyState;

impl RenderOnce for HistoryEmptyState {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(Colors::muted_foreground())
            .child(t!("history.empty").to_string())
    }
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
        format!("{bytes} B")
    }
}

fn format_source_label(row: &HistoryRow) -> String {
    if row.provider_kind == "http" {
        row.source_label.clone()
    } else {
        format!("{}: {}", row.provider_kind, row.source_label)
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
        t!("history.age.just_now").to_string()
    } else if secs < 3600 {
        let minutes = secs / 60;
        if minutes == 1 {
            t!("history.age.minute_ago").to_string()
        } else {
            t!("history.age.minutes_ago", count = minutes).to_string()
        }
    } else if secs < 86400 {
        let hours = secs / 3600;
        if hours == 1 {
            t!("history.age.hour_ago").to_string()
        } else {
            t!("history.age.hours_ago", count = hours).to_string()
        }
    } else {
        let days = secs / 86400;
        if days == 1 {
            t!("history.age.yesterday").to_string()
        } else {
            t!("history.age.days_ago", count = days).to_string()
        }
    }
}
