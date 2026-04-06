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

use std::ops::Range;

use gpui::{Along, Axis, Bounds, Context, ElementId, EventEmitter, IsZero, Pixels, px};

mod panel;
mod resize_handle;

pub use panel::*;
pub(crate) use resize_handle::*;

pub const PANEL_MIN_SIZE: Pixels = px(100.0);

pub fn h_resizable(id: impl Into<ElementId>) -> ResizablePanelGroup {
    ResizablePanelGroup::new(id).axis(Axis::Horizontal)
}

pub fn v_resizable(id: impl Into<ElementId>) -> ResizablePanelGroup {
    ResizablePanelGroup::new(id).axis(Axis::Vertical)
}

pub fn resizable_panel() -> ResizablePanel {
    ResizablePanel::new()
}

#[derive(Debug, Clone)]
pub struct ResizableState {
    axis: Axis,
    panels: Vec<ResizablePanelState>,
    sizes: Vec<Pixels>,
    pub(crate) resizing_panel_ix: Option<usize>,
    bounds: Bounds<Pixels>,
}

impl Default for ResizableState {
    fn default() -> Self {
        Self {
            axis: Axis::Horizontal,
            panels: Vec::new(),
            sizes: Vec::new(),
            resizing_panel_ix: None,
            bounds: Bounds::default(),
        }
    }
}

impl ResizableState {
    pub fn sizes(&self) -> &Vec<Pixels> {
        &self.sizes
    }

    pub(crate) fn sync_panels_count(
        &mut self,
        axis: Axis,
        panels_count: usize,
        cx: &mut Context<Self>,
    ) {
        if self.sync_panels_count_internal(axis, panels_count) {
            cx.notify();
        }
    }

    pub(crate) fn update_panel_size(
        &mut self,
        panel_ix: usize,
        bounds: Bounds<Pixels>,
        size_range: Range<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.update_panel_size_internal(panel_ix, bounds, size_range) {
            cx.notify();
        }
    }

    pub(crate) fn done_resizing(&mut self, cx: &mut Context<Self>) {
        if let Some(event) = self.done_resizing_internal() {
            cx.emit(event);
            cx.notify();
        }
    }

    #[inline]
    pub(crate) fn container_size(&self) -> Pixels {
        self.bounds.size.along(self.axis)
    }

    pub(crate) fn set_bounds(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        let size_changed = self.bounds.size.along(self.axis) != bounds.size.along(self.axis);
        self.bounds = bounds;
        if size_changed && self.adjust_to_container_size_internal() {
            cx.notify();
        }
    }

    pub(crate) fn resize_panel(&mut self, ix: usize, size: Pixels, cx: &mut Context<Self>) {
        if self.resize_panel_internal(ix, size) {
            cx.notify();
        }
    }

    fn panel_size_range(&self, ix: usize) -> Range<Pixels> {
        self.panels
            .get(ix)
            .map(|panel| panel.size_range.clone())
            .unwrap_or(PANEL_MIN_SIZE..Pixels::MAX)
    }

    fn sync_real_panel_sizes(&mut self) {
        for (i, panel) in self.panels.iter().enumerate() {
            if i < self.sizes.len() {
                self.sizes[i] = panel.bounds.size.along(self.axis);
            }
        }
    }

    #[cfg(test)]
    fn insert_panel_internal(&mut self, size: Option<Pixels>, ix: Option<usize>) -> bool {
        let panel_state = ResizablePanelState {
            size,
            size_range: PANEL_MIN_SIZE..Pixels::MAX,
            ..Default::default()
        };
        let size = size.unwrap_or(PANEL_MIN_SIZE);

        if let Some(ix) = ix {
            self.panels.insert(ix, panel_state);
            self.sizes.insert(ix, size);
        } else {
            self.panels.push(panel_state);
            self.sizes.push(size);
        }

        self.adjust_to_container_size_internal()
    }

    fn sync_panels_count_internal(&mut self, axis: Axis, panels_count: usize) -> bool {
        let mut changed = self.axis != axis;
        self.axis = axis;

        if panels_count > self.panels.len() {
            let diff = panels_count - self.panels.len();
            self.panels
                .extend((0..diff).map(|_| ResizablePanelState::default()));
            self.sizes.extend((0..diff).map(|_| PANEL_MIN_SIZE));
            changed = true;
        }

        if panels_count < self.panels.len() {
            self.panels.truncate(panels_count);
            self.sizes.truncate(panels_count);
            changed = true;
        }

        if changed {
            return self.adjust_to_container_size_internal() || changed;
        }

        false
    }

    fn update_panel_size_internal(
        &mut self,
        panel_ix: usize,
        bounds: Bounds<Pixels>,
        size_range: Range<Pixels>,
    ) -> bool {
        let Some(panel) = self.panels.get_mut(panel_ix) else {
            return false;
        };

        let mut changed = panel.bounds != bounds || panel.size_range != size_range;
        let size = bounds.size.along(self.axis);

        if self
            .sizes
            .get(panel_ix)
            .is_some_and(|current| f32::from(*current) == f32::from(PANEL_MIN_SIZE))
        {
            self.sizes[panel_ix] = size;
            panel.size = Some(size);
            changed = true;
        }

        panel.bounds = bounds;
        panel.size_range = size_range;
        changed
    }

