//! Add Download modal overlay.

use std::path::PathBuf;

use gpui::{Context, Entity, EventEmitter, IntoElement, Render, Window, div, prelude::*, px};
use rust_i18n::t;

use crate::app::Downloads;
use crate::engine::AddDownloadRequest;
use crate::ui::prelude::*;

pub struct DownloadConfirmed {
    pub url: String,
    pub destination: PathBuf,
}

pub struct DownloadCancelled;

pub struct DownloadModal {
    url_input: Entity<TextField>,
    destination_input: Entity<TextField>,
    destination_edited: bool,
}

impl EventEmitter<DownloadConfirmed> for DownloadModal {}
impl EventEmitter<DownloadCancelled> for DownloadModal {}

impl DownloadModal {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let url = Self::clipboard_url(cx).unwrap_or_default();
        let destination = if url.is_empty() {
            String::new()
        } else {
            Self::destination_for(&url).to_string_lossy().to_string()
        };

        let url_input = cx.new(|cx| {
            TextField::new(
                url.clone(),
                t!("download_modal.url_placeholder").to_string(),
                cx,
            )
        });
        let destination_input = cx.new(|cx| {
            TextField::new(
                destination,
                t!("download_modal.destination_placeholder").to_string(),
                cx,
            )
        });

        cx.subscribe(
            &url_input,
            |this: &mut Self, _, _: &TextFieldChanged, cx| {
                if this.destination_edited {
                    cx.notify();
                    return;
                }

                let next_destination = {
                    let url = this.url_input.read(cx).text().trim().to_string();
                    if url.is_empty() {
                        String::new()
                    } else {
                        Self::destination_for(&url).to_string_lossy().to_string()
                    }
                };

                this.destination_input.update(cx, |input, cx| {
                    input.set_text(next_destination, cx);
                });
                cx.notify();
            },
        )
        .detach();

        cx.subscribe(
            &destination_input,
            |this: &mut Self, _, event: &TextFieldChanged, cx| {
                let auto_destination = {
                    let url = this.url_input.read(cx).text().trim().to_string();
                    if url.is_empty() {
                        String::new()
                    } else {
                        Self::destination_for(&url).to_string_lossy().to_string()
                    }
                };

                this.destination_edited = event.text.as_ref() != auto_destination;
                cx.notify();
            },
        )
        .detach();

        cx.subscribe(
            &url_input,
            |this: &mut Self, _, _: &TextFieldSubmitted, cx| {
                this.confirm_if_valid(cx);
            },
        )
        .detach();

        cx.subscribe(
            &destination_input,
            |this: &mut Self, _, _: &TextFieldSubmitted, cx| {
                this.confirm_if_valid(cx);
            },
        )
        .detach();

        Self {
            url_input,
            destination_input,
            destination_edited: false,
        }
    }

    fn clipboard_url(cx: &mut Context<Self>) -> Option<String> {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .map(|text| text.trim().to_string())
            .filter(|text| Self::is_valid_url(text))
    }

    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = Self::clipboard_url(cx) {
            self.url_input
                .update(cx, |input, cx| input.set_text(url, cx));
        }
        cx.notify();
    }

    fn destination_for(url: &str) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        AddDownloadRequest::from_url(url.to_string())
            .destination_in(&PathBuf::from(home).join("Downloads"))
    }

    fn is_valid_url(url: &str) -> bool {
        let url = url.trim();
        url.starts_with("http://") || url.starts_with("https://")
    }

    fn form_values(&self, cx: &mut Context<Self>) -> (String, String) {
        (
            self.url_input.read(cx).text().trim().to_string(),
            self.destination_input.read(cx).text().trim().to_string(),
        )
    }

    fn can_confirm(&self, cx: &mut Context<Self>) -> bool {
        let (url, destination) = self.form_values(cx);
        Self::is_valid_url(&url) && !destination.is_empty()
    }

    fn confirm_if_valid(&mut self, cx: &mut Context<Self>) {
        let (url, destination) = self.form_values(cx);
        if Self::is_valid_url(&url) && !destination.is_empty() {
            cx.emit(DownloadConfirmed {
                url,
                destination: PathBuf::from(destination),
            });
        }
    }
}

pub struct DownloadModalLayer {
    show: Entity<bool>,
    downloads: Entity<Downloads>,
    modal: Option<Entity<DownloadModal>>,
}

