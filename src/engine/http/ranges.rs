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

//! Range math for chunked HTTP downloads
//!
//! Clips, merges, subtracts, and counts byte ranges

#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ByteRange {
    start: u64,
    end: u64,
}

impl ByteRange {
    pub(super) fn new(start: u64, end: u64) -> Option<Self> {
        (start < end).then_some(Self { start, end })
    }

    pub(super) fn from_len(start: u64, len: u64) -> Option<Self> {
        let end = start.checked_add(len)?;
        Self::new(start, end)
    }

    pub(super) fn start(self) -> u64 {
        self.start
    }

    pub(super) fn end(self) -> u64 {
        self.end
    }

    pub(super) fn len(self) -> u64 {
        self.end - self.start
    }

    fn touches_or_overlaps(self, other: Self) -> bool {
        self.start <= other.end && other.start <= self.end
    }

    fn merge(self, other: Self) -> Option<Self> {
        self.touches_or_overlaps(other).then_some(Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        })
    }

    pub(super) fn intersection(self, other: Self) -> Option<Self> {
        Self::new(self.start.max(other.start), self.end.min(other.end))
    }

    fn split_at(self, offset: u64) -> Option<(Self, Self)> {
        if offset <= self.start || offset >= self.end {
            return None;
        }
        Some((
            Self {
                start: self.start,
                end: offset,
            },
            Self {
                start: offset,
                end: self.end,
            },
        ))
    }

    pub(super) fn subtract(self, other: Self) -> Vec<Self> {
        let Some(overlap) = self.intersection(other) else {
            return vec![self];
        };

        let mut ranges = Vec::with_capacity(2);
        if self.start < overlap.start {
            ranges.push(Self {
                start: self.start,
                end: overlap.start,
            });
        }
        if overlap.end < self.end {
            ranges.push(Self {
                start: overlap.end,
                end: self.end,
            });
        }
        ranges
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct RangeSet {
    ranges: Vec<ByteRange>,
}

impl RangeSet {
    pub(super) fn new() -> Self {
        Self::default()
    }

    fn from_total(total_bytes: u64) -> Self {
        let mut set = Self::new();
        if let Some(range) = ByteRange::new(0, total_bytes) {
            set.ranges.push(range);
        }
        set
    }

    pub(super) fn from_ranges(ranges: impl IntoIterator<Item = ByteRange>) -> Self {
        let mut set = Self::new();
        for range in ranges {
            set.insert(range);
        }
        set
    }

    pub(super) fn remaining_from_completed(total_bytes: u64, completed: &Self) -> Self {
        let mut remaining = Self::from_total(total_bytes);
        for &range in completed.ranges() {
            remaining.subtract(range);
        }
        remaining
    }

    pub(super) fn ranges(&self) -> &[ByteRange] {
        &self.ranges
    }

    pub(super) fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub(super) fn total_len(&self) -> u64 {
        self.ranges.iter().map(|range| range.len()).sum()
    }

    pub(super) fn insert_and_count_new(&mut self, range: ByteRange) -> u64 {
        let before = self.total_len();
        self.insert(range);
        self.total_len().saturating_sub(before)
    }

    pub(super) fn insert(&mut self, range: ByteRange) {
        self.ranges.push(range);
        self.normalize();
    }

    pub(super) fn subtract(&mut self, range: ByteRange) {
        self.ranges = self
            .ranges
            .drain(..)
            .flat_map(|existing| existing.subtract(range))
            .collect();
        self.normalize();
    }

    fn normalize(&mut self) {
        self.ranges.sort_by_key(|range| range.start);

        let mut merged: Vec<ByteRange> = Vec::with_capacity(self.ranges.len());
        for range in self.ranges.drain(..) {
            if let Some(last) = merged.last_mut()
                && let Some(combined) = last.merge(range)
            {
                *last = combined;
                continue;
            }
            merged.push(range);
        }
        self.ranges = merged;
    }
}

#[cfg(test)]
mod tests {
    use super::{ByteRange, RangeSet};

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    #[test]
    fn byte_range_rejects_empty_ranges() {
        assert_eq!(ByteRange::new(10, 10), None);
        assert_eq!(ByteRange::new(20, 10), None);
        assert_eq!(ByteRange::from_len(10, 0), None);
    }

    #[test]
    fn byte_range_splits_inside_only() {
        let original = range(10, 20);

        assert_eq!(original.split_at(10), None);
        assert_eq!(original.split_at(20), None);
        assert_eq!(original.split_at(15), Some((range(10, 15), range(15, 20))));
    }

    #[test]
    fn byte_range_subtracts_overlap() {
        assert_eq!(
            range(0, 100).subtract(range(20, 40)),
            vec![range(0, 20), range(40, 100)]
        );
        assert_eq!(range(0, 100).subtract(range(0, 50)), vec![range(50, 100)]);
        assert_eq!(range(0, 100).subtract(range(50, 100)), vec![range(0, 50)]);
        assert_eq!(range(0, 100).subtract(range(100, 200)), vec![range(0, 100)]);
        assert!(range(0, 100).subtract(range(0, 100)).is_empty());
    }

    #[test]
    fn range_set_normalizes_overlaps_and_touching_ranges() {
        let set = RangeSet::from_ranges([
            range(40, 50),
            range(0, 10),
            range(10, 20),
            range(18, 45),
            range(80, 90),
        ]);

        assert_eq!(set.ranges(), &[range(0, 50), range(80, 90)]);
        assert_eq!(set.total_len(), 60);
    }

    #[test]
    fn range_set_counts_only_new_bytes() {
        let mut set = RangeSet::new();

        assert_eq!(set.insert_and_count_new(range(0, 100)), 100);
        assert_eq!(set.insert_and_count_new(range(50, 150)), 50);
        assert_eq!(set.insert_and_count_new(range(25, 75)), 0);
        assert_eq!(set.ranges(), &[range(0, 150)]);
        assert_eq!(set.total_len(), 150);
    }

    #[test]
    fn range_set_subtract_can_split_ranges() {
        let mut set = RangeSet::from_ranges([range(0, 100), range(200, 300)]);

        set.subtract(range(25, 250));

        assert_eq!(set.ranges(), &[range(0, 25), range(250, 300)]);
        assert_eq!(set.total_len(), 75);
    }

    #[test]
    fn remaining_ranges_are_total_minus_completed_ranges() {
        let completed = RangeSet::from_ranges([range(0, 10), range(20, 40), range(35, 50)]);

        let remaining = RangeSet::remaining_from_completed(60, &completed);

        assert_eq!(remaining.ranges(), &[range(10, 20), range(50, 60)]);
        assert_eq!(remaining.total_len(), 20);
    }
}
