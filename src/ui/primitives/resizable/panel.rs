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

use std::{ops::Range, rc::Rc};

use gpui::{
    AnyElement, App, Axis, Bounds, Element, ElementId, Entity, EventEmitter, GlobalElementId,
    InteractiveElement as _, IntoElement, IsZero as _, MouseMoveEvent, MouseUpEvent, ParentElement,
    Pixels, RenderOnce, Style, Styled, Window, canvas, div,
};

use crate::ui::prelude::{h_flex, v_flex};

use super::{
    PANEL_MIN_SIZE, ResizablePanelEvent, ResizablePanelState, ResizableState, resize_handle,
};

#[derive(Clone)]
struct ResizeCallbacks {
    on_resize: Rc<dyn Fn(&Entity<ResizableState>, &mut Window, &mut App)>,
}

#[derive(IntoElement)]
pub struct ResizablePanelGroup {
    id: ElementId,
    state: Option<Entity<ResizableState>>,
    axis: Axis,
    children: Vec<ResizablePanel>,
    on_resize: Rc<dyn Fn(&Entity<ResizableState>, &mut Window, &mut App)>,
}

impl ResizablePanelGroup {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            axis: Axis::Horizontal,
            children: Vec::new(),
            state: None,
            on_resize: Rc::new(|_, _, _| {}),
        }
    }

    pub fn with_state(mut self, state: &Entity<ResizableState>) -> Self {
        self.state = Some(state.clone());
        self
    }

    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    pub fn child(mut self, panel: impl Into<ResizablePanel>) -> Self {
        self.children.push(panel.into());
        self
    }

    #[allow(dead_code)]
    pub fn children<I>(mut self, panels: impl IntoIterator<Item = I>) -> Self
    where
        I: Into<ResizablePanel>,
    {
        self.children.extend(panels.into_iter().map(Into::into));
        self
    }

    pub fn on_resize(
        mut self,
        on_resize: impl Fn(&Entity<ResizableState>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_resize = Rc::new(on_resize);
        self
    }
}

impl<T> From<T> for ResizablePanel
where
    T: Into<AnyElement>,
{
    fn from(value: T) -> Self {
        super::resizable_panel().child(value.into())
    }
}

impl EventEmitter<ResizablePanelEvent> for ResizablePanelGroup {}

impl RenderOnce for ResizablePanelGroup {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state.unwrap_or(
            window.use_keyed_state(self.id.clone(), cx, |_, _| ResizableState::default()),
        );

        state.update(cx, |state, cx| {
            state.sync_panels_count(self.axis, self.children.len(), cx);
        });

        let callbacks = ResizeCallbacks {
            on_resize: Rc::clone(&self.on_resize),
        };

        let container = match self.axis {
            Axis::Horizontal => h_flex(),
            Axis::Vertical => v_flex(),
        };

