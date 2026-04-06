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

//! Text input adapted from the `Input` component in
//! [longbridge/gpui-component](https://github.com/longbridge/gpui-component),
//! which is Apache-2.0 licensed. Ophelia keeps a local copy so it can tailor
//! behavior and styling without taking a direct dependency on that component
//! library.

use std::ops::Range;

use gpui::{
    App, Bounds, ContentMask, Context, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, IntoElement, KeyBinding, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render,
    ShapedLine, SharedString, Style, TextAlign, TextRun, UTF16Selection, Window, actions, div,
    fill, point, prelude::*, px, relative, rgba, size,
};

use crate::ui::prelude::*;

actions!(
    ophelia_text_field,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
        Submit,
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, None),
        KeyBinding::new("delete", Delete, None),
        KeyBinding::new("left", Left, None),
        KeyBinding::new("right", Right, None),
        KeyBinding::new("shift-left", SelectLeft, None),
        KeyBinding::new("shift-right", SelectRight, None),
        KeyBinding::new("home", Home, None),
        KeyBinding::new("end", End, None),
        KeyBinding::new("cmd-left", Home, None),
        KeyBinding::new("cmd-right", End, None),
        KeyBinding::new("ctrl-a", SelectAll, None),
        KeyBinding::new("cmd-a", SelectAll, None),
        KeyBinding::new("ctrl-v", Paste, None),
        KeyBinding::new("cmd-v", Paste, None),
        KeyBinding::new("ctrl-c", Copy, None),
        KeyBinding::new("cmd-c", Copy, None),
        KeyBinding::new("ctrl-x", Cut, None),
        KeyBinding::new("cmd-x", Cut, None),
        KeyBinding::new("enter", Submit, None),
    ]);
}

pub struct TextFieldChanged {
    pub text: SharedString,
}

pub struct TextFieldSubmitted;

pub struct TextField {
    focus_handle: FocusHandle,
    text: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    scroll_offset: Pixels,
    embedded: bool,
    is_selecting: bool,
}

impl gpui::EventEmitter<TextFieldChanged> for TextField {}
impl gpui::EventEmitter<TextFieldSubmitted> for TextField {}

impl TextField {
    pub fn new(
        initial_text: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = initial_text.into();
        let len = text.len();

        Self {
            focus_handle: cx.focus_handle(),
            text,
            placeholder: placeholder.into(),
            selected_range: len..len,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            scroll_offset: px(0.0),
            embedded: false,
            is_selecting: false,
        }
    }

    pub fn embedded(
        initial_text: impl Into<SharedString>,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut input = Self::new(initial_text, placeholder, cx);
        input.embedded = true;
        input
    }