    fn done_resizing_internal(&mut self) -> Option<ResizablePanelEvent> {
        self.resizing_panel_ix
            .take()
            .map(|_| ResizablePanelEvent::Resized)
    }

    fn resize_panel_internal(&mut self, ix: usize, size: Pixels) -> bool {
        if ix >= self.sizes.len().saturating_sub(1) {
            return false;
        }

        self.sync_real_panel_sizes();
        let old_sizes = self.sizes.clone();
        let move_changed = size - old_sizes[ix];
        if move_changed == px(0.0) {
            return false;
        }

        let size_range = self.panel_size_range(ix);
        let new_size = size.clamp(size_range.start, size_range.end);
        let is_expand = move_changed > px(0.0);

        let main_ix = ix;
        let mut new_sizes = old_sizes.clone();
        let mut cursor_ix = ix;

        if is_expand {
            let mut changed = new_size - old_sizes[cursor_ix];
            new_sizes[cursor_ix] = new_size;

            while changed > px(0.0) && cursor_ix < old_sizes.len() - 1 {
                cursor_ix += 1;
                let size_range = self.panel_size_range(cursor_ix);
                let available_size = (new_sizes[cursor_ix] - size_range.start).max(px(0.0));
                let to_reduce = changed.min(available_size);
                new_sizes[cursor_ix] -= to_reduce;
                changed -= to_reduce;
            }
        } else {
            let mut changed = old_sizes[cursor_ix] - new_size;
            new_sizes[cursor_ix] = new_size;

            while changed > px(0.0) && cursor_ix < old_sizes.len() - 1 {
                cursor_ix += 1;
                let size_range = self.panel_size_range(cursor_ix);
                let available_size = (size_range.end - new_sizes[cursor_ix]).max(px(0.0));
                let to_grow = changed.min(available_size);
                new_sizes[cursor_ix] += to_grow;
                changed -= to_grow;
            }
        }

        let container_size = self.container_size();
        let total_size = px(sum_sizes(&new_sizes));
        if total_size > container_size {
            let overflow = total_size - container_size;
            new_sizes[main_ix] = (new_sizes[main_ix] - overflow).max(size_range.start);
        } else if total_size < container_size {
            let deficit = container_size - total_size;
            if let Some(last) = new_sizes.last_mut() {
                *last += deficit;
            }
        }

        for (panel, size) in self.panels.iter_mut().zip(new_sizes.iter().copied()) {
            panel.size = Some(size);
        }
        self.sizes = new_sizes;
        true
    }

    fn adjust_to_container_size_internal(&mut self) -> bool {
        if self.container_size().is_zero() || self.panels.is_empty() {
            return false;
        }

        let container_size = self.container_size();
        let current_sizes = if self.sizes.is_empty() {
            vec![PANEL_MIN_SIZE; self.panels.len()]
        } else {
            self.sizes.clone()
        };
        let ranges = self
            .panels
            .iter()
            .map(|panel| panel.size_range.clone())
            .collect::<Vec<_>>();
        let adjusted = distribute_sizes_to_target(&current_sizes, &ranges, container_size);

        if adjusted == self.sizes {
            return false;
        }

        for (panel, size) in self.panels.iter_mut().zip(adjusted.iter().copied()) {
            panel.size = Some(size);
        }
        self.sizes = adjusted;
        true
    }
}

