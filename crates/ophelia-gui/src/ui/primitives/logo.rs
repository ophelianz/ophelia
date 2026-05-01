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

use crate::theme::Colors;
use gpui::{
    App, FillOptions, FillRule, PathBuilder, PathStyle, Window, canvas, div, point, prelude::*, px,
};

#[derive(IntoElement)]
pub struct OpheliaLogo {
    pub size: f32,
}

impl OpheliaLogo {
    pub fn new(size: f32) -> Self {
        Self { size }
    }
}

impl RenderOnce for OpheliaLogo {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let size = self.size;
        let scale = size / 24.0;
        let ring_color = Colors::active();
        let dot_color = Colors::active_dim();

        div().size(px(size)).child(
            canvas(
                move |_bounds, _window, _cx| (),
                move |bounds, (), window, _cx| {
                    let ox = f32::from(bounds.origin.x);
                    let oy = f32::from(bounds.origin.y);

                    // Ring: outer circle (center 12,12 r=9) + inner circle (center 15,12 r=7.5)
                    // EvenOdd fill rule makes their overlap transparent, creating the crescent ring.
                    // The inner circle extends past the outer on the right (x=22.5 vs x=21),
                    // so that overhang gets filled too.
                    let mut builder = PathBuilder::fill().with_style(PathStyle::Fill(
                        FillOptions::default().with_fill_rule(FillRule::EvenOdd),
                    ));
                    builder.scale(scale);
                    builder.translate(point(px(ox), px(oy)));

                    builder.move_to(point(px(21.0), px(12.0)));
                    builder.arc_to(
                        point(px(9.0), px(9.0)),
                        px(0.0),
                        false,
                        false,
                        point(px(3.0), px(12.0)),
                    );
                    builder.arc_to(
                        point(px(9.0), px(9.0)),
                        px(0.0),
                        false,
                        false,
                        point(px(21.0), px(12.0)),
                    );
                    builder.close();

                    builder.move_to(point(px(22.5), px(12.0)));
                    builder.arc_to(
                        point(px(7.5), px(7.5)),
                        px(0.0),
                        false,
                        false,
                        point(px(7.5), px(12.0)),
                    );
                    builder.arc_to(
                        point(px(7.5), px(7.5)),
                        px(0.0),
                        false,
                        false,
                        point(px(22.5), px(12.0)),
                    );
                    builder.close();

                    if let Ok(ring) = builder.build() {
                        window.paint_path(ring, ring_color);
                    }

                    // Dot: center (15,12) r=1.5
                    let mut dot_builder = PathBuilder::fill();
                    dot_builder.scale(scale);
                    dot_builder.translate(point(px(ox), px(oy)));

                    dot_builder.move_to(point(px(16.5), px(12.0)));
                    dot_builder.arc_to(
                        point(px(1.5), px(1.5)),
                        px(0.0),
                        false,
                        false,
                        point(px(13.5), px(12.0)),
                    );
                    dot_builder.arc_to(
                        point(px(1.5), px(1.5)),
                        px(0.0),
                        false,
                        false,
                        point(px(16.5), px(12.0)),
                    );
                    dot_builder.close();

                    if let Ok(dot) = dot_builder.build() {
                        window.paint_path(dot, dot_color);
                    }
                },
            )
            .size(px(size)),
        )
    }
}
