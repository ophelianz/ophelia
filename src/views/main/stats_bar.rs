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

use crate::ui::prelude::*;
use gpui::{
    App, Background, Hsla, PathBuilder, Window, canvas, div, linear_color_stop, linear_gradient,
    point, prelude::*, px, rgba,
};
use rust_i18n::t;

#[derive(IntoElement)]
pub struct StatsBar {
    pub download_samples: Vec<f32>,
    pub download_speed: f32,
    pub disk_read_speed: Option<f32>,
    pub disk_write_speed: Option<f32>,
    pub active_count: usize,
    pub finished_count: usize,
    pub queued_count: usize,
}

impl RenderOnce for StatsBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let download_speed = format_speed_value(self.download_speed);

        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(Spacing::SECTION_GAP))
            .child(
                h_flex()
                    .items_start()
                    .justify_between()
                    .gap(px(Spacing::SECTION_GAP))
                    .flex_wrap()
                    .child(
                        primary_speed_metric(
                            t!("stats.download").to_string(),
                            download_speed,
                            t!("stats.window").to_string(),
                        )
                        .into_any_element(),
                    )
                    .child(
                        disk_io_metric(
                            self.disk_read_speed,
                            self.disk_write_speed,
                            t!("stats.disk_io").to_string(),
                        )
                        .into_any_element(),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(Chrome::STATS_GRAPH_HEIGHT))
                    .w_full()
                    .child(throughput_graph(self.download_samples)),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(Spacing::SECTION_GAP))
                    .flex_wrap()
                    .child(count_metric(
                        &t!("stats.active").to_string(),
                        &self.active_count.to_string(),
                        Colors::active().into(),
                    ))
                    .child(count_metric(
                        &t!("stats.finished").to_string(),
                        &self.finished_count.to_string(),
                        Colors::finished().into(),
                    ))
                    .child(count_metric(
                        &t!("stats.queued").to_string(),
                        &self.queued_count.to_string(),
                        Colors::queued().into(),
                    )),
            )
    }
}

// ---------------------------------------------------------------------------
// Graph
// ---------------------------------------------------------------------------

fn throughput_graph(download: Vec<f32>) -> impl IntoElement {
    let line: Hsla = Colors::active().into();
    let background: Hsla = Colors::background().into();
    let grid = rgba(0xffffff08);
    let render_max = graph_label_max(&download).max(1.0);

    canvas(
        |_bounds, _window, _cx| (),
        move |bounds, (), window, _cx| {
            let w = f32::from(bounds.size.width);
            let h = f32::from(bounds.size.height);
            let ox = f32::from(bounds.origin.x);
            let oy = f32::from(bounds.origin.y);

            // Small inset so curves don't clip at canvas edges
            let pad = 2.0;
            let gw = w - pad * 2.0;
            let gh = h - pad * 2.0;
            let gx = ox + pad;
            let gy = oy + pad;

            let dl = graph_points(&download, gx, gy, gw, gh, render_max);

            for frac in [0.0_f32, 0.5, 1.0] {
                hline(window, gx, gx + gw, gy + gh * frac, grid);
            }

            smooth_area(
                window,
                &dl,
                gx,
                gy + gh,
                gx + gw,
                linear_gradient(
                    0.0,
                    linear_color_stop(line.opacity(0.42), 1.0),
                    linear_color_stop(background.opacity(0.0), 0.0),
                ),
            );
            smooth_stroke(window, &dl, 1.5, line);
        },
    )
    .size_full()
}

// ---------------------------------------------------------------------------
// Bezier-smoothed drawing
// ---------------------------------------------------------------------------

/// Catmull-Rom → cubic bezier control points for segment pts[i] → pts[i+1].
fn catmull_rom_cp(pts: &[(f32, f32)], i: usize) -> ((f32, f32), (f32, f32)) {
    let p0 = if i > 0 { pts[i - 1] } else { pts[i] };
    let p1 = pts[i];
    let p2 = pts[i + 1];
    let p3 = if i + 2 < pts.len() {
        pts[i + 2]
    } else {
        pts[i + 1]
    };

    let cp1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
    let cp2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
    (cp1, cp2)
}

fn graph_points(samples: &[f32], gx: f32, gy: f32, gw: f32, gh: f32, max: f32) -> Vec<(f32, f32)> {
    let n = samples.len();
    if n == 0 {
        return Vec::new();
    }
    let denom = (n - 1).max(1) as f32;
    samples
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = gx + (i as f32 / denom) * gw;
            let y = gy + gh - (v / max) * gh * 0.92;
            (x, y)
        })
        .collect()
}

