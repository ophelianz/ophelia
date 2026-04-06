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

//! Add Download modal overlay.

use std::path::PathBuf;

use gpui::{Context, Entity, EventEmitter, IntoElement, Render, Window, div, prelude::*, px};
use rust_i18n::t;

use crate::app::Downloads;
use crate::engine::AddDownloadRequest;
use crate::settings::Settings;
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
    settings: Settings,
}

impl EventEmitter<DownloadConfirmed> for DownloadModal {}
impl EventEmitter<DownloadCancelled> for DownloadModal {}

impl DownloadModal {
    pub fn new(settings: Settings, cx: &mut Context<Self>) -> Self {
        let url = Self::clipboard_source(&settings, cx).unwrap_or_default();
        let destination = Self::preview_destination_for(&url, &settings)
            .map(|destination| destination.to_string_lossy().to_string())
            .unwrap_or_default();

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
                    Self::preview_destination_for(&url, &this.settings)
                        .map(|destination| destination.to_string_lossy().to_string())
                        .unwrap_or_default()
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
                    Self::preview_destination_for(&url, &this.settings)
                        .map(|destination| destination.to_string_lossy().to_string())
                        .unwrap_or_default()
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
            settings,
        }
    }

    fn clipboard_source(settings: &Settings, cx: &mut Context<Self>) -> Option<String> {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .map(|text| text.trim().to_string())
            .filter(|text| Self::preview_destination_for(text, settings).is_some())
    }

    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = Self::clipboard_source(&self.settings, cx) {
            self.url_input
                .update(cx, |input, cx| input.set_text(url, cx));
        }
        cx.notify();
    }

    fn preview_destination_for(url: &str, settings: &Settings) -> Option<PathBuf> {
        let url = url.trim();
        if url.is_empty() {
            return None;
        }
        Some(AddDownloadRequest::from_url(url.to_string()).preview_destination(settings))
    }

    fn form_values(&self, cx: &mut Context<Self>) -> (String, String) {
        (
            self.url_input.read(cx).text().trim().to_string(),
            self.destination_input.read(cx).text().trim().to_string(),
        )
    }

    fn can_confirm(&self, cx: &mut Context<Self>) -> bool {
        let (url, destination) = self.form_values(cx);
        !url.is_empty() && !destination.is_empty()
    }

    fn confirm_if_valid(&mut self, cx: &mut Context<Self>) {
        let (url, destination) = self.form_values(cx);
        if !url.is_empty() && !destination.is_empty() {
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
        let settings = self.downloads.read(cx).settings.clone();
        let modal = cx.new(|cx| DownloadModal::new(settings, cx));

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
                                        Button::new(
                                            "paste-btn",
                                            t!("download_modal.paste").to_string(),
                                        )
                                        .compact()
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.paste_from_clipboard(cx);
                                            }),
                                        ),
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
                                Button::new("cancel-btn", t!("download_modal.cancel").to_string())
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(DownloadCancelled);
                                    })),
                            )
                            .child(
                                Button::new(
                                    "confirm-btn",
                                    t!("download_modal.confirm").to_string(),
                                )
                                .primary()
                                .disabled(!can_confirm)
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.confirm_if_valid(cx);
                                    },
                                )),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gpui::{
        Bounds, ClipboardItem, Context, Entity, Focusable, MouseButton, Render, TestApp, Window,
        WindowBounds, WindowOptions, point, px,
    };

    use super::*;

    struct DownloadModalHost {
        modal: Entity<DownloadModal>,
        confirmed: Option<(String, PathBuf)>,
        cancelled: usize,
    }

    impl DownloadModalHost {
        fn new(settings: Settings, _window: &mut Window, cx: &mut Context<Self>) -> Self {
            let modal = cx.new(|cx| DownloadModal::new(settings, cx));

            cx.subscribe(
                &modal,
                |this: &mut Self, _, event: &DownloadConfirmed, cx| {
                    this.confirmed = Some((event.url.clone(), event.destination.clone()));
                    cx.notify();
                },
            )
            .detach();

            cx.subscribe(&modal, |this: &mut Self, _, _: &DownloadCancelled, cx| {
                this.cancelled += 1;
                cx.notify();
            })
            .detach();

            Self {
                modal,
                confirmed: None,
                cancelled: 0,
            }
        }
    }

    impl Render for DownloadModalHost {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.modal.clone())
        }
    }

    fn test_settings() -> Settings {
        let mut settings = Settings::default();
        settings.default_download_dir = Some(std::env::temp_dir().join("ophelia-modal-tests"));
        settings.destination_rules_enabled = false;
        settings.destination_rules.clear();
        settings
    }

    fn open_host(app: &mut TestApp, settings: Settings) -> gpui::TestAppWindow<DownloadModalHost> {
        app.update(|cx| {
            crate::ui::chrome::modal::bind_actions(cx);
            crate::ui::controls::text_field::init(cx);
        });

        let bounds = Bounds::from_corners(point(px(0.0), px(0.0)), point(px(800.0), px(600.0)));
        let mut window = app.open_window_with_options(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| DownloadModalHost::new(settings.clone(), window, cx),
        );
        window.draw();
        window
    }

    fn focus_url_input(window: &mut gpui::TestAppWindow<DownloadModalHost>) {
        window.update(|host, window, cx| {
            host.modal.update(cx, |modal, cx| {
                modal.url_input.focus_handle(cx).focus(window, cx);
            });
        });
    }

    fn focus_destination_input(window: &mut gpui::TestAppWindow<DownloadModalHost>) {
        window.update(|host, window, cx| {
            host.modal.update(cx, |modal, cx| {
                modal.destination_input.focus_handle(cx).focus(window, cx);
            });
        });
    }

    #[test]
    fn typing_a_source_updates_preview_and_user_editing_preserves_destination() {
        let settings = test_settings();
        let expected_initial = settings
            .download_dir()
            .join("music.mp3")
            .to_string_lossy()
            .to_string();

        let mut app = TestApp::new();
        let mut window = open_host(&mut app, settings.clone());

        focus_url_input(&mut window);
        window.simulate_input("https://example.com/music.mp3");

        window.read(|host, app| {
            let modal = host.modal.read(app);
            assert_eq!(modal.destination_input.read(app).text(), expected_initial);
        });

        focus_destination_input(&mut window);
        window.simulate_keystrokes("cmd-a backspace");
        window.simulate_input("custom.mp3");

        focus_url_input(&mut window);
        window.simulate_keystrokes("cmd-a backspace");
        window.simulate_input("https://example.com/video.mp4");

        window.read(|host, app| {
            let modal = host.modal.read(app);
            assert_eq!(modal.destination_input.read(app).text(), "custom.mp3");
            assert!(modal.destination_edited);
        });
    }

    #[test]
    fn enter_submits_only_when_the_form_is_valid() {
        let settings = test_settings();
        let expected_destination = settings
            .download_dir()
            .join("file.bin")
            .to_string_lossy()
            .to_string();

        let mut app = TestApp::new();
        let mut window = open_host(&mut app, settings);

        focus_url_input(&mut window);
        window.simulate_keystrokes("enter");

        window.read(|host, _| {
            assert!(host.confirmed.is_none());
        });

        window.simulate_input("https://example.com/file.bin");
        window.simulate_keystrokes("enter");

        window.read(|host, _| {
            assert_eq!(
                host.confirmed,
                Some((
                    "https://example.com/file.bin".to_string(),
                    PathBuf::from(expected_destination),
                ))
            );
        });
    }

    #[test]
    fn clicking_the_backdrop_emits_cancel() {
        let mut app = TestApp::new();
        let mut window = open_host(&mut app, test_settings());

        window.simulate_click(point(px(10.0), px(10.0)), MouseButton::Left);

        window.read(|host, _| {
            assert_eq!(host.cancelled, 1);
        });
    }

    #[test]
    fn paste_button_reads_from_the_test_clipboard() {
        let mut app = TestApp::new();
        app.write_to_clipboard(ClipboardItem::new_string(
            "https://example.com/clipboard.bin".to_string(),
        ));
        let mut window = open_host(&mut app, test_settings());

        window.simulate_click(point(px(600.0), px(266.0)), MouseButton::Left);

        window.read(|host, app| {
            let modal = host.modal.read(app);
            assert_eq!(
                modal.url_input.read(app).text(),
                "https://example.com/clipboard.bin"
            );
        });
    }
}
