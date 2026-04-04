use gpui::{Context, PathPromptOptions, div, prelude::*, px};
use rust_i18n::t;

use crate::ui::prelude::*;

use super::SettingsWindow;

pub(super) fn render(this: &SettingsWindow, cx: &mut Context<SettingsWindow>) -> gpui::Div {
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
                if let Ok(Ok(Some(paths))) = receiver.await
                    && let Some(path) = paths.into_iter().next()
                {
                    cx.update(move |app| {
                        entity
                            .update(app, |this, cx| {
                                this.download_dir_input.update(cx, |input, cx| {
                                    input.set_text(path.to_string_lossy().to_string(), cx);
                                });
                            })
                            .ok();
                    })
                    .ok();
                }
            })
            .detach();
        }))
        .child(icon_sm(IconName::Folder, Colors::muted_foreground()));

    div().flex_col().gap(px(20.0)).child(super::setting_row(
        t!("settings.general.download_folder_label").to_string(),
        t!("settings.general.download_folder_description").to_string(),
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(super::setting_text_input(this.download_dir_input.clone()))
            .child(folder_btn),
    ))
}
