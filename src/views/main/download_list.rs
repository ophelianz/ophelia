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

use gpui::{
    App, Context, Entity, IntoElement, Render, RenderOnce, SharedString, Window, div, prelude::*,
    px,
};

use crate::app::Downloads;
use crate::engine::DownloadStatus;
use crate::ui::prelude::*;
use crate::views::main::download_row::DownloadRow;

use rust_i18n::t;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransferFilter {
    All,
    Active,
    Finished,
    Paused,
    Failed,
}

impl TransferFilter {
    fn matches(self, status: DownloadStatus) -> bool {
        match self {
            Self::All => true,
            Self::Active => matches!(
                status,
                DownloadStatus::Downloading | DownloadStatus::Pending
            ),
            Self::Finished => status == DownloadStatus::Finished,
            Self::Paused => status == DownloadStatus::Paused,
            Self::Failed => matches!(status, DownloadStatus::Error | DownloadStatus::Cancelled),
        }
    }
}

pub struct DownloadList {
    downloads: Entity<Downloads>,
    filter: TransferFilter,
}

impl DownloadList {
    pub fn new(downloads: Entity<Downloads>, cx: &mut Context<Self>) -> Self {
        cx.observe(&downloads, |_, _, cx| cx.notify()).detach();
        Self {
            downloads,
            filter: TransferFilter::All,
        }
    }

    fn view_model(&self, cx: &App) -> DownloadListViewModel {
        let entity = self.downloads.clone();
        let downloads = self.downloads.read(cx);

        let filters = vec![
            TransferFilterChipModel::new(
                0,
                TransferFilter::All,
                t!("transfers.filter_all").to_string(),
                self.filter == TransferFilter::All,
            ),
            TransferFilterChipModel::new(
                1,
                TransferFilter::Active,
                t!("transfers.filter_active").to_string(),
                self.filter == TransferFilter::Active,
            ),
            TransferFilterChipModel::new(
                2,
                TransferFilter::Finished,
                t!("transfers.filter_finished").to_string(),
                self.filter == TransferFilter::Finished,
            ),
            TransferFilterChipModel::new(
                3,
                TransferFilter::Paused,
                t!("transfers.filter_paused").to_string(),
                self.filter == TransferFilter::Paused,
            ),
            TransferFilterChipModel::new(
                4,
                TransferFilter::Failed,
                t!("transfers.filter_failed").to_string(),
                self.filter == TransferFilter::Failed,
            ),
        ];

        let rows = downloads
            .transfer_rows()
            .into_iter()
            .filter(|row| self.filter.matches(row.status))
            .map(|row| {
                let id = row.id;
                let on_pause_resume: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>> =
                    if row.available_actions.pause {
                        let entity = entity.clone();
                        Some(Box::new(move |_window: &mut Window, app: &mut App| {
                            entity.update(app, |downloads, cx| downloads.pause(id, cx));
                        }))
                    } else if row.available_actions.resume {
                        let entity = entity.clone();
                        Some(Box::new(move |_window: &mut Window, app: &mut App| {
                            entity.update(app, |downloads, cx| downloads.resume(id, cx));
                        }))
                    } else {
                        None
                    };

                let on_remove = if row.available_actions.delete_artifact {
                    let entity = entity.clone();
                    Some(Box::new(move |_window: &mut Window, app: &mut App| {
                        entity.update(app, |downloads, cx| downloads.remove(id, cx));
                    })
                        as Box<dyn Fn(&mut Window, &mut App) + 'static>)
                } else {
                    None
                };

                DownloadRow {
                    id,
                    filename: row.filename,
                    destination: row.destination,
                    progress: row.progress,
                    speed: format_speed(row.speed_bps).into(),
                    state: row.display_state,
                    on_pause_resume,
                    on_remove,
                }
            })
            .collect();

        DownloadListViewModel { filters, rows }
    }
}

impl Render for DownloadList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view_model = self.view_model(cx);
        let weak = cx.weak_entity();

        v_flex()
            .pt(px(Spacing::SECTION_GAP))
            .child(
                div()
                    .text_sm()
                    .text_color(Colors::muted_foreground())
                    .font_weight(gpui::FontWeight::EXTRA_BOLD)
                    .mb(px(Spacing::SECTION_LABEL_BOTTOM_MARGIN))
                    .child(t!("transfers.section_label").to_string()),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(Chrome::MENU_BAR_GAP))
                    .mb(px(Spacing::SECTION_GAP))
                    .children(view_model.filters.into_iter().map(|filter_model| {
                        let filter = filter_model.filter;
                        FilterChip::new(
                            ("transfer-filter", filter_model.id),
                            filter_model.label,
                            filter_model.active,
                        )
                        .on_click({
                            let weak = weak.clone();
                            move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.filter = filter;
                                    cx.notify();
                                });
                            }
                        })
                        .into_any_element()
                    })),
            )
            .child(if view_model.rows.is_empty() {
                DownloadListEmptyState.into_any_element()
            } else {
                v_flex()
                    .gap(px(Spacing::LIST_GAP))
                    .children(view_model.rows)
                    .into_any_element()
            })
    }
}

struct DownloadListViewModel {
    filters: Vec<TransferFilterChipModel>,
    rows: Vec<DownloadRow>,
}

#[derive(Clone)]
struct TransferFilterChipModel {
    id: usize,
    filter: TransferFilter,
    label: SharedString,
    active: bool,
}

impl TransferFilterChipModel {
    fn new(
        id: usize,
        filter: TransferFilter,
        label: impl Into<SharedString>,
        active: bool,
    ) -> Self {
        Self {
            id,
            filter,
            label: label.into(),
            active,
        }
    }
}

#[derive(IntoElement)]
struct DownloadListEmptyState;

impl RenderOnce for DownloadListEmptyState {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(Colors::muted_foreground())
            .child(t!("transfers.empty_state").to_string())
    }
}

fn format_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec == 0 {
        return String::new();
    }
    const MB: u64 = 1_000_000;
    const KB: u64 = 1_000;
    if bytes_per_sec >= MB {
        format!("{:.1} MB/s", bytes_per_sec as f64 / MB as f64)
    } else if bytes_per_sec >= KB {
        format!("{:.0} KB/s", bytes_per_sec as f64 / KB as f64)
    } else {
        format!("{} B/s", bytes_per_sec)
    }
}