/// Filled area under a smooth catmull-rom curve. Uses native cubic_bezier_to
/// on PathBuilder, one single paint_path call for the entire area.
fn smooth_area(
    window: &mut Window,
    pts: &[(f32, f32)],
    x0: f32,
    y_floor: f32,
    x1: f32,
    color: impl Into<Background>,
) {
    if pts.len() < 2 {
        return;
    }

    let mut p = PathBuilder::fill();
    // Start at bottom-left, line up to first data point
    p.move_to(point(px(x0), px(y_floor)));
    p.line_to(point(px(pts[0].0), px(pts[0].1)));

    // Smooth cubic segments through every data point
    for i in 0..pts.len() - 1 {
        let (cp1, cp2) = catmull_rom_cp(pts, i);
        p.cubic_bezier_to(
            point(px(pts[i + 1].0), px(pts[i + 1].1)),
            point(px(cp1.0), px(cp1.1)),
            point(px(cp2.0), px(cp2.1)),
        );
    }

    // Close back along the floor
    p.line_to(point(px(x1), px(y_floor)));
    p.close();
    if let Ok(path) = p.build() {
        window.paint_path(path, color);
    }
}

fn smooth_stroke(
    window: &mut Window,
    pts: &[(f32, f32)],
    thickness: f32,
    color: impl Into<Background>,
) {
    if pts.len() < 2 {
        return;
    }

    let mut p = PathBuilder::stroke(px(thickness));
    p.move_to(point(px(pts[0].0), px(pts[0].1)));
    for i in 0..pts.len() - 1 {
        let (cp1, cp2) = catmull_rom_cp(pts, i);
        p.cubic_bezier_to(
            point(px(pts[i + 1].0), px(pts[i + 1].1)),
            point(px(cp1.0), px(cp1.1)),
            point(px(cp2.0), px(cp2.1)),
        );
    }
    if let Ok(path) = p.build() {
        window.paint_path(path, color);
    }
}

// ---------------------------------------------------------------------------
// Canvas primitives
// ---------------------------------------------------------------------------

fn hline(window: &mut Window, x0: f32, x1: f32, y: f32, color: impl Into<Background>) {
    let mut p = PathBuilder::fill();
    p.move_to(point(px(x0), px(y - 0.5)));
    p.line_to(point(px(x1), px(y - 0.5)));
    p.line_to(point(px(x1), px(y + 0.5)));
    p.line_to(point(px(x0), px(y + 0.5)));
    p.close();
    if let Ok(path) = p.build() {
        window.paint_path(path, color);
    }
}

fn graph_label_max(samples: &[f32]) -> f32 {
    samples.iter().copied().fold(0.0_f32, f32::max)
}

fn primary_speed_metric(label: String, value: String, caption: String) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(4.0))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::LIGHT)
                .text_color(Colors::muted_foreground())
                .child(label),
        )
        .child(
            h_flex()
                .items_end()
                .gap(px(8.0))
                .child(
                    div()
                        .text_size(px(28.0))
                        .font_weight(gpui::FontWeight::EXTRA_BOLD)
                        .text_color(Colors::foreground())
                        .child(value),
                )
                .child(
                    div()
                        .pb(px(3.0))
                        .text_sm()
                        .font_weight(gpui::FontWeight::LIGHT)
                        .text_color(Colors::muted_foreground())
                        .child("MB/s"),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(Colors::muted_foreground())
                .child(caption),
        )
}

fn disk_io_metric(
    read_speed: Option<f32>,
    write_speed: Option<f32>,
    label: String,
) -> impl IntoElement {
    div()
        .flex()
        .items_start()
        .gap(px(Spacing::CONTROL_GAP))
        .flex_shrink_0()
        .child(icon_m(IconName::Storage, Colors::muted_foreground()))
        .child(
            v_flex()
                .gap(px(6.0))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::LIGHT)
                        .text_color(Colors::muted_foreground())
                        .child(label),
                )
                .child(
                    h_flex()
                        .items_start()
                        .gap(px(Spacing::SECTION_GAP))
                        .child(io_metric(t!("stats.read").to_string(), read_speed))
                        .child(io_metric(t!("stats.write").to_string(), write_speed)),
                ),
        )
}

fn io_metric(label: String, speed: Option<f32>) -> impl IntoElement {
    v_flex()
        .gap(px(2.0))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::LIGHT)
                .text_color(Colors::muted_foreground())
                .child(label),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::LIGHT)
                .text_color(if speed.is_some() {
                    Colors::foreground()
                } else {
                    Colors::muted_foreground()
                })
                .child(format_optional_speed(speed)),
        )
}

fn count_metric(label: &str, value: &str, color: Hsla) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap(px(6.0))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::LIGHT)
                .text_color(Colors::muted_foreground())
                .child(label.to_string()),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::EXTRA_BOLD)
                .text_color(color)
                .child(value.to_string()),
        )
}

fn format_speed_value(speed: f32) -> String {
    format!("{speed:.1}")
}

fn format_optional_speed(speed: Option<f32>) -> String {
    speed
        .map(|speed| format!("{speed:.1} MB/s"))
        .unwrap_or_else(|| "—".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_speed_value_uses_one_decimal_place() {
        assert_eq!(format_speed_value(12.34), "12.3");
        assert_eq!(format_speed_value(0.0), "0.0");
    }

    #[test]
    fn format_optional_speed_uses_placeholder_when_missing() {
        assert_eq!(format_optional_speed(None), "—");
        assert_eq!(format_optional_speed(Some(5.26)), "5.3 MB/s");
    }
}
