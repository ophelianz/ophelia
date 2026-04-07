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
    App, Context, Entity, Pixels, StatefulInteractiveElement as _, Window, div, prelude::*, px,
};

use crate::app::Downloads;
use crate::app_actions;
use crate::app_menu;
use crate::engine::DownloadId;
use crate::settings::Settings;
use crate::theme::{APP_FONT_FAMILY, Spacing};
use crate::ui::prelude::*;
use crate::views::overlays::about_modal::AboutLayer;
use crate::views::overlays::download_modal::DownloadModalLayer;

use super::chunk_map::{ChunkMapCard, ChunkMapCardModel};
use super::history::HistoryView;
use super::sidebar::Sidebar;
use super::stats_bar::StatsBar;
use super::transfers_list::{TransferList, TransferListSelectionChanged};

const HISTORY_NAV_INDEX: usize = 1;
const SIDEBAR_MIN_WIDTH: f32 = 200.0;
const SIDEBAR_MAX_WIDTH: f32 = 320.0;
const TRANSFERS_TOP_PANEL_DEFAULT_HEIGHT: f32 = 320.0;
const TRANSFERS_TOP_PANEL_MIN_HEIGHT: f32 = 180.0;
const TRANSFERS_TOP_PANEL_MAX_HEIGHT: f32 = 320.0;
const TRANSFERS_BOTTOM_PANEL_MIN_HEIGHT: f32 = 260.0;
const TRANSFERS_STATS_PANEL_DEFAULT_WIDTH: f32 = 640.0;
const TRANSFERS_STATS_PANEL_MIN_WIDTH: f32 = 360.0;
const TRANSFERS_CHUNK_PANEL_DEFAULT_WIDTH: f32 = 320.0;
const TRANSFERS_CHUNK_PANEL_MIN_WIDTH: f32 = 220.0;

/// Root view
/// owns the full window layout and all live state.
pub struct MainWindow {
    menu_bar: Entity<AppMenuBar>,
    sidebar: Entity<Sidebar>,
    sidebar_layout: Entity<ResizableState>,
    downloads: Entity<Downloads>,
    transfer_list: Entity<TransferList>,
    selected_transfer_id: Option<DownloadId>,
    history_view: Entity<HistoryView>,
    about_modal: Entity<AboutLayer>,
    download_modal: Entity<DownloadModalLayer>,
}

impl MainWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let menu_bar = cx.new(|cx| AppMenuBar::new(app_menu::build_owned_menus(), cx));
        let downloads = cx.new(|cx| Downloads::new(cx));
        let sidebar = cx.new(|cx| Sidebar::new(downloads.clone(), cx));
        let sidebar_layout = cx.new(|_| ResizableState::default());
        let transfer_list = cx.new(|cx| TransferList::new(downloads.clone(), cx));
        let history_view = cx.new(|cx| HistoryView::new(downloads.clone(), cx));
        let about_visibility = cx.global::<app_actions::AppState>().show_about.clone();
        let download_modal_visibility = cx
            .global::<app_actions::AppState>()
            .show_download_modal
            .clone();
        let about_modal = cx.new(|cx| AboutLayer::new(about_visibility, cx));
        let download_modal =
            cx.new(|cx| DownloadModalLayer::new(downloads.clone(), download_modal_visibility, cx));

        // Re-render when sidebar nav changes (to switch content pane).
        cx.observe(&sidebar, |_, _, cx| cx.notify()).detach();
        cx.subscribe(
            &transfer_list,
            |this: &mut Self, _, event: &TransferListSelectionChanged, cx| {
                this.selected_transfer_id = event.id;
                cx.notify();
            },
        )
        .detach();

        Self {
            menu_bar,
            sidebar,
            sidebar_layout,
            downloads,
            transfer_list,
            selected_transfer_id: None,
            history_view,
            about_modal,
            download_modal,
        }
    }

    pub(crate) fn apply_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        self.downloads.update(cx, |downloads, cx| {
            downloads.apply_settings(settings.clone(), cx);
        });
        self.menu_bar.update(cx, |menu_bar, cx| {
            menu_bar.set_menus(app_menu::build_owned_menus(), cx);
        });
        cx.notify();
    }

    fn view_model(&self, cx: &App) -> MainWindowViewModel {
        let active_nav = self.sidebar.read(cx).active_item;

        let content = if active_nav == HISTORY_NAV_INDEX {
            MainContentViewModel::History
        } else {
            let downloads = self.downloads.read(cx);
            let transfer_rows = self.transfer_list.read(cx).visible_transfer_rows(cx);
            let selected_transfer_id = resolve_selected_transfer_id_for_transfers(
                &transfer_rows,
                self.selected_transfer_id,
            );
            let (active, finished, queued) = downloads.status_counts();

            MainContentViewModel::Downloads(TransfersSummaryViewModel {
                stats: StatsBarViewModel {
                    download_samples: downloads.speed_samples_mbs(),
                    download_speed: downloads.download_speed_bps() as f32 / 1_000_000.0,
                    disk_read_speed: None,
                    disk_write_speed: None,
                    active_count: active,
                    finished_count: finished,
                    queued_count: queued,
                },
                chunk_map: ChunkMapCardModel::from_transfer_rows(
                    &transfer_rows,
                    &downloads,
                    selected_transfer_id,
                ),
            })
        };

        MainWindowViewModel { content }
    }
}

