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

//! Number input adapted from the `NumberInput` component in
//! [longbridge/gpui-component](https://github.com/longbridge/gpui-component),
//! which is Apache-2.0 licensed. Ophelia keeps a local copy so it can tailor
//! behavior and styling without taking a direct dependency on that component
//! library.

use gpui::{
    App, Context, Entity, IntoElement, KeyBinding, Render, SharedString, Stateful, Window, actions,
    div, prelude::*, px,
};

use crate::ui::prelude::*;

actions!(ophelia_number_input, [Increment, Decrement]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", Increment, Some("NumberInput")),
        KeyBinding::new("down", Decrement, Some("NumberInput")),
    ]);
}

pub struct NumberInput {
    input: Entity<TextField>,
    value: SharedString,
    min: u64,
    max: u64,
    step: u64,
}

impl NumberInput {
    pub fn new(
        initial_value: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        min: u64,
        max: u64,
        step: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_value = initial_value.into();
        let sanitized: SharedString = sanitize_digits(initial_value.as_ref()).into();
        let input = cx.new(|cx| TextField::embedded(sanitized.clone(), placeholder, cx));

        cx.subscribe(
            &input,
            |this: &mut Self, _, event: &TextFieldChanged, cx| {
                let sanitized: SharedString = sanitize_digits(event.text.as_ref()).into();
                let needs_reset = sanitized != event.text;

                if this.value != sanitized {
                    this.value = sanitized.clone();
                }

                if needs_reset {
                    this.input.update(cx, |input, cx| {
                        input.set_text(sanitized, cx);
                    });
                }

                cx.notify();
            },
        )
        .detach();

        Self {
            input,
            value: sanitized,
            min,
            max,
            step: step.max(1),
        }
    }

    pub fn text(&self) -> &str {
        self.value.as_ref()
    }

    fn step_value(&mut self, direction: StepDirection, cx: &mut Context<Self>) {
        let fallback = match direction {
            StepDirection::Increment => self.min.max(self.step),
            StepDirection::Decrement => self.min,
        };

        let current = self.value.parse::<u64>().ok().unwrap_or(fallback);
        let next = match direction {
            StepDirection::Increment => current.saturating_add(self.step).min(self.max),
            StepDirection::Decrement => current.saturating_sub(self.step).max(self.min),
        };

        let next_text: SharedString = next.to_string().into();
        self.value = next_text.clone();
        self.input.update(cx, |input, cx| {
            input.set_text(next_text.clone(), cx);
        });
        cx.notify();
    }

    fn increment(&mut self, _: &Increment, _: &mut Window, cx: &mut Context<Self>) {
        self.step_value(StepDirection::Increment, cx);
    }

    fn decrement(&mut self, _: &Decrement, _: &mut Window, cx: &mut Context<Self>) {
        self.step_value(StepDirection::Decrement, cx);
    }
}

impl Render for NumberInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.input.read(cx).is_focused(window);

        h_flex()
            .id(("number-input", cx.entity_id()))
            .key_context("NumberInput")
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
            .on_action(cx.listener(Self::increment))
            .on_action(cx.listener(Self::decrement))
            .child(
                step_button("decrement", "-", StepButtonSide::Left)
                    .border_r_1()
                    .border_color(Colors::input_border())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.step_value(StepDirection::Decrement, cx);
                    })),
            )
            .child(div().flex_1().min_w_0().child(self.input.clone()))
            .child(
                step_button("increment", "+", StepButtonSide::Right)
                    .border_l_1()
                    .border_color(Colors::input_border())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.step_value(StepDirection::Increment, cx);
                    })),
            )
    }
}

#[derive(Clone, Copy)]
enum StepDirection {
    Increment,
    Decrement,
}

#[derive(Clone, Copy)]
enum StepButtonSide {
    Left,
    Right,
}

fn sanitize_digits(text: &str) -> String {
    text.chars().filter(|ch| ch.is_ascii_digit()).collect()
}

fn step_button(id: &'static str, label: &'static str, side: StepButtonSide) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .w(px(Chrome::SIDEBAR_BUTTON_SIZE))
        .h(px(Chrome::SIDEBAR_BUTTON_SIZE))
        .when(matches!(side, StepButtonSide::Left), |this| {
            this.rounded_l(px(Chrome::BUTTON_RADIUS))
        })
        .when(matches!(side, StepButtonSide::Right), |this| {
            this.rounded_r(px(Chrome::BUTTON_RADIUS))
        })
        .flex()
        .items_center()
        .justify_center()
        .bg(Colors::background())
        .text_base()
        .font_weight(gpui::FontWeight::NORMAL)
        .text_color(Colors::foreground())
        .cursor_pointer()
        .hover(|style: gpui::StyleRefinement| style.bg(Colors::muted()))
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::sanitize_digits;

    #[test]
    fn sanitizes_non_numeric_characters() {
        assert_eq!(sanitize_digits("12abc-34"), "1234");
        assert_eq!(sanitize_digits(""), "");
    }
}
