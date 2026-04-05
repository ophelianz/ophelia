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

use gpui::{App, Context, Entity, Window, div, prelude::*, px};

use crate::app::Downloads;
use crate::app_actions;
use crate::app_menu;
use crate::settings::Settings;
use crate::theme::{APP_FONT_FAMILY, Spacing};
use crate::ui::prelude::*;
use crate::views::overlays::about_modal::AboutLayer;
use crate::views::overlays::download_modal::DownloadModalLayer;

use super::download_list::DownloadList;
use super::history::HistoryView;
use super::sidebar::Sidebar;
use super::stats_bar::StatsBar;

const HISTORY_NAV_INDEX: usize = 1;

/// Root view
/// owns the full window layout and all live state.
pub struct MainWindow {
    menu_bar: Entity<AppMenuBar>,
    sidebar: Entity<Sidebar>,
    downloads: Entity<Downloads>,
    download_list: Entity<DownloadList>,
    history_view: Entity<HistoryView>,
    about_modal: Entity<AboutLayer>,
    download_modal: Entity<DownloadModalLayer>,
}

impl MainWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let menu_bar = cx.new(|cx| AppMenuBar::new(app_menu::build_owned_menus(), cx));
        let sidebar = cx.new(|_| Sidebar {
            active_item: 0,
            collapsed: false,
            download_dir: Settings::load().download_dir(),
        });

        let downloads = cx.new(|cx| Downloads::new(cx));
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
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.download_dir = settings.download_dir();
            cx.notify();
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
                    .child(self.sidebar.clone())
                    .child(
                        div().flex().flex_col().flex_1().overflow_hidden().child(
                            div()
                                .id("main-content")
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(px(Spacing::CARD_GAP))
                                .overflow_y_scroll()
                                .px(px(Spacing::CONTENT_PADDING_X))
                                .py(px(Spacing::CONTENT_PADDING_Y))
                                .child(self.render_content(view_model, cx)),
                        ),
                    ),
            )
            .child(self.download_modal.clone())
            .child(self.about_modal.clone())
    }
}

impl MainWindow {
    fn render_header(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        if cfg!(target_os = "macos") {
            WindowHeader::empty().into_any_element()
        } else {
            WindowHeader::empty()
                .leading(self.menu_bar.clone())
                .into_any_element()
        }
    }

    fn render_content(
        &self,
        view_model: MainWindowViewModel,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match view_model.content {
            MainContentViewModel::History => self.history_view.clone().into_any_element(),
            MainContentViewModel::Downloads(stats) => v_flex()
                .gap(px(Spacing::CARD_GAP))
                .child(StatsBar {
                    download_samples: stats.download_samples,
                    upload_samples: stats.upload_samples,
                    download_speed: stats.download_speed,
                    upload_speed: stats.upload_speed,
                    active_count: stats.active_count,
                    finished_count: stats.finished_count,
                    queued_count: stats.queued_count,
                })
                .child(self.download_list.clone())
                .into_any_element(),
        }
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
