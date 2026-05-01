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

use gpui::{Axis, ElementId, InteractiveElement as _, ParentElement, Stateful, Styled, div, px};

use crate::ui::prelude::*;

pub(crate) fn resize_handle(id: impl Into<ElementId>, axis: Axis) -> Stateful<gpui::Div> {
    let negative_offset = -px(Chrome::RESIZE_HANDLE_PADDING);

    let handle = div()
        .id(id)
        .occlude()
        .absolute()
        .flex_shrink_0()
        .group("resize-handle");

    let handle = match axis {
        Axis::Horizontal => handle
            .cursor_col_resize()
            .top_0()
            .left(negative_offset)
            .h_full()
            .w(px(Chrome::RESIZE_HANDLE_SIZE))
            .px(px(Chrome::RESIZE_HANDLE_PADDING)),
        Axis::Vertical => handle
            .cursor_row_resize()
            .top(negative_offset)
            .left_0()
            .w_full()
            .h(px(Chrome::RESIZE_HANDLE_SIZE))
            .py(px(Chrome::RESIZE_HANDLE_PADDING)),
    };

    handle.child(match axis {
        Axis::Horizontal => div()
            .bg(Colors::border())
            .group_hover("resize-handle", |this| this.bg(Colors::ring()))
            .h_full()
            .w(px(Chrome::RESIZE_HANDLE_SIZE)),
        Axis::Vertical => div()
            .bg(Colors::border())
            .group_hover("resize-handle", |this| this.bg(Colors::ring()))
            .w_full()
            .h(px(Chrome::RESIZE_HANDLE_SIZE)),
    })
}
