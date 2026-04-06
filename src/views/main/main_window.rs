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
    App, Context, Entity, Pixels, StatefulInteractiveElement as _, Window, div, prelude::*, px,
};

use crate::app::Downloads;
use crate::app_actions;
use crate::app_menu;
use crate::settings::Settings;
use crate::theme::{APP_FONT_FAMILY, Spacing};
use crate::ui::prelude::*;
use crate::views::overlays::about_modal::AboutLayer;
use crate::views::overlays::download_modal::DownloadModalLayer;

use super::chunk_bitmap::ChunkBitmapCard;
use super::download_list::DownloadList;
use super::history::HistoryView;
use super::sidebar::Sidebar;
use super::stats_bar::StatsBar;

const HISTORY_NAV_INDEX: usize = 1;
const SIDEBAR_MIN_WIDTH: f32 = 200.0;
const SIDEBAR_MAX_WIDTH: f32 = 320.0;
const TRANSFERS_TOP_PANEL_DEFAULT_HEIGHT: f32 = 220.0;
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
    download_list: Entity<DownloadList>,
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
        let download_list = cx.new(|cx| DownloadList::new(downloads.clone(), cx));
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

        Self {
            menu_bar,
            sidebar,
            sidebar_layout,
            downloads,
            download_list,
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
            let (active, finished, queued) = downloads.status_counts();

            MainContentViewModel::Downloads(StatsBarViewModel {
                download_samples: downloads.speed_samples_mbs(),
                upload_samples: Vec::new(),
                download_speed: downloads.download_speed_bps() as f32 / 1_000_000.0,
                upload_speed: 0.0,
                active_count: active,
                finished_count: finished,
                queued_count: queued,
            })
        };

        MainWindowViewModel { content }
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
            MainContentViewModel::Downloads(stats) => {
                self.render_transfers(stats).into_any_element()
            }
        }
    }

    fn render_transfers(&self, stats: StatsBarViewModel) -> impl IntoElement {
        v_resizable("transfers-layout")
            .child(
                resizable_panel()
                    .size(px(TRANSFERS_TOP_PANEL_DEFAULT_HEIGHT))
                    .size_range(
                        px(TRANSFERS_TOP_PANEL_MIN_HEIGHT)..px(TRANSFERS_TOP_PANEL_MAX_HEIGHT),
                    )
                    .child(self.render_transfers_summary(stats)),
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
                            .child(self.download_list.clone()),
                    ),
            )
    }

    fn render_transfers_summary(&self, stats: StatsBarViewModel) -> impl IntoElement {
        h_resizable("transfers-top-layout")
            .child(
                resizable_panel()
                    .size(px(TRANSFERS_STATS_PANEL_DEFAULT_WIDTH))
                    .size_range(px(TRANSFERS_STATS_PANEL_MIN_WIDTH)..Pixels::MAX)
                    .child(StatsBar {
                        download_samples: stats.download_samples,
                        upload_samples: stats.upload_samples,
                        download_speed: stats.download_speed,
                        upload_speed: stats.upload_speed,
                        active_count: stats.active_count,
                        finished_count: stats.finished_count,
                        queued_count: stats.queued_count,
                    }),
            )
            .child(
                resizable_panel()
                    .size(px(TRANSFERS_CHUNK_PANEL_DEFAULT_WIDTH))
                    .size_range(px(TRANSFERS_CHUNK_PANEL_MIN_WIDTH)..Pixels::MAX)
                    .child(ChunkBitmapCard),
            )
    }
}

struct MainWindowViewModel {
    content: MainContentViewModel,
}

enum MainContentViewModel {
    History,
    Downloads(StatsBarViewModel),
}

struct StatsBarViewModel {
    download_samples: Vec<f32>,
    upload_samples: Vec<f32>,
    download_speed: f32,
    upload_speed: f32,
    active_count: usize,
    finished_count: usize,
    queued_count: usize,
}
