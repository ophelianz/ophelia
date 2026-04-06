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

/***************************************************
** This file is part of Ophelia, distributed under the
** terms of the GPL License, version 3 or later.
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs, do no evil and behave plz )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

use gpui::{
    App, Context, Entity, Hsla, IntoElement, Render, RenderOnce, SharedString, Window, div,
    prelude::*, px,
};

use crate::app::{Downloads, HistoryListRow};
use crate::engine::{ArtifactState, DownloadId, DownloadStatus, HistoryFilter};
use crate::ui::prelude::*;

use rust_i18n::t;

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
                HistoryFilterChipModel::new(
                    4,
                    HistoryFilter::Cancelled,
                    t!("history.filter_cancelled").to_string(),
                    downloads.history_filter == HistoryFilter::Cancelled,
                ),
            ],
            rows: downloads
                .history_rows()
                .into_iter()
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
                        FilterChip::new(
                            ("history-filter", filter_model.id),
                            filter_model.label,
                            filter_model.active,
                        )
                        .on_click({
                            let downloads = downloads.clone();
                            move |_, _, cx| {
                                downloads.update(cx, |downloads, cx| {
                                    downloads.set_history_filter(filter, cx);
                                });
                            }
                        })
                        .into_any_element()
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
    artifact_label: SharedString,
    artifact_color: Hsla,
    size_label: SharedString,
    age_label: SharedString,
}

impl HistoryRowModel {
    fn from_row(row: HistoryListRow) -> Self {
        let (status_icon, status_color) = match row.status {
            DownloadStatus::Finished => (IconName::CircleCheck, Colors::active().into()),
            DownloadStatus::Error => (IconName::CircleX, Colors::error().into()),
            DownloadStatus::Cancelled => (IconName::CircleX, Colors::muted_foreground().into()),
            DownloadStatus::Paused => (IconName::CirclePause, Colors::queued().into()),
            DownloadStatus::Downloading => (IconName::ArrowDownToLine, Colors::finished().into()),
            DownloadStatus::Pending => {
                (IconName::ArrowDownToLine, Colors::muted_foreground().into())
            }
        };
        let (artifact_label, artifact_color) = artifact_state_presentation(row.artifact_state);
        let subtitle = row.source_summary();

        Self {
            id: row.id,
            status_icon,
            status_color,
            filename: row.filename,
            subtitle,
            artifact_label: artifact_label.into(),
            artifact_color,
            size_label: format_bytes(row.total_bytes.unwrap_or(row.downloaded_bytes)).into(),
            age_label: format_age(row.finished_at.unwrap_or(row.added_at)).into(),
        }
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
                            .text_color(self.model.artifact_color)
                            .child(self.model.artifact_label),
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

fn artifact_state_presentation(state: ArtifactState) -> (String, Hsla) {
    match state {
        ArtifactState::Present => (
            t!("history.artifact.present").to_string(),
            Colors::muted_foreground().into(),
        ),
        ArtifactState::Deleted => (
            t!("history.artifact.deleted").to_string(),
            Colors::muted_foreground().into(),
        ),
        ArtifactState::Missing => (
            t!("history.artifact.missing").to_string(),
            Colors::error().into(),
        ),
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
