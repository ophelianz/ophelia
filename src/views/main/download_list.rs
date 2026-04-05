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

use std::rc::Rc;

use gpui::{
    App, Context, Entity, IntoElement, Render, RenderOnce, SharedString, Window, div, prelude::*,
    px, transparent_black,
};

use crate::app::Downloads;
use crate::engine::DownloadStatus;
use crate::ui::prelude::*;
use crate::views::main::download_row::{DownloadRow, DownloadState};

use rust_i18n::t;

type ClickHandler = Rc<dyn Fn(&mut Window, &mut App)>;

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

        let rows = (0..downloads.len())
            .filter(|&i| self.filter.matches(downloads.statuses[i]))
            .map(|i| {
                let id = downloads.ids[i];
                let progress = match downloads.total_bytes[i] {
                    Some(total) if total > 0 => downloads.downloaded_bytes[i] as f32 / total as f32,
                    _ => 0.0,
                };
                let state = match downloads.statuses[i] {
                    DownloadStatus::Downloading => DownloadState::Active,
                    DownloadStatus::Paused => DownloadState::Paused,
                    DownloadStatus::Finished => DownloadState::Finished,
                    DownloadStatus::Error | DownloadStatus::Cancelled => DownloadState::Error,
                    DownloadStatus::Pending => DownloadState::Queued,
                };

                let on_pause_resume: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>> =
                    match state {
                        DownloadState::Active | DownloadState::Queued => {
                            let entity = entity.clone();
                            Some(Box::new(move |_, cx| {
                                entity.update(cx, |downloads, cx| downloads.pause(id, cx));
                            }))
                        }
                        DownloadState::Paused => {
                            let entity = entity.clone();
                            Some(Box::new(move |_, cx| {
                                entity.update(cx, |downloads, cx| downloads.resume(id, cx));
                            }))
                        }
                        DownloadState::Finished | DownloadState::Error => None,
                    };

                let on_remove: Box<dyn Fn(&mut Window, &mut App) + 'static> = {
                    let entity = entity.clone();
                    Box::new(move |_, cx| {
                        entity.update(cx, |downloads, cx| downloads.remove(id, cx));
                    })
                };

                DownloadRow {
                    id,
                    filename: downloads.filenames[i].clone(),
                    destination: downloads.destinations[i].clone(),
                    progress,
                    speed: format_speed(downloads.speeds[i]).into(),
                    state,
                    on_pause_resume,
                    on_remove: Some(on_remove),
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
                        let on_click: ClickHandler = Rc::new({
                            let weak = weak.clone();
                            move |_, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.filter = filter;
                                    cx.notify();
                                });
                            }
                        });
                        TransferFilterChip::new(filter_model, on_click)
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
struct TransferFilterChip {
    model: TransferFilterChipModel,
    on_click: ClickHandler,
}

impl TransferFilterChip {
    fn new(model: TransferFilterChipModel, on_click: ClickHandler) -> Self {
        Self { model, on_click }
    }
}

impl RenderOnce for TransferFilterChip {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_click = Rc::clone(&self.on_click);

        div()
            .id(("transfer-filter", self.model.id))
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
            .child(t!("downloads.empty_state").to_string())
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
