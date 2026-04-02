use gpui::{div, prelude::*, px, Context, PathPromptOptions};

use crate::settings::Settings;
use crate::ui::prelude::*;

use super::SettingsWindow;

pub(super) fn render(settings: &Settings, cx: &mut Context<SettingsWindow>) -> gpui::Div {
    let dir: gpui::SharedString =
        settings.download_dir().to_string_lossy().to_string().into();

    let folder_btn = div()
        .id("folder-picker")
        .w(px(28.0))
        .h(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .border_1()
        .border_color(Colors::border())
        .cursor_pointer()
        .on_click(cx.listener(|_this, _, _, cx| {
            let receiver = cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: None,
            });
            cx.spawn(async move |entity, cx: &mut gpui::AsyncApp| {
                if let Ok(Ok(Some(paths))) = receiver.await {
                    if let Some(path) = paths.into_iter().next() {
                        cx.update(move |app| {
                            entity.update(app, |this, cx| {
                                this.settings.default_download_dir = Some(path);
                                cx.notify();
                            })
                            .ok();
                        })
                        .ok();
                    }
                }
            })
            .detach();
        }))
        .child(icon_sm(IconName::Folder, Colors::muted_foreground()));

    div()
        .flex_col()
        .gap(px(20.0))
        .child(super::setting_row(
            "Download Folder",
            "Where files are saved when no destination is specified",
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .px(px(12.0))
                        .py(px(7.0))
                        .max_w(px(200.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(Colors::border())
                        .text_sm()
                        .text_color(Colors::muted_foreground())
                        .overflow_hidden()
                        .child(dir),
                )
                .child(folder_btn),
        ))
}
