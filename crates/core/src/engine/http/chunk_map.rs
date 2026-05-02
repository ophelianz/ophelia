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

//! Chunk-map snapshots for the Transfers view
//!
//! Turns completed byte ranges into fixed-width cells

use crate::engine::{ChunkMapCellState, DirectChunkMapSnapshot};

pub(crate) const HTTP_CHUNK_MAP_CELLS: usize = 128;

pub(super) fn snapshot_from_covered_ranges(
    total_bytes: u64,
    covered_ranges: impl IntoIterator<Item = (u64, u64)>,
) -> DirectChunkMapSnapshot {
    let merged = merge_ranges(covered_ranges);
    let mut cells = Vec::with_capacity(HTTP_CHUNK_MAP_CELLS);

    for index in 0..HTTP_CHUNK_MAP_CELLS {
        let cell_start = scaled_boundary(total_bytes, index);
        let cell_end = scaled_boundary(total_bytes, index + 1);

        if cell_end <= cell_start {
            cells.push(ChunkMapCellState::Empty);
            continue;
        }

        let covered = covered_len_in_range(&merged, cell_start, cell_end);
        let cell_width = cell_end - cell_start;
        let state = if covered == 0 {
            ChunkMapCellState::Empty
        } else if covered >= cell_width {
            ChunkMapCellState::Complete
        } else {
            ChunkMapCellState::Partial
        };
        cells.push(state);
    }

    DirectChunkMapSnapshot { total_bytes, cells }
}

fn scaled_boundary(total_bytes: u64, step: usize) -> u64 {
    ((total_bytes as u128 * step as u128) / HTTP_CHUNK_MAP_CELLS as u128) as u64
}

fn merge_ranges(covered_ranges: impl IntoIterator<Item = (u64, u64)>) -> Vec<(u64, u64)> {
    let mut ranges: Vec<(u64, u64)> = covered_ranges
        .into_iter()
        .filter(|(start, end)| end > start)
        .collect();
    ranges.sort_unstable_by_key(|&(start, _)| start);

    let mut merged = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        if let Some((_, previous_end)) = merged.last_mut()
            && start <= *previous_end
        {
            *previous_end = (*previous_end).max(end);
            continue;
        }
        merged.push((start, end));
    }

    merged
}

fn covered_len_in_range(merged: &[(u64, u64)], start: u64, end: u64) -> u64 {
    let mut covered = 0u64;
    for &(range_start, range_end) in merged {
        if range_end <= start {
            continue;
        }
        if range_start >= end {
            break;
        }
        let overlap_start = range_start.max(start);
        let overlap_end = range_end.min(end);
        if overlap_end > overlap_start {
            covered += overlap_end - overlap_start;
        }
    }
    covered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_cells(snapshot: &DirectChunkMapSnapshot) -> usize {
        snapshot
            .cells
            .iter()
            .filter(|&&cell| cell == ChunkMapCellState::Complete)
            .count()
    }

    #[test]
    fn empty_coverage_yields_empty_cells() {
        let snapshot = snapshot_from_covered_ranges(1_280, std::iter::empty());
        assert_eq!(snapshot.cells.len(), HTTP_CHUNK_MAP_CELLS);
        assert!(
            snapshot
                .cells
                .iter()
                .all(|&cell| cell == ChunkMapCellState::Empty)
        );
    }

    #[test]
    fn full_coverage_yields_complete_cells() {
        let snapshot = snapshot_from_covered_ranges(128, [(0, 128)]);
        assert!(
            snapshot
                .cells
                .iter()
                .all(|&cell| cell == ChunkMapCellState::Complete)
        );
    }

    #[test]
    fn partial_coverage_marks_partial_cells() {
        let snapshot = snapshot_from_covered_ranges(1_280, [(0, 5)]);
        assert_eq!(snapshot.cells[0], ChunkMapCellState::Partial);
        assert!(
            snapshot.cells[1..]
                .iter()
                .all(|&cell| cell == ChunkMapCellState::Empty)
        );
    }

    #[test]
    fn resumed_chunk_progress_seeds_initial_coverage() {
        let snapshot = snapshot_from_covered_ranges(1_280, [(0, 640), (640, 965)]);
        assert_eq!(complete_cells(&snapshot), 96);
        assert_eq!(snapshot.cells[96], ChunkMapCellState::Partial);
        assert!(
            snapshot.cells[97..]
                .iter()
                .all(|&cell| cell == ChunkMapCellState::Empty)
        );
    }

    #[test]
    fn overlapping_hedge_ranges_do_not_overcount() {
        let snapshot = snapshot_from_covered_ranges(128, [(0, 64), (32, 96)]);
        assert_eq!(complete_cells(&snapshot), 96);
        assert!(
            snapshot.cells[96..]
                .iter()
                .all(|&cell| cell == ChunkMapCellState::Empty)
        );
    }

    #[test]
    fn stolen_ranges_render_as_contiguous_coverage() {
        let snapshot = snapshot_from_covered_ranges(128, [(0, 32), (32, 64), (64, 96)]);
        assert_eq!(complete_cells(&snapshot), 96);
        assert!(
            snapshot.cells[96..]
                .iter()
                .all(|&cell| cell == ChunkMapCellState::Empty)
        );
    }
}