    pub fn text(&self) -> &str {
        self.text.as_ref()
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        if self.text == text {
            return;
        }

        self.text = text;
        let len = self.text.len();
        self.selected_range = len..len;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.emit(TextFieldChanged {
            text: self.text.clone(),
        });
        cx.notify();
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }

        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }

        cx.notify();
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.text[..offset]
            .char_indices()
            .last()
            .map(|(idx, _)| idx)
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        if offset >= self.text.len() {
            return self.text.len();
        }

        self.text[offset..]
            .chars()
            .next()
            .map(|ch| offset + ch.len_utf8())
            .unwrap_or(self.text.len())
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.text.chars() {
            if utf16_count >= offset {
                break;
            }

            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.text.chars() {
            if utf8_count >= offset {
                break;
            }

            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn emit_changed(&self, cx: &mut Context<Self>) {
        cx.emit(TextFieldChanged {
            text: self.text.clone(),
        });
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.text.len();
        self.selection_reversed = false;
        cx.notify();
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.text.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextFieldSubmitted);
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace('\n', " "), window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                self.text[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                self.text[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.text.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return self.text.len();
        };

        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.text.len();
        }

        line.closest_index_for_x(position.x - bounds.left() - self.scroll_offset)
    }

    fn clamp_scroll_offset(
        scroll_offset: Pixels,
        content_width: Pixels,
        visible_width: Pixels,
    ) -> Pixels {
        let min_scroll = (visible_width - content_width).min(px(0.0));
        scroll_offset.max(min_scroll).min(px(0.0))
    }

    fn adjusted_scroll_offset(
        scroll_offset: Pixels,
        cursor_pos: Pixels,
        selection_bounds: Option<(Pixels, Pixels)>,
        selection_reversed: bool,
        content_width: Pixels,
        visible_width: Pixels,
    ) -> Pixels {
        let safety_margin = px(6.0);
        let max_cursor_x = (visible_width - safety_margin).max(safety_margin);

        let mut scroll_offset = if scroll_offset + cursor_pos > max_cursor_x {
            max_cursor_x - cursor_pos
        } else if scroll_offset + cursor_pos < safety_margin {
            safety_margin - cursor_pos
        } else {
            scroll_offset
        };

        if let Some((selection_start, selection_end)) = selection_bounds {
            if selection_reversed {
                if scroll_offset + selection_end < px(0.0) {
                    scroll_offset = -selection_end;
                }
            } else if scroll_offset + selection_start < px(0.0) {
                scroll_offset = -selection_start;
            }
        }

        Self::clamp_scroll_offset(scroll_offset, content_width, visible_width)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.is_selecting = true;

        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        adjusted_range.replace(self.range_to_utf16(&range));
        Some(self.text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.text =
            (self.text[0..range.start].to_owned() + new_text + &self.text[range.end..]).into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.selection_reversed = false;
        self.marked_range.take();
        self.emit_changed(cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.text =
            (self.text[0..range.start].to_owned() + new_text + &self.text[range.end..]).into();
        self.marked_range = if new_text.is_empty() {
            None
        } else {
            Some(range.start..range.start + new_text.len())
        };
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.start)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.selection_reversed = false;
        self.emit_changed(cx);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);

        Some(Bounds::from_corners(
            point(
                bounds.left() + self.scroll_offset + last_layout.x_for_index(range.start),
                bounds.top(),
            ),
            point(
                bounds.left() + self.scroll_offset + last_layout.x_for_index(range.end),
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        if point.y < bounds.top() || point.y > bounds.bottom() {
            return None;
        }
        let last_layout = self.last_layout.as_ref()?;
        let utf8_index = last_layout.index_for_x(point.x - bounds.left() - self.scroll_offset)?;
        Some(self.offset_to_utf16(utf8_index))
    }
}

impl Focusable for TextField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

struct TextFieldElement {
    input: Entity<TextField>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    scroll_offset: Pixels,
}

impl IntoElement for TextFieldElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextFieldElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = input.text.clone();
        let content_is_empty = content.is_empty();
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let mut scroll_offset = input.scroll_offset;
        let style = window.text_style();

        let (display_text, text_color) = if content_is_empty {
            (input.placeholder.clone(), Colors::muted_foreground().into())
        } else {
            (content, style.color)
        };

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        let runs = if let Some(marked_range) = input.marked_range.as_ref() {
            vec![
                TextRun {
                    len: marked_range.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(gpui::UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - marked_range.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);

        if content_is_empty {
            scroll_offset = px(0.0);
        } else {
            let cursor_pos = line.x_for_index(cursor);
            let visible_width = bounds.size.width;
            let selection_bounds = (!selected_range.is_empty()).then(|| {
                (
                    line.x_for_index(selected_range.start),
                    line.x_for_index(selected_range.end),
                )
            });

            scroll_offset = TextField::adjusted_scroll_offset(
                scroll_offset,
                cursor_pos,
                selection_bounds,
                input.selection_reversed,
                line.width,
                visible_width,
            );
        }

        let cursor_pos = line.x_for_index(cursor) + scroll_offset;
        let (selection, cursor) = if selected_range.is_empty() || input.text.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + cursor_pos, bounds.top()),
                        size(px(2.0), bounds.bottom() - bounds.top()),
                    ),
                    Colors::active(),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + scroll_offset + line.x_for_index(selected_range.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + scroll_offset + line.x_for_index(selected_range.end),
                            bounds.bottom(),
                        ),
                    ),
                    rgba(0x7ED37F33),
                )),
                None,
            )
        };

        PrepaintState {
            line: Some(line),
            cursor,
            selection,
            scroll_offset,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        let line = prepaint.line.take().unwrap();
        let scroll_offset = prepaint.scroll_offset;
        let mask = ContentMask { bounds };

        window.with_content_mask(Some(mask), |window| {
            if let Some(selection) = prepaint.selection.take() {
                window.paint_quad(selection);
            }

            line.paint(
                point(bounds.origin.x + scroll_offset, bounds.origin.y),
                window.line_height(),
                TextAlign::Left,
                None,
                window,
                cx,
            )
            .unwrap();

            if focus_handle.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });

        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
            input.scroll_offset = scroll_offset;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_scroll_offset_keeps_content_within_visible_bounds() {
        assert_eq!(
            TextField::clamp_scroll_offset(px(-20.0), px(300.0), px(100.0)),
            px(-20.0)
        );
        assert_eq!(
            TextField::clamp_scroll_offset(px(40.0), px(300.0), px(100.0)),
            px(0.0)
        );
        assert_eq!(
            TextField::clamp_scroll_offset(px(-260.0), px(300.0), px(100.0)),
            px(-200.0)
        );
    }

    #[test]
    fn adjusted_scroll_offset_keeps_caret_visible_for_long_lines() {
        assert_eq!(
            TextField::adjusted_scroll_offset(
                px(0.0),
                px(220.0),
                None,
                false,
                px(320.0),
                px(100.0),
            ),
            px(-126.0)
        );
    }

    #[test]
    fn adjusted_scroll_offset_keeps_forward_selection_start_in_view() {
        assert_eq!(
            TextField::adjusted_scroll_offset(
                px(-50.0),
                px(80.0),
                Some((px(20.0), px(140.0))),
                false,
                px(220.0),
                px(100.0),
            ),
            px(-20.0)
        );
    }

    #[test]
    fn adjusted_scroll_offset_keeps_reversed_selection_end_in_view() {
        assert_eq!(
            TextField::adjusted_scroll_offset(
                px(-50.0),
                px(80.0),
                Some((px(20.0), px(20.0))),
                true,
                px(220.0),
                px(100.0),
            ),
            px(-20.0)
        );
    }
}

impl Render for TextField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);

        div()
            .id(("text-field", cx.entity_id()))
            .flex()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .key_context("TextField")
            .track_focus(&self.focus_handle(cx))
            .cursor_text()
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::submit))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .line_height(px(20.0))
            .text_size(px(14.0))
            .when(!self.embedded, |this| {
                this.bg(Colors::background())
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(if focused {
                        Colors::ring()
                    } else {
                        Colors::input_border()
                    })
            })
            .px(px(12.0))
            .py(px(10.0))
            .child(TextFieldElement { input: cx.entity() })
    }
}
