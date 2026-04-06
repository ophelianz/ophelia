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

use gpui::{App, RenderOnce, Window, div, prelude::*, px};
use rust_i18n::t;

use crate::ui::prelude::*;

#[derive(IntoElement)]
pub struct ChunkBitmapCard;

impl RenderOnce for ChunkBitmapCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .justify_between()
            .h_full()
            .gap(px(Spacing::SECTION_GAP))
            .p(px(Chrome::STATS_CARD_PADDING))
            .rounded(px(Chrome::PANEL_RADIUS))
            .border_1()
            .border_color(Colors::border())
            .bg(Colors::card())
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(Colors::muted_foreground())
                    .child(t!("stats.chunk_bitmap_title").to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(Chrome::CONTROL_RADIUS))
                    .border_1()
                    .border_color(Colors::border())
                    .bg(Colors::muted())
                    .child(
                        div()
                            .text_sm()
                            .text_center()
                            .text_color(Colors::muted_foreground())
                            .child(t!("stats.chunk_bitmap_placeholder").to_string()),
                    ),
            )
    }
}
