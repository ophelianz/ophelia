use gpui::{Context, Entity, Window, div, prelude::*, px};

use crate::app::Downloads;
use crate::app_menu;
use crate::engine::http::HttpDownloadConfig;
use crate::settings::Settings;
use crate::theme::{APP_FONT_FAMILY, Spacing};
use crate::ui::prelude::*;
use crate::views::download_list::DownloadList;
use crate::views::download_modal::{DownloadCancelled, DownloadConfirmed, DownloadModal};
use crate::views::history::HistoryView;
use crate::views::sidebar::{AddDownloadClicked, Sidebar};
use crate::views::stats_bar::StatsBar;

const HISTORY_NAV_INDEX: usize = 4;

/// Root view
/// owns the full window layout and all live state.
pub struct MainWindow {
    menu_bar: Entity<AppMenuBar>,
    sidebar: Entity<Sidebar>,
    downloads: Entity<Downloads>,
    download_list: Entity<DownloadList>,
    history_view: Entity<HistoryView>,
    modal: Option<Entity<DownloadModal>>,
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

        // Re-render when sidebar nav changes (to switch content pane).
        cx.observe(&sidebar, |_, _, cx| cx.notify()).detach();

        // Open the modal when the sidebar Add button is clicked.
        cx.subscribe(
            &sidebar,
            |this: &mut Self, _, _: &AddDownloadClicked, cx| {
                this.open_modal(cx);
            },
        )
        .detach();

        Self {
            menu_bar,
            sidebar,
            downloads,
            download_list,
            history_view,
            modal: None,
        }
    }

    pub(crate) fn open_modal(&mut self, cx: &mut Context<Self>) {
        let modal = cx.new(|cx| DownloadModal::new(cx));

        cx.subscribe(
            &modal,
            |this: &mut Self, _, event: &DownloadConfirmed, cx| {
                let url = event.url.clone();
                let destination = event.destination.clone();
                this.downloads.update(cx, |d, cx| {
                    d.add(url, destination, HttpDownloadConfig::default(), cx);
                });
                this.modal = None;
                cx.notify();
            },
        )
        .detach();

        cx.subscribe(&modal, |this: &mut Self, _, _: &DownloadCancelled, cx| {
            this.modal = None;
            cx.notify();
        })
        .detach();

        self.modal = Some(modal);
        cx.notify();
    }

    pub(crate) fn apply_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        self.downloads.update(cx, |downloads, _| {
            downloads.settings = settings;
        });
        cx.notify();
    }
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_nav = self.sidebar.read(cx).active_item;

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
                                .child(self.render_content(active_nav, cx)),
                        ),
                    ),
            )
            // Download modal rendered last so it sits on top.
            .when_some(self.modal.clone(), |el, modal| el.child(modal))
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

    fn render_content(&self, active_nav: usize, cx: &mut Context<Self>) -> impl IntoElement {
        if active_nav == HISTORY_NAV_INDEX {
            return self.history_view.clone().into_any_element();
        }

        let downloads = self.downloads.read(cx);
        let (active, finished, queued) = downloads.status_counts();

        v_flex()
            .gap(px(Spacing::CARD_GAP))
            .child(StatsBar {
                download_samples: downloads.speed_samples_mbs(),
                upload_samples: Vec::new(),
                download_speed: downloads.download_speed_bps() as f32 / 1_000_000.0,
                upload_speed: 0.0,
                active_count: active,
                finished_count: finished,
                queued_count: queued,
            })
            .child(self.download_list.clone())
            .into_any_element()
    }
}
