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
use gpui::{App, Background, Hsla, PathBuilder, Window, canvas, div, point, prelude::*, px, rgba};
use rust_i18n::t;

#[derive(IntoElement)]
pub struct StatsBar {
    pub download_samples: Vec<f32>,
    pub upload_samples: Vec<f32>,
    pub download_speed: f32,
    pub upload_speed: f32,
    pub active_count: usize,
    pub finished_count: usize,
    pub queued_count: usize,
}

impl RenderOnce for StatsBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let dl_speed = format!("{:.1}", self.download_speed);
        let ul_speed = format!("{:.1}", self.upload_speed);

        div()
            .flex()
            .flex_col()
            .gap(px(Spacing::SECTION_GAP))
            .p(px(Chrome::STATS_CARD_PADDING))
            .rounded(px(Chrome::PANEL_RADIUS))
            .border_1()
            .border_color(Colors::border())
            .bg(Colors::card())
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(Colors::muted_foreground())
                            .child(t!("stats.title").to_string()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(Colors::muted_foreground())
                            .child(t!("stats.window").to_string()),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(Chrome::STATS_GRAPH_HEIGHT))
                    .child(network_graph(self.download_samples, self.upload_samples)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.0))
                    .child(speed_label("↓", &dl_speed, "MB/s", Colors::active().into()))
                    .child(speed_label(
                        "↑",
                        &ul_speed,
                        "MB/s",
                        Colors::finished().into(),
                    ))
                    .child(div().w(px(1.0)).h(px(14.0)).bg(Colors::border()))
                    .child(stat_pill(
                        &t!("stats.active").to_string(),
                        &self.active_count.to_string(),
                        Colors::active().into(),
                    ))
                    .child(stat_pill(
                        &t!("stats.finished").to_string(),
                        &self.finished_count.to_string(),
                        Colors::finished().into(),
                    ))
                    .child(stat_pill(
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

fn network_graph(download: Vec<f32>, upload: Vec<f32>) -> impl IntoElement {
    let dl_line = Colors::active();
    let dl_fill = Colors::active_dim();
    let ul_line = Colors::finished();
    let ul_fill = rgba(0x4A90D918);
    let grid = rgba(0xffffff08);

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

            let combined = download.iter().chain(upload.iter()).cloned();
            let max = combined.fold(0.0_f32, f32::max).max(1.0);

            let to_pts = |samples: &[f32]| -> Vec<(f32, f32)> {
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
            };

            let dl = to_pts(&download);
            let ul = to_pts(&upload);

            // Horizontal grid guides only
            for frac in [0.25_f32, 0.5, 0.75] {
                hline(window, gx, gx + gw, gy + gh * frac, grid);
            }

            // Upload (drawn first, sits behind download)
            smooth_area(window, &ul, gx, gy + gh, gx + gw, ul_fill);
            smooth_stroke(window, &ul, 1.0, ul_line);

            // Download
            smooth_area(window, &dl, gx, gy + gh, gx + gw, dl_fill);
            smooth_stroke(window, &dl, 1.5, dl_line);

            // Live indicator, single dot at latest value
            if let Some(&(cx, cy)) = dl.last() {
                dot(window, cx, cy, 3.0, dl_line);
            }
            if let Some(&(cx, cy)) = ul.last() {
                dot(window, cx, cy, 2.5, ul_line);
            }
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

/// Evaluate cubic bezier at parameter t ∈ [0, 1].
fn bezier_at(
    p0: (f32, f32),
    cp1: (f32, f32),
    cp2: (f32, f32),
    p1: (f32, f32),
    t: f32,
) -> (f32, f32) {
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    let mt3 = mt2 * mt;
    let t2 = t * t;
    let t3 = t2 * t;
    (
        mt3 * p0.0 + 3.0 * mt2 * t * cp1.0 + 3.0 * mt * t2 * cp2.0 + t3 * p1.0,
        mt3 * p0.1 + 3.0 * mt2 * t * cp1.1 + 3.0 * mt * t2 * cp2.1 + t3 * p1.1,
    )
}

/// Sample the catmull-rom spline at high resolution for ribbon construction.
fn sample_spline(pts: &[(f32, f32)], steps_per_seg: usize) -> Vec<(f32, f32)> {
    if pts.len() < 2 {
        return pts.to_vec();
    }
    let mut out = Vec::with_capacity((pts.len() - 1) * steps_per_seg + 1);
    for i in 0..pts.len() - 1 {
        let (cp1, cp2) = catmull_rom_cp(pts, i);
        let end = if i == pts.len() - 2 {
            steps_per_seg + 1
        } else {
            steps_per_seg
        };
        for j in 0..end {
            let t = j as f32 / steps_per_seg as f32;
            out.push(bezier_at(pts[i], cp1, cp2, pts[i + 1], t));
        }
    }
    out
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

/// Smooth line rendered as a filled ribbon polygon. Samples the spline
/// at high resolution, offsets each sample by its normal, and builds
/// a single closed path. One paint_path call.
fn smooth_stroke(
    window: &mut Window,
    pts: &[(f32, f32)],
    thickness: f32,
    color: impl Into<Background>,
) {
    let samples = sample_spline(pts, 8);
    if samples.len() < 2 {
        return;
    }

    let half = thickness / 2.0;
    let n = samples.len();
    let mut top = Vec::with_capacity(n);
    let mut bot = Vec::with_capacity(n);

    for i in 0..n {
        let (dx, dy) = if i == 0 {
            (samples[1].0 - samples[0].0, samples[1].1 - samples[0].1)
        } else if i == n - 1 {
            (
                samples[i].0 - samples[i - 1].0,
                samples[i].1 - samples[i - 1].1,
            )
        } else {
            (
                samples[i + 1].0 - samples[i - 1].0,
                samples[i + 1].1 - samples[i - 1].1,
            )
        };
        let len = (dx * dx + dy * dy).sqrt().max(0.001);
        let nx = -dy / len * half;
        let ny = dx / len * half;

        top.push((samples[i].0 + nx, samples[i].1 + ny));
        bot.push((samples[i].0 - nx, samples[i].1 - ny));
    }

    let mut p = PathBuilder::fill();
    p.move_to(point(px(top[0].0), px(top[0].1)));
    for &(x, y) in &top[1..] {
        p.line_to(point(px(x), px(y)));
    }
    for &(x, y) in bot.iter().rev() {
        p.line_to(point(px(x), px(y)));
    }
    p.close();
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

fn dot(window: &mut Window, cx: f32, cy: f32, r: f32, color: impl Into<Background>) {
    let mut p = PathBuilder::fill();
    p.move_to(point(px(cx + r), px(cy)));
    p.arc_to(
        point(px(r), px(r)),
        px(0.0),
        false,
        false,
        point(px(cx - r), px(cy)),
    );
    p.arc_to(
        point(px(r), px(r)),
        px(0.0),
        false,
        false,
        point(px(cx + r), px(cy)),
    );
    p.close();
    if let Ok(path) = p.build() {
        window.paint_path(path, color);
    }
}

// ---------------------------------------------------------------------------
// Layout helpers
// ---------------------------------------------------------------------------

fn speed_label(direction: &str, value: &str, unit: &str, color: Hsla) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.0))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(color)
                .child(direction.to_string()),
        )
        .child(
            div()
                .text_base()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(color)
                .child(value.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(Colors::muted_foreground())
                .child(unit.to_string()),
        )
}

fn stat_pill(label: &str, value: &str, color: Hsla) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(color))
        .child(
            div()
                .text_sm()
                .text_color(Colors::muted_foreground())
                .child(label.to_string()),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(Colors::foreground())
                .child(value.to_string()),
        )
}