fn resolve_selected_transfer_id_for_transfers(
    rows: &[crate::app::TransferListRow],
    selected_id: Option<DownloadId>,
) -> Option<DownloadId> {
    match selected_id {
        Some(selected_id) if rows.iter().any(|row| row.id == selected_id) => Some(selected_id),
        _ => rows.first().map(|row| row.id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{TransferAvailableActions, TransferDisplayState, TransferListRow};
    use crate::engine::DownloadStatus;

    #[test]
    fn chunk_map_selection_defaults_to_first_visible_transfer() {
        let rows = vec![test_row(DownloadId(4)), test_row(DownloadId(9))];

        assert_eq!(
            resolve_selected_transfer_id_for_transfers(&rows, None),
            Some(DownloadId(4))
        );
    }

    #[test]
    fn chunk_map_selection_falls_back_when_current_transfer_disappears() {
        let rows = vec![test_row(DownloadId(12)), test_row(DownloadId(15))];

        assert_eq!(
            resolve_selected_transfer_id_for_transfers(&rows, Some(DownloadId(99))),
            Some(DownloadId(12))
        );
    }

    fn test_row(id: DownloadId) -> TransferListRow {
        TransferListRow {
            id,
            provider_kind: "http".into(),
            source_label: "https://example.com/file.bin".into(),
            filename: "file.bin".into(),
            destination: "/tmp/file.bin".into(),
            status: DownloadStatus::Downloading,
            downloaded_bytes: 512,
            total_bytes: Some(1024),
            progress: 0.5,
            speed_bps: 0,
            display_state: TransferDisplayState::Active,
            available_actions: TransferAvailableActions {
                pause: true,
                resume: false,
                cancel: true,
                delete_artifact: true,
            },
        }
    }
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view_model = self.view_model(cx);

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(Colors::background())
            .text_color(Colors::foreground())
            .font_family(APP_FONT_FAMILY)
            .child(self.render_header(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .overflow_hidden()
                    .child(self.render_body(view_model, cx)),
            )
            .child(self.download_modal.clone())
            .child(self.about_modal.clone())
    }
}

impl MainWindow {
    fn render_body(
        &self,
        view_model: MainWindowViewModel,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let sidebar = self.sidebar.read(cx);
        let sidebar_width = sidebar.expanded_width();
        let collapsed = sidebar.is_collapsed();
        let _ = sidebar;

        let content = div()
            .id("main-content")
            .flex_1()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .px(px(Spacing::CONTENT_PADDING_X))
            .py(px(Spacing::CONTENT_PADDING_Y))
            .child(self.render_content(view_model));

        if collapsed {
            h_flex()
                .size_full()
                .child(
                    div()
                        .w(px(Spacing::SIDEBAR_COLLAPSED_WIDTH))
                        .h_full()
                        .flex_shrink_0()
                        .child(self.sidebar.clone()),
                )
                .child(content)
                .into_any_element()
        } else {
            let sidebar_entity = self.sidebar.clone();
            h_resizable("main-window-layout")
                .with_state(&self.sidebar_layout)
                .on_resize(move |state, _, cx| {
                    let width = state
                        .read(cx)
                        .sizes()
                        .first()
                        .copied()
                        .unwrap_or(px(Spacing::SIDEBAR_WIDTH));
                    let _ = sidebar_entity.update(cx, |sidebar, cx| {
                        sidebar.set_expanded_width(f32::from(width));
                        cx.notify();
                    });
                })
                .child(
                    resizable_panel()
                        .size(px(sidebar_width))
                        .size_range(px(SIDEBAR_MIN_WIDTH)..px(SIDEBAR_MAX_WIDTH))
                        .child(self.sidebar.clone()),
                )
                .child(resizable_panel().child(content))
                .into_any_element()
        }
    }

    fn render_header(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        if cfg!(target_os = "macos") {
            WindowHeader::empty().into_any_element()
        } else {
            WindowHeader::empty()
                .leading(self.menu_bar.clone())
                .into_any_element()
        }
    }

    fn render_content(&self, view_model: MainWindowViewModel) -> impl IntoElement {
        match view_model.content {
            MainContentViewModel::History => div()
                .id("history-scroll")
                .size_full()
                .min_h_0()
                .overflow_y_scroll()
                .child(self.history_view.clone())
                .into_any_element(),
            MainContentViewModel::Downloads(summary) => {
                self.render_transfers(summary).into_any_element()
            }
        }
    }

    fn render_transfers(&self, summary: TransfersSummaryViewModel) -> impl IntoElement {
        v_resizable("transfers-layout")
            .child(
                resizable_panel()
                    .size(px(TRANSFERS_TOP_PANEL_DEFAULT_HEIGHT))
                    .size_range(
                        px(TRANSFERS_TOP_PANEL_MIN_HEIGHT)..px(TRANSFERS_TOP_PANEL_MAX_HEIGHT),
                    )
                    .child(self.render_transfers_summary(summary)),
            )
            .child(
                resizable_panel()
                    .size_range(px(TRANSFERS_BOTTOM_PANEL_MIN_HEIGHT)..Pixels::MAX)
                    .child(
                        div()
                            .id("transfers-list-scroll")
                            .size_full()
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(self.transfer_list.clone()),
                    ),
            )
    }

    fn render_transfers_summary(&self, summary: TransfersSummaryViewModel) -> impl IntoElement {
        h_resizable("transfers-top-layout")
            .child(
                resizable_panel()
                    .size(px(TRANSFERS_STATS_PANEL_DEFAULT_WIDTH))
                    .size_range(px(TRANSFERS_STATS_PANEL_MIN_WIDTH)..Pixels::MAX)
                    .child(transfers_summary_panel(StatsBar {
                        download_samples: summary.stats.download_samples,
                        download_speed: summary.stats.download_speed,
                        disk_read_speed: summary.stats.disk_read_speed,
                        disk_write_speed: summary.stats.disk_write_speed,
                        active_count: summary.stats.active_count,
                        finished_count: summary.stats.finished_count,
                        queued_count: summary.stats.queued_count,
                    })),
            )
            .child(
                resizable_panel()
                    .size(px(TRANSFERS_CHUNK_PANEL_DEFAULT_WIDTH))
                    .size_range(px(TRANSFERS_CHUNK_PANEL_MIN_WIDTH)..Pixels::MAX)
                    .child(transfers_summary_panel(ChunkMapCard::new(
                        summary.chunk_map,
                    ))),
            )
    }
}

fn transfers_summary_panel(content: impl IntoElement) -> impl IntoElement {
    div()
        .size_full()
        .min_w_0()
        .min_h_0()
        .rounded(px(Chrome::PANEL_RADIUS))
        .border_1()
        .border_color(Colors::border())
        .overflow_hidden()
        .child(
            div()
                .size_full()
                .min_w_0()
                .min_h_0()
                .p(px(Chrome::STATS_CARD_PADDING))
                .child(content),
        )
}

struct MainWindowViewModel {
    content: MainContentViewModel,
}

enum MainContentViewModel {
    History,
    Downloads(TransfersSummaryViewModel),
}

struct StatsBarViewModel {
    download_samples: Vec<f32>,
    download_speed: f32,
    disk_read_speed: Option<f32>,
    disk_write_speed: Option<f32>,
    active_count: usize,
    finished_count: usize,
    queued_count: usize,
}

struct TransfersSummaryViewModel {
    stats: StatsBarViewModel,
    chunk_map: ChunkMapCardModel,
}
