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

//! Directory input built using the same integrated-control pattern as the
//! `NumberInput` component in
//! [longbridge/gpui-component](https://github.com/longbridge/gpui-component),
//! which is Apache-2.0 licensed. Ophelia keeps a local copy so it can tailor
//! behavior and styling without taking a direct dependency on that component
//! library.

use gpui::{
    Context, Entity, IntoElement, PathPromptOptions, Render, SharedString, Window, div, prelude::*,
    px,
};

use crate::ui::prelude::*;

pub struct DirectoryInput {
    input: Entity<TextField>,
}

impl DirectoryInput {
    pub fn new(
        initial_value: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextField::embedded(initial_value, placeholder, cx));
        Self { input }
    }

    pub fn text<T>(&self, cx: &Context<T>) -> SharedString {
        self.input
            .read_with(cx, |input, _| input.text().to_string().into())
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.set_text(text, cx);
        });
        cx.notify();
    }

    fn pick_directory(&mut self, cx: &mut Context<Self>) {
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
                let path_text = path.to_string_lossy().to_string();
                cx.update(move |app| {
                    entity
                        .update(app, |this, cx| {
                            this.set_text(path_text, cx);
                        })
                        .ok();
                })
                .ok();
            }
        })
        .detach();
    }
}

impl Render for DirectoryInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.input.read(cx).is_focused(window);

        h_flex()
            .id(("directory-input", cx.entity_id()))
            .items_center()
            .overflow_hidden()
            .rounded(px(Chrome::BUTTON_RADIUS))
            .border_1()
            .border_color(if focused {
                Colors::ring()
            } else {
                Colors::input_border()
            })
            .bg(Colors::background())
            .child(div().flex_1().min_w_0().child(self.input.clone()))
            .child(
                div()
                    .id("directory-picker")
                    .w(px(Chrome::SIDEBAR_BUTTON_SIZE))
                    .h(px(Chrome::SIDEBAR_BUTTON_SIZE))
                    .rounded_r(px(Chrome::BUTTON_RADIUS))
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_l_1()
                    .border_color(Colors::input_border())
                    .bg(Colors::background())
                    .cursor_pointer()
                    .hover(|style: gpui::StyleRefinement| style.bg(Colors::muted()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pick_directory(cx);
                    }))
                    .child(icon_sm(IconName::Folder, Colors::muted_foreground())),
            )
    }
}