impl DownloadModalLayer {
    pub fn new(downloads: Entity<Downloads>, show: Entity<bool>, cx: &mut Context<Self>) -> Self {
        cx.observe(&show, |this, show, cx| {
            if *show.read(cx) {
                if this.modal.is_none() {
                    this.mount_modal(cx);
                }
            } else if this.modal.is_some() {
                this.modal = None;
                cx.notify();
            }
        })
        .detach();

        Self {
            show,
            downloads,
            modal: None,
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.modal = None;
        self.show.update(cx, |show, cx| {
            *show = false;
            cx.notify();
        });
        cx.notify();
    }

    fn mount_modal(&mut self, cx: &mut Context<Self>) {
        let modal = cx.new(|cx| DownloadModal::new(cx));

        cx.subscribe(
            &modal,
            |this: &mut Self, _, event: &DownloadConfirmed, cx| {
                let url = event.url.clone();
                let destination = event.destination.clone();
                this.downloads.update(cx, |downloads, cx| {
                    downloads.add(url, destination, cx);
                });
                this.close(cx);
            },
        )
        .detach();

        cx.subscribe(&modal, |this: &mut Self, _, _: &DownloadCancelled, cx| {
            this.close(cx);
        })
        .detach();

        self.modal = Some(modal);
        cx.notify();
    }
}

impl Render for DownloadModalLayer {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.modal
            .clone()
            .map(IntoElement::into_any_element)
            .unwrap_or_else(|| div().into_any_element())
    }
}

impl Render for DownloadModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_confirm = self.can_confirm(cx);
        let weak = cx.weak_entity();

        modal()
            .on_exit(move |_, cx| {
                let _ = weak.update(cx, |_, cx| {
                    cx.emit(DownloadCancelled);
                });
            })
            .child(
                div()
                    .w(px(Chrome::DOWNLOAD_MODAL_WIDTH))
                    .p(px(Chrome::MODAL_PADDING))
                    .flex()
                    .flex_col()
                    .gap(px(Chrome::MODAL_STACK_GAP))
                    .child(
                        div()
                            .text_xl()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(Colors::foreground())
                            .child(t!("download_modal.title").to_string()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(Spacing::LIST_GAP))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(Colors::muted_foreground())
                                    .child(t!("download_modal.url_label").to_string()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(px(Spacing::LIST_GAP))
                                    .child(div().flex_1().child(self.url_input.clone()))
                                    .child(
                                        div()
                                            .id("paste-btn")
                                            .flex()
                                            .items_center()
                                            .px(px(12.0))
                                            .py(px(10.0))
                                            .rounded(px(Chrome::BUTTON_RADIUS))
                                            .border_1()
                                            .border_color(Colors::border())
                                            .bg(Colors::background())
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(Colors::foreground())
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.paste_from_clipboard(cx);
                                            }))
                                            .child(t!("download_modal.paste").to_string()),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(Spacing::LIST_GAP))
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(Colors::muted_foreground())
                                    .child(t!("download_modal.destination_label").to_string()),
                            )
                            .child(self.destination_input.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(Spacing::CONTROL_GAP))
                            .child(
                                div()
                                    .id("cancel-btn")
                                    .px(px(18.0))
                                    .py(px(10.0))
                                    .rounded(px(Chrome::BUTTON_RADIUS))
                                    .border_1()
                                    .border_color(Colors::border())
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(Colors::foreground())
                                    .cursor_pointer()
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(DownloadCancelled);
                                    }))
                                    .child(t!("download_modal.cancel").to_string()),
                            )
                            .child(
                                div()
                                    .id("confirm-btn")
                                    .px(px(18.0))
                                    .py(px(10.0))
                                    .rounded(px(Chrome::BUTTON_RADIUS))
                                    .bg(if can_confirm {
                                        Colors::active()
                                    } else {
                                        Colors::muted()
                                    })
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(Colors::background())
                                    .cursor_pointer()
                                    .when(can_confirm, |el| {
                                        el.on_click(cx.listener(|this, _, _, cx| {
                                            this.confirm_if_valid(cx);
                                        }))
                                    })
                                    .child(t!("download_modal.confirm").to_string()),
                            ),
                    ),
            )
    }
}
