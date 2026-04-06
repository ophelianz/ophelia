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

use std::rc::Rc;

use gpui::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled, Window, div, px,
};

use crate::ui::prelude::*;

#[derive(Clone)]
pub struct SegmentedControlOption {
    id: ElementId,
    label: SharedString,
    selected: bool,
    min_width: Option<f32>,
    on_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}

impl SegmentedControlOption {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            selected: false,
            min_width: None,
            on_click: None,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn min_width(mut self, min_width: f32) -> Self {
        self.min_width = Some(min_width);
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

#[derive(IntoElement)]
pub struct SegmentedControl {
    id: ElementId,
    options: Vec<SegmentedControlOption>,
}

impl SegmentedControl {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            options: Vec::new(),
        }
    }

    pub fn option(mut self, option: SegmentedControlOption) -> Self {
        self.options.push(option);
        self
    }
}

impl RenderOnce for SegmentedControl {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let option_count = self.options.len();

        h_flex()
            .id(self.id)
            .overflow_hidden()
            .rounded(px(Chrome::BUTTON_RADIUS))
            .border_1()
            .border_color(Colors::input_border())
            .bg(Colors::background())
            .children(self.options.into_iter().enumerate().map(|(index, option)| {
                let segment = div()
                    .id(option.id)
                    .min_w(px(option.min_width.unwrap_or(0.0)))
                    .h(px(Chrome::BUTTON_HEIGHT))
                    .px(px(Chrome::BUTTON_PADDING_X))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .font_weight(if option.selected {
                        gpui::FontWeight::SEMIBOLD
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .text_color(if option.selected {
                        Colors::foreground()
                    } else {
                        Colors::muted_foreground()
                    })
                    .bg(if option.selected {
                        Colors::muted()
                    } else {
                        Colors::background()
                    })
                    .cursor_pointer()
                    .hover(|style| style.bg(Colors::muted()))
                    .child(option.label);

                let segment = if option_count == 1 {
                    segment.rounded(px(Chrome::BUTTON_RADIUS))
                } else if index == 0 {
                    segment.rounded_l(px(Chrome::BUTTON_RADIUS))
                } else if index == option_count.saturating_sub(1) {
                    segment.rounded_r(px(Chrome::BUTTON_RADIUS))
                } else {
                    segment
                };

                let segment = if index < option_count.saturating_sub(1) {
                    segment.border_r_1().border_color(Colors::input_border())
                } else {
                    segment
                };

                let segment = if let Some(on_click) = option.on_click {
                    segment.on_click(move |event, window, cx| on_click(event, window, cx))
                } else {
                    segment
                };

                segment.into_any_element()
            }))
    }
}
