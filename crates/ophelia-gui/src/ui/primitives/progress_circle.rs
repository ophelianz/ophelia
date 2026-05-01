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

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use gpui::{
    App, Background, Hsla, PathBuilder, RenderOnce, Window, canvas, div, point, prelude::*, px,
};

use crate::ui::prelude::*;

#[derive(IntoElement)]
pub struct ProgressCircle {
    id: gpui::ElementId,
    value: f32,
    loading: bool,
    size: f32,
    thickness: f32,
    color: Hsla,
    track_color: Hsla,
}

impl ProgressCircle {
    pub fn new(id: impl Into<gpui::ElementId>) -> Self {
        Self {
            id: id.into(),
            value: 0.0,
            loading: false,
            size: 16.0,
            thickness: 1.5,
            color: Colors::foreground().into(),
            track_color: Colors::border().into(),
        }
    }

    pub fn value(mut self, value: f32) -> Self {
        self.value = value.clamp(0.0, 1.0);
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness;
        self
    }

    pub fn color(mut self, color: impl Into<Hsla>) -> Self {
        self.color = color.into();
        self
    }

    pub fn track_color(mut self, color: impl Into<Hsla>) -> Self {
        self.track_color = color.into();
        self
    }
}

impl RenderOnce for ProgressCircle {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let size = self.size;
        let thickness = self.thickness;
        let color = self.color;
        let track_color = self.track_color;
        let value = self.value;
        let loading = self.loading;

        div().id(self.id).size(px(size)).child(
            canvas(
                |_bounds, _window, _cx| (),
                move |bounds, (), window, _cx| {
                    let origin_x = f32::from(bounds.origin.x);
                    let origin_y = f32::from(bounds.origin.y);
                    let center_x = origin_x + size / 2.0;
                    let center_y = origin_y + size / 2.0;
                    let radius = (size - thickness) / 2.0;

                    let geometry = ArcGeometry {
                        center_x,
                        center_y,
                        radius,
                    };
                    paint_arc(
                        window,
                        geometry,
                        ArcAngles {
                            start: 0.0,
                            end: TAU,
                        },
                        ArcStroke {
                            thickness,
                            color: track_color,
                        },
                    );

                    if loading {
                        paint_arc(
                            window,
                            geometry,
                            ArcAngles {
                                start: -FRAC_PI_2,
                                end: PI * 0.95,
                            },
                            ArcStroke { thickness, color },
                        );
                    } else if value > 0.0 {
                        paint_arc(
                            window,
                            geometry,
                            ArcAngles {
                                start: -FRAC_PI_2,
                                end: -FRAC_PI_2 + TAU * value,
                            },
                            ArcStroke { thickness, color },
                        );
                    }
                },
            )
            .size_full(),
        )
    }
}

#[derive(Clone, Copy)]
struct ArcGeometry {
    center_x: f32,
    center_y: f32,
    radius: f32,
}

#[derive(Clone, Copy)]
struct ArcAngles {
    start: f32,
    end: f32,
}

struct ArcStroke<C> {
    thickness: f32,
    color: C,
}

fn paint_arc(
    window: &mut Window,
    geometry: ArcGeometry,
    angles: ArcAngles,
    stroke: ArcStroke<impl Into<Background>>,
) {
    let delta = angles.end - angles.start;
    if delta.abs() <= f32::EPSILON {
        return;
    }

    let mut path = PathBuilder::stroke(px(stroke.thickness));
    let start = arc_point(
        geometry.center_x,
        geometry.center_y,
        geometry.radius,
        angles.start,
    );
    path.move_to(point(px(start.0), px(start.1)));

    for (segment_start, segment_end) in arc_segments(angles.start, angles.end) {
        let end = arc_point(
            geometry.center_x,
            geometry.center_y,
            geometry.radius,
            segment_end,
        );
        path.arc_to(
            point(px(geometry.radius), px(geometry.radius)),
            px(0.0),
            (segment_end - segment_start).abs() > PI,
            segment_end >= segment_start,
            point(px(end.0), px(end.1)),
        );
    }

    if let Ok(path) = path.build() {
        window.paint_path(path, stroke.color);
    }
}

fn arc_segments(start_angle: f32, end_angle: f32) -> Vec<(f32, f32)> {
    let mut segments = Vec::new();
    let direction = if end_angle >= start_angle { 1.0 } else { -1.0 };
    let mut cursor = start_angle;
    let mut remaining = (end_angle - start_angle).abs();

    while remaining > 0.0 {
        let step = remaining.min(PI - 0.001);
        let next = cursor + step * direction;
        segments.push((cursor, next));
        cursor = next;
        remaining -= step;
    }

    segments
}

fn arc_point(center_x: f32, center_y: f32, radius: f32, angle: f32) -> (f32, f32) {
    (
        center_x + radius * angle.cos(),
        center_y + radius * angle.sin(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_circle_clamps_values() {
        let circle = ProgressCircle::new("progress").value(4.0);
        assert_eq!(circle.value, 1.0);
    }
}