        container
            .id(self.id)
            .size_full()
            .children(
                self.children
                    .into_iter()
                    .enumerate()
                    .map(|(ix, mut panel)| {
                        panel.axis = self.axis;
                        panel.panel_ix = ix;
                        panel.state = Some(state.clone());
                        panel
                    }),
            )
            .child(
                canvas(
                    {
                        let state = state.clone();
                        move |bounds, _, cx| {
                            state.update(cx, |state, cx| {
                                state.set_bounds(bounds, cx);
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(ResizePanelGroupElement {
                state,
                axis: self.axis,
                callbacks,
            })
    }
}

#[derive(IntoElement)]
pub struct ResizablePanel {
    pub(super) axis: Axis,
    pub(super) panel_ix: usize,
    pub(super) state: Option<Entity<ResizableState>>,
    initial_size: Option<Pixels>,
    size_range: Range<Pixels>,
    children: Vec<AnyElement>,
    visible: bool,
}

impl ResizablePanel {
    pub(super) fn new() -> Self {
        Self {
            axis: Axis::Horizontal,
            panel_ix: 0,
            state: None,
            initial_size: None,
            size_range: PANEL_MIN_SIZE..Pixels::MAX,
            children: Vec::new(),
            visible: true,
        }
    }

    #[allow(dead_code)]
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.initial_size = Some(size.into());
        self
    }

    pub fn size_range(mut self, range: impl Into<Range<Pixels>>) -> Self {
        self.size_range = range.into();
        self
    }
}

impl ParentElement for ResizablePanel {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for ResizablePanel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        if !self.visible {
            return div().id(("resizable-panel-hidden", self.panel_ix));
        }

        let state = self
            .state
            .expect("ResizablePanelGroup must inject state into its panels");
        let panel_state =
            state
                .read(cx)
                .panels
                .get(self.panel_ix)
                .cloned()
                .unwrap_or(ResizablePanelState {
                    size: self.initial_size,
                    size_range: self.size_range.clone(),
                    bounds: Bounds::default(),
                });
        let size_range = self.size_range.clone();

        let panel = div()
            .id(("resizable-panel", self.panel_ix))
            .flex()
            .flex_grow()
            .size_full()
            .relative()
            .min_w_0()
            .min_h_0();

        let panel = match self.axis {
            Axis::Vertical => panel.min_h(size_range.start).max_h(size_range.end),
            Axis::Horizontal => panel.min_w(size_range.start).max_w(size_range.end),
        };

        let panel = if self.initial_size.is_none() {
            panel.flex_shrink()
        } else {
            panel
        };

        let panel = if let Some(initial_size) = self.initial_size {
            let panel = if panel_state.size.is_none() && !initial_size.is_zero() {
                panel.flex_none()
            } else {
                panel
            };

            panel.flex_basis(initial_size)
        } else {
            panel
        };

        let panel = match panel_state.size {
            Some(size) => panel.flex_basis(size.min(size_range.end).max(size_range.start)),
            None => panel,
        };

        let panel = panel
            .child(
                canvas(
                    {
                        let state = state.clone();
                        let size_range = self.size_range.clone();
                        move |bounds, _, cx| {
                            state.update(cx, |state, cx| {
                                state.update_panel_size(
                                    self.panel_ix,
                                    bounds,
                                    size_range.clone(),
                                    cx,
                                );
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .children(self.children);

        if self.panel_ix > 0 {
            let handle_ix = self.panel_ix - 1;
            panel.child(
                resize_handle(("resizable-handle", handle_ix), self.axis).on_mouse_down(
                    gpui::MouseButton::Left,
                    move |_, window, cx| {
                        cx.stop_propagation();
                        window.prevent_default();
                        state.update(cx, |state, _| {
                            state.resizing_panel_ix = Some(handle_ix);
                        });
                    },
                ),
            )
        } else {
            panel
        }
    }
}

struct ResizePanelGroupElement {
    state: Entity<ResizableState>,
    axis: Axis,
    callbacks: ResizeCallbacks,
}

impl IntoElement for ResizePanelGroupElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ResizePanelGroupElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        (window.request_layout(Style::default(), None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        ()
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        window.on_mouse_event({
            let state = self.state.clone();
            let axis = self.axis;
            move |e: &MouseMoveEvent, phase, window, cx| {
                if !phase.bubble() {
                    return;
                }

                let Some(ix) = state.read(cx).resizing_panel_ix else {
                    return;
                };

                state.update(cx, |state, cx| {
                    let Some(panel) = state.panels.get(ix) else {
                        return;
                    };

                    let size = match axis {
                        Axis::Horizontal => e.position.x - panel.bounds.left(),
                        Axis::Vertical => e.position.y - panel.bounds.top(),
                    };
                    state.resize_panel(ix, size, cx);
                });
                window.refresh();
            }
        });

        window.on_mouse_event({
            let state = self.state.clone();
            let on_resize = Rc::clone(&self.callbacks.on_resize);
            move |_: &MouseUpEvent, phase, window, cx| {
                if !phase.bubble() || state.read(cx).resizing_panel_ix.is_none() {
                    return;
                }

                state.update(cx, |state, cx| {
                    state.done_resizing(cx);
                });
                on_resize(&state, window, cx);
                window.refresh();
            }
        });
    }
}
