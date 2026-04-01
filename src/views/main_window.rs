use gpui::{div, prelude::*, px, Context, Entity, Window};

use crate::app::Downloads;
use crate::engine::http::HttpDownloadConfig;
use crate::platform;
use crate::theme::Spacing;
use crate::ui::prelude::*;
use crate::views::download_list::DownloadList;
use crate::views::download_modal::{DownloadCancelled, DownloadConfirmed, DownloadModal};
use crate::views::sidebar::{AddDownloadClicked, Sidebar};
use crate::views::stats_bar::StatsBar;

/// Root view
/// owns the full window layout and all live state.
pub struct MainWindow {
    sidebar: Entity<Sidebar>,
    downloads: Entity<Downloads>,
    download_list: Entity<DownloadList>,
    modal: Option<Entity<DownloadModal>>,
}

impl MainWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let sidebar = cx.new(|_| Sidebar {
            active_item: 0,
            collapsed: false,
            storage_used_bytes: 0,
            storage_total_bytes: 0,
        });

        let downloads = cx.new(|cx| Downloads::new(cx));
        let download_list = cx.new(|cx| DownloadList::new(downloads.clone(), cx));

        // Open the modal when the sidebar Add button is clicked.
        cx.subscribe(&sidebar, |this: &mut Self, _, _: &AddDownloadClicked, cx| {
            this.open_modal(cx);
        })
        .detach();

        Self {
            sidebar,
            downloads,
            download_list,
            modal: None,
        }
    }

    fn open_modal(&mut self, cx: &mut Context<Self>) {
        let modal = cx.new(|cx| DownloadModal::new(cx));

        cx.subscribe(&modal, |this: &mut Self, _, event: &DownloadConfirmed, cx| {
            let url = event.url.clone();
            let destination = event.destination.clone();
            this.downloads.update(cx, |d, cx| {
                d.add(url, destination, HttpDownloadConfig::default(), cx);
            });
            this.modal = None;
            cx.notify();
        })
        .detach();

        cx.subscribe(&modal, |this: &mut Self, _, _: &DownloadCancelled, cx| {
            this.modal = None;
            cx.notify();
        })
        .detach();

        self.modal = Some(modal);
        cx.notify();
    }
}

impl Render for MainWindow {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(Colors::background())
            .text_color(Colors::foreground())
            .font_family("Inter")
            // Full-width titlebar
            .child(
                div()
                    .h(px(platform::TITLEBAR_HEIGHT))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .px(px(24.0))
                    .border_b_1()
                    .border_color(Colors::border())
                    .child(icon_sm(IconName::Settings, Colors::muted_foreground())),
            )
            // Sidebar + content below
            .child(
                div()
                    .flex()
                    .flex_1()
                    .overflow_hidden()
                    .child(self.sidebar.clone())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .overflow_hidden()
                            .child(
                                div()
                                    .id("main-content")
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .overflow_y_scroll()
                                    .px(px(Spacing::CONTENT_PADDING_X))
                                    .py(px(Spacing::CONTENT_PADDING_Y))
                                    .child(StatsBar::new())
                                    .child(self.download_list.clone()),
                            ),
                    ),
            )
            // Modal overlay (rendered last so it sits on top)
            .when_some(self.modal.clone(), |el, modal| el.child(modal))
    }
}
