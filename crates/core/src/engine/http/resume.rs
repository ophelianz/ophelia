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

//! Resume helpers for chunked HTTP downloads
//!
//! Turns saved chunk rows into completed and missing ranges before workers start

#![allow(dead_code)]

use crate::engine::types::ChunkSnapshot;

use super::ranges::{ByteRange, RangeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RangeResumeSnapshot {
    total_bytes: u64,
    remaining: RangeSet,
}

impl RangeResumeSnapshot {
    pub(super) fn from_remaining_ranges(
        total_bytes: u64,
        remaining: impl IntoIterator<Item = ByteRange>,
    ) -> Self {
        let remaining = Self::clip_to_total(total_bytes, remaining);
        Self {
            total_bytes,
            remaining,
        }
    }

    pub(super) fn from_completed(total_bytes: u64, completed: RangeSet) -> Self {
        let remaining = RangeSet::remaining_from_completed(total_bytes, &completed);
        Self {
            total_bytes,
            remaining,
        }
    }

    pub(super) fn from_old_chunks(chunks: &[ChunkSnapshot]) -> Option<Self> {
        let total_bytes = chunks
            .iter()
            .filter(|chunk| chunk.start < chunk.end)
            .map(|chunk| chunk.end)
            .max()?;

        let mut completed = RangeSet::new();
        for chunk in chunks.iter().filter(|chunk| chunk.start < chunk.end) {
            let chunk_len = chunk.end - chunk.start;
            let downloaded = chunk.downloaded.min(chunk_len);
            let Some(completed_range) = ByteRange::from_len(chunk.start, downloaded)
                .and_then(|range| ByteRange::new(range.start(), range.end().min(total_bytes)))
            else {
                continue;
            };

            completed.insert(completed_range);
        }

        Some(Self::from_completed(total_bytes, completed))
    }

    pub(super) fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub(super) fn downloaded_bytes(&self) -> u64 {
        self.total_bytes.saturating_sub(self.remaining_bytes())
    }

    pub(super) fn remaining_bytes(&self) -> u64 {
        self.remaining.total_len().min(self.total_bytes)
    }

    pub(super) fn remaining_ranges(&self) -> &[ByteRange] {
        self.remaining.ranges()
    }

    pub(super) fn chunk_snapshots(&self) -> Vec<ChunkSnapshot> {
        let completed = RangeSet::remaining_from_completed(self.total_bytes, &self.remaining);
        let mut snapshots = Vec::new();
        snapshots.extend(completed.ranges().iter().map(|range| ChunkSnapshot {
            start: range.start(),
            end: range.end(),
            downloaded: range.len(),
        }));
        snapshots.extend(self.remaining.ranges().iter().map(|range| ChunkSnapshot {
            start: range.start(),
            end: range.end(),
            downloaded: 0,
        }));
        snapshots.sort_by_key(|snapshot| (snapshot.start, snapshot.end));
        snapshots
    }

    pub(super) fn is_complete(&self) -> bool {
        self.remaining.is_empty()
    }

    fn clip_to_total(total_bytes: u64, ranges: impl IntoIterator<Item = ByteRange>) -> RangeSet {
        let Some(total_range) = ByteRange::new(0, total_bytes) else {
            return RangeSet::new();
        };

        RangeSet::from_ranges(
            ranges
                .into_iter()
                .filter_map(|range| range.intersection(total_range)),
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::{
        http::{
            ranges::{ByteRange, RangeSet},
            resume::RangeResumeSnapshot,
        },
        types::ChunkSnapshot,
    };

    fn chunk(start: u64, end: u64, downloaded: u64) -> ChunkSnapshot {
        ChunkSnapshot {
            start,
            end,
            downloaded,
        }
    }

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    #[test]
    fn old_chunks_use_largest_end_not_last_row() {
        let snapshot =
            RangeResumeSnapshot::from_old_chunks(&[chunk(0, 1000, 500), chunk(500, 750, 0)])
                .unwrap();

        assert_eq!(snapshot.total_bytes(), 1000);
        assert_eq!(snapshot.downloaded_bytes(), 500);
        assert_eq!(snapshot.remaining_ranges(), &[range(500, 1000)]);
    }

    #[test]
    fn overlapping_old_chunks_do_not_double_count_completed_bytes() {
        let snapshot =
            RangeResumeSnapshot::from_old_chunks(&[chunk(0, 100, 80), chunk(50, 150, 80)]).unwrap();

        assert_eq!(snapshot.total_bytes(), 150);
        assert_eq!(snapshot.downloaded_bytes(), 130);
        assert_eq!(snapshot.remaining_ranges(), &[range(130, 150)]);
    }

    #[test]
    fn old_chunk_downloaded_bytes_are_clamped_to_chunk_length() {
        let snapshot = RangeResumeSnapshot::from_old_chunks(&[chunk(10, 20, 50)]).unwrap();

        assert_eq!(snapshot.total_bytes(), 20);
        assert_eq!(snapshot.downloaded_bytes(), 10);
        assert_eq!(snapshot.remaining_ranges(), &[range(0, 10)]);
    }

    #[test]
    fn empty_or_invalid_old_chunks_have_no_snapshot() {
        assert!(RangeResumeSnapshot::from_old_chunks(&[]).is_none());
        assert!(
            RangeResumeSnapshot::from_old_chunks(&[chunk(10, 10, 1), chunk(20, 10, 1)]).is_none()
        );
    }

    #[test]
    fn remaining_ranges_are_normalized_and_non_overlapping() {
        let snapshot = RangeResumeSnapshot::from_old_chunks(&[
            chunk(0, 100, 20),
            chunk(50, 100, 10),
            chunk(100, 200, 0),
        ])
        .unwrap();

        assert_eq!(snapshot.total_bytes(), 200);
        assert_eq!(
            snapshot.remaining_ranges(),
            &[range(20, 50), range(60, 200)]
        );
    }

    #[test]
    fn completed_old_chunks_have_no_remaining_ranges() {
        let snapshot =
            RangeResumeSnapshot::from_old_chunks(&[chunk(0, 100, 100), chunk(100, 200, 100)])
                .unwrap();

        assert_eq!(snapshot.total_bytes(), 200);
        assert_eq!(snapshot.downloaded_bytes(), 200);
        assert!(snapshot.is_complete());
        assert!(snapshot.remaining_ranges().is_empty());
    }

    #[test]
    fn from_remaining_ranges_clips_ranges_to_the_file_size() {
        let snapshot =
            RangeResumeSnapshot::from_remaining_ranges(100, [range(40, 80), range(80, 150)]);

        assert_eq!(snapshot.remaining_ranges(), &[range(40, 100)]);
        assert_eq!(snapshot.remaining_bytes(), 60);
    }

    #[test]
    fn from_completed_derives_the_missing_ranges() {
        let completed = RangeSet::from_ranges([range(0, 25), range(50, 100)]);
        let snapshot = RangeResumeSnapshot::from_completed(100, completed);

        assert_eq!(snapshot.remaining_ranges(), &[range(25, 50)]);
        assert_eq!(snapshot.downloaded_bytes(), 75);
    }

    #[test]
    fn chunk_snapshots_describe_completed_and_missing_ranges() {
        let snapshot =
            RangeResumeSnapshot::from_old_chunks(&[chunk(0, 100, 20), chunk(80, 120, 10)]).unwrap();

        let snapshots = snapshot
            .chunk_snapshots()
            .into_iter()
            .map(|chunk| (chunk.start, chunk.end, chunk.downloaded))
            .collect::<Vec<_>>();
        assert_eq!(
            snapshots,
            vec![(0, 20, 20), (20, 80, 0), (80, 90, 10), (90, 120, 0)]
        );
    }
}