impl EventEmitter<ResizablePanelEvent> for ResizableState {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizablePanelEvent {
    Resized,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ResizablePanelState {
    pub size: Option<Pixels>,
    pub size_range: Range<Pixels>,
    pub bounds: Bounds<Pixels>,
}

fn distribute_sizes_to_target(
    sizes: &[Pixels],
    ranges: &[Range<Pixels>],
    target: Pixels,
) -> Vec<Pixels> {
    if sizes.is_empty() {
        return Vec::new();
    }

    let mut adjusted = sizes
        .iter()
        .copied()
        .zip(ranges.iter())
        .map(|(size, range)| size.clamp(range.start, range.end))
        .collect::<Vec<_>>();

    let min_total = px(ranges
        .iter()
        .map(|range| f32::from(range.start))
        .sum::<f32>());
    if target <= min_total {
        return ranges.iter().map(|range| range.start).collect();
    }

    let mut current_total = px(sum_sizes(&adjusted));
    let original_weights = sizes
        .iter()
        .map(|size| f32::from(*size).max(1.0))
        .collect::<Vec<_>>();

    while current_total < target {
        let remaining = target - current_total;
        let growable = adjusted
            .iter()
            .enumerate()
            .filter_map(|(ix, size)| {
                let capacity = ranges[ix].end - *size;
                (capacity > px(0.0)).then_some((ix, capacity))
            })
            .collect::<Vec<_>>();

        if growable.is_empty() {
            break;
        }

        let total_weight = growable
            .iter()
            .map(|(ix, _)| original_weights[*ix])
            .sum::<f32>()
            .max(1.0);
        let mut changed = px(0.0);

        for (ix, capacity) in growable {
            let share = remaining * (original_weights[ix] / total_weight);
            let delta = share.min(capacity);
            adjusted[ix] += delta;
            changed += delta;
        }

        if changed == px(0.0) {
            break;
        }
        current_total += changed;
    }

    while current_total > target {
        let overflow = current_total - target;
        let shrinkable = adjusted
            .iter()
            .enumerate()
            .filter_map(|(ix, size)| {
                let capacity = *size - ranges[ix].start;
                (capacity > px(0.0)).then_some((ix, capacity))
            })
            .collect::<Vec<_>>();

        if shrinkable.is_empty() {
            break;
        }

        let total_weight = shrinkable
            .iter()
            .map(|(ix, _)| f32::from(adjusted[*ix]).max(1.0))
            .sum::<f32>()
            .max(1.0);
        let mut changed = px(0.0);

        for (ix, capacity) in shrinkable {
            let weight = f32::from(adjusted[ix]).max(1.0) / total_weight;
            let share = overflow * weight;
            let delta = share.min(capacity);
            adjusted[ix] -= delta;
            changed += delta;
        }

        if changed == px(0.0) {
            break;
        }
        current_total -= changed;
    }

    adjusted
}

fn sum_sizes(sizes: &[Pixels]) -> f32 {
    sizes.iter().map(|size| f32::from(*size)).sum::<f32>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Axis, bounds, point, size};

    #[test]
    fn insert_panel_initializes_sizes() {
        let mut state = ResizableState::default();
        state.bounds = bounds(point(px(0.0), px(0.0)), size(px(500.0), px(300.0)));
        assert!(state.insert_panel_internal(Some(px(180.0)), None));
        assert_eq!(state.sizes, vec![px(500.0)]);
    }

    #[test]
    fn sync_panels_count_adds_and_removes_panels() {
        let mut state = ResizableState::default();
        state.bounds = bounds(point(px(0.0), px(0.0)), size(px(600.0), px(400.0)));
        assert!(state.sync_panels_count_internal(Axis::Horizontal, 3));
        assert_eq!(state.sizes.len(), 3);
        assert!(state.sync_panels_count_internal(Axis::Horizontal, 2));
        assert_eq!(state.sizes.len(), 2);
    }

    #[test]
    fn resize_panel_clamps_to_size_range() {
        let mut state = ResizableState::default();
        state.axis = Axis::Horizontal;
        state.bounds = bounds(point(px(0.0), px(0.0)), size(px(600.0), px(300.0)));
        state.panels = vec![
            ResizablePanelState {
                size: Some(px(200.0)),
                size_range: px(180.0)..px(220.0),
                bounds: bounds(point(px(0.0), px(0.0)), size(px(200.0), px(300.0))),
            },
            ResizablePanelState {
                size: Some(px(400.0)),
                size_range: px(200.0)..Pixels::MAX,
                bounds: bounds(point(px(200.0), px(0.0)), size(px(400.0), px(300.0))),
            },
        ];
        state.sizes = vec![px(200.0), px(400.0)];

        assert!(state.resize_panel_internal(0, px(260.0)));
        assert_eq!(state.sizes[0], px(220.0));
        assert_eq!(px(sum_sizes(&state.sizes)), px(600.0));
    }

    #[test]
    fn container_resize_respects_panel_minimums() {
        let mut state = ResizableState::default();
        state.axis = Axis::Horizontal;
        state.bounds = bounds(point(px(0.0), px(0.0)), size(px(600.0), px(300.0)));
        state.panels = vec![
            ResizablePanelState {
                size: Some(px(240.0)),
                size_range: px(200.0)..Pixels::MAX,
                bounds: Bounds::default(),
            },
            ResizablePanelState {
                size: Some(px(360.0)),
                size_range: px(260.0)..Pixels::MAX,
                bounds: Bounds::default(),
            },
        ];
        state.sizes = vec![px(240.0), px(360.0)];

        state.bounds = bounds(point(px(0.0), px(0.0)), size(px(500.0), px(300.0)));
        assert!(state.adjust_to_container_size_internal());
        assert!(state.sizes[0] >= px(200.0));
        assert!(state.sizes[1] >= px(260.0));
        assert_eq!(px(sum_sizes(&state.sizes)), px(500.0));
    }

    #[test]
    fn done_resizing_returns_resized_event() {
        let mut state = ResizableState::default();
        state.resizing_panel_ix = Some(0);
        assert_eq!(
            state.done_resizing_internal(),
            Some(ResizablePanelEvent::Resized)
        );
        assert_eq!(state.resizing_panel_ix, None);
    }
}
