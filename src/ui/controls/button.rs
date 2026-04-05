//! Button control adapted from the `Button` component in
//! [longbridge/gpui-component](https://github.com/longbridge/gpui-component),
//! which is Apache-2.0 licensed. Ophelia keeps a focused local adaptation for
//! its own UI instead of depending on the full component library.

use std::rc::Rc;

use gpui::{
    App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, prelude::*, px,
};

use crate::ui::prelude::*;

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    compact: bool,
    on_click: Option<Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            compact: false,
            on_click: None,
        }
    }

    pub fn compact(mut self) -> Self {
        self.compact = true;
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

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .id(self.id)
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .h(px(Chrome::BUTTON_HEIGHT))
            .px(px(if self.compact {
                Chrome::BUTTON_COMPACT_PADDING_X
            } else {
                Chrome::BUTTON_PADDING_X
            }))
            .rounded(px(Chrome::BUTTON_RADIUS))
            .border_1()
            .border_color(Colors::input_border())
            .bg(Colors::muted())
            .text_sm()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(Colors::foreground())
            .cursor_pointer()
            .hover(|style| style.bg(Colors::card_hover()))
            .active(|style| style.bg(Colors::card()))
            .child(self.label)
            .when_some(self.on_click, |this, on_click| {
                this.on_click(move |event, window, cx| on_click(event, window, cx))
            })
    }
}
