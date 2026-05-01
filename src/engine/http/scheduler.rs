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

//! Range scheduler for chunked HTTP downloads
//!
//! Tracks pending, active, completed, and hedged ranges
//! Stealing cuts the back half off one active range and puts it back into pending work

#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};

use super::events::{SchedulerAction, WorkerEvent, WorkerFailure};
use super::ranges::{ByteRange, RangeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct AttemptId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct HedgeGroupId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptRole {
    Normal,
    HedgeOriginal { group: HedgeGroupId },
    HedgeDuplicate { group: HedgeGroupId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ActiveAttempt {
    id: AttemptId,
    range: ByteRange,
    current: u64,
    stop_at: u64,
    role: AttemptRole,
}

impl ActiveAttempt {
    pub(super) fn id(self) -> AttemptId {
        self.id
    }

    pub(super) fn range(self) -> ByteRange {
        self.range
    }

    pub(super) fn stop_at(self) -> u64 {
        self.stop_at
    }

    pub(super) fn remaining_range(self) -> Option<ByteRange> {
        ByteRange::new(self.current, self.stop_at)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StealResult {
    pub(super) victim: AttemptId,
    pub(super) stolen: ByteRange,
    pub(super) victim_stop_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FinishResult {
    pub(super) cancel_loser: Option<AttemptId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AttemptFailure {
    Retryable,
    HedgeLost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HedgeGroup {
    original: AttemptId,
    duplicate: AttemptId,
    winner: Option<AttemptId>,
}

#[derive(Debug, Clone)]
pub(super) struct RangeScheduler {
    total_bytes: u64,
    pending: VecDeque<ByteRange>,
    completed: RangeSet,
    active: HashMap<AttemptId, ActiveAttempt>,
    hedges: HashMap<HedgeGroupId, HedgeGroup>,
    next_attempt: u64,
    next_hedge_group: u64,
}

impl RangeScheduler {
    pub(super) fn new(total_bytes: u64, pending: impl IntoIterator<Item = ByteRange>) -> Self {
        let pending = normalize_pending(total_bytes, pending);
        Self {
            total_bytes,
            pending,
            completed: RangeSet::new(),
            active: HashMap::new(),
            hedges: HashMap::new(),
            next_attempt: 0,
            next_hedge_group: 0,
        }
    }

    pub(super) fn from_completed(total_bytes: u64, completed: RangeSet) -> Self {
        let completed = clip_completed_to_total(total_bytes, completed);
        let pending: Vec<_> = RangeSet::remaining_from_completed(total_bytes, &completed)
            .ranges()
            .to_vec();
        Self::from_completed_and_pending(total_bytes, completed, pending)
    }

    pub(super) fn from_completed_and_pending(
        total_bytes: u64,
        completed: RangeSet,
        pending: impl IntoIterator<Item = ByteRange>,
    ) -> Self {
        let completed = clip_completed_to_total(total_bytes, completed);
        let mut pending = normalize_pending(total_bytes, pending);
        for &completed_range in completed.ranges() {
            pending = pending
                .into_iter()
                .flat_map(|range| range.subtract(completed_range))
                .collect();
        }
        Self {
            total_bytes,
            pending,
            completed,
            active: HashMap::new(),
            hedges: HashMap::new(),
            next_attempt: 0,
            next_hedge_group: 0,
        }
    }

    pub(super) fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub(super) fn downloaded_bytes(&self) -> u64 {
        self.completed.total_len().min(self.total_bytes)
    }

    pub(super) fn is_complete(&self) -> bool {
        RangeSet::remaining_from_completed(self.total_bytes, &self.completed).is_empty()
    }

    pub(super) fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub(super) fn active_len(&self) -> usize {
        self.active.len()
    }

    pub(super) fn pending_ranges(&self) -> impl Iterator<Item = ByteRange> + '_ {
        self.pending.iter().copied()
    }

    pub(super) fn completed_ranges(&self) -> &[ByteRange] {
        self.completed.ranges()
    }

    pub(super) fn active_attempt(&self, id: AttemptId) -> Option<ActiveAttempt> {
        self.active.get(&id).copied()
    }

    pub(super) fn apply_worker_event(&mut self, event: WorkerEvent) -> SchedulerAction {
        match event {
            WorkerEvent::DataReceived { attempt, .. } => {
                if self.active.contains_key(&attempt) {
                    SchedulerAction::Nothing
                } else {
                    SchedulerAction::UnknownAttempt { attempt }
                }
            }
            WorkerEvent::BytesWritten { attempt, written } => {
                let known = self.active.contains_key(&attempt);
                match self.record_progress(attempt, written) {
                    Some(new_bytes) => SchedulerAction::CountedProgress { new_bytes },
                    None if known => SchedulerAction::Nothing,
                    None => SchedulerAction::UnknownAttempt { attempt },
                }
            }
            WorkerEvent::Finished { attempt } => match self.finish_attempt(attempt) {
                Some(FinishResult {
                    cancel_loser: Some(attempt),
                }) => SchedulerAction::CancelAttempt { attempt },
                Some(FinishResult { cancel_loser: None }) => SchedulerAction::Nothing,
                None => SchedulerAction::UnknownAttempt { attempt },
            },
            WorkerEvent::Paused { attempt } => {
                if self.active.contains_key(&attempt) {
                    SchedulerAction::PauseDownload
                } else {
                    SchedulerAction::UnknownAttempt { attempt }
                }
            }
            WorkerEvent::Failed { attempt, failure } => {
                let known = self.active.contains_key(&attempt);
                if !known {
                    return SchedulerAction::UnknownAttempt { attempt };
                }

                let retry_after = failure.retry_after();
                let Some(attempt_failure) = failure.attempt_failure() else {
                    return self.action_for_unrecoverable_failure(attempt, failure);
                };

                match self.fail_attempt(attempt, attempt_failure) {
                    Some(range) => SchedulerAction::Requeued { range, retry_after },
                    None => SchedulerAction::Nothing,
                }
            }
        }
    }

    pub(super) fn start_next_attempt(&mut self) -> Option<ActiveAttempt> {
        let range = self.pending.pop_front()?;
        Some(self.insert_attempt(range, AttemptRole::Normal))
    }

    fn record_progress(&mut self, id: AttemptId, written: ByteRange) -> Option<u64> {
        let attempt = self.active.get_mut(&id)?;
        let allowed = ByteRange::new(attempt.range.start(), attempt.stop_at)?;
        let written = written.intersection(allowed)?;

        attempt.current = attempt.current.max(written.end()).min(attempt.stop_at);
        Some(self.completed.insert_and_count_new(written))
    }

    fn finish_attempt(&mut self, id: AttemptId) -> Option<FinishResult> {
        let attempt = self.active.remove(&id)?;
        let cancel_loser = match attempt.role {
            AttemptRole::Normal => None,
            AttemptRole::HedgeOriginal { group } | AttemptRole::HedgeDuplicate { group } => {
                self.mark_hedge_winner(group, id)
            }
        };
        Some(FinishResult { cancel_loser })
    }

    fn fail_attempt(&mut self, id: AttemptId, failure: AttemptFailure) -> Option<ByteRange> {
        let attempt = self.active.remove(&id)?;
        match failure {
            AttemptFailure::Retryable => self.handle_retryable_failure(attempt),
            AttemptFailure::HedgeLost => {
                self.remove_from_hedge_group(attempt);
                None
            }
        }
    }

    fn action_for_unrecoverable_failure(
        &mut self,
        id: AttemptId,
        failure: WorkerFailure,
    ) -> SchedulerAction {
        if self.handle_unrecoverable_failure(id) {
            SchedulerAction::Nothing
        } else {
            SchedulerAction::FailDownload { failure }
        }
    }

    pub(super) fn steal_largest(
        &mut self,
        safe_zone: u64,
        min_steal_bytes: u64,
        align: u64,
    ) -> Option<StealResult> {
        let align = align.max(1);
        let (&victim_id, victim, stealable_start, stealable) = self
            .active
            .iter()
            .filter_map(|(id, attempt)| {
                if !matches!(attempt.role, AttemptRole::Normal) {
                    return None;
                }

                let stealable_start = attempt
                    .current
                    .saturating_add(safe_zone)
                    .min(attempt.stop_at);
                let stealable = attempt.stop_at.saturating_sub(stealable_start);
                (stealable >= 2 * min_steal_bytes).then_some((
                    id,
                    attempt,
                    stealable_start,
                    stealable,
                ))
            })
            .max_by_key(|(_, _, _, stealable)| *stealable)?;

        let raw_midpoint = stealable_start + stealable / 2;
        let midpoint = align_up(raw_midpoint, align);
        if midpoint >= victim.stop_at || victim.stop_at - midpoint < min_steal_bytes {
            return None;
        }

        let stolen = ByteRange::new(midpoint, victim.stop_at)?;
        let victim = self.active.get_mut(&victim_id)?;
        victim.stop_at = midpoint;
        self.pending.push_front(stolen);

        Some(StealResult {
            victim: victim_id,
            stolen,
            victim_stop_at: midpoint,
        })
    }

    pub(super) fn start_hedge_for(
        &mut self,
        original_id: AttemptId,
        min_remaining: u64,
    ) -> Option<ActiveAttempt> {
        let original = self.active.get(&original_id).copied()?;
        if !matches!(original.role, AttemptRole::Normal) {
            return None;
        }

        let remaining = original.remaining_range()?;
        if remaining.len() < min_remaining {
            return None;
        }

        let group = HedgeGroupId(self.next_hedge_group);
        self.next_hedge_group += 1;

        let duplicate = self.insert_attempt(remaining, AttemptRole::HedgeDuplicate { group });
        let original = self.active.get_mut(&original_id)?;
        original.role = AttemptRole::HedgeOriginal { group };

        self.hedges.insert(
            group,
            HedgeGroup {
                original: original_id,
                duplicate: duplicate.id,
                winner: None,
            },
        );

        Some(duplicate)
    }

    pub(super) fn start_largest_hedge(&mut self, min_remaining: u64) -> Option<ActiveAttempt> {
        let original_id = self
            .active
            .iter()
            .filter(|(_, attempt)| matches!(attempt.role, AttemptRole::Normal))
            .filter_map(|(id, attempt)| {
                let remaining = attempt.remaining_range()?;
                (remaining.len() >= min_remaining).then_some((*id, remaining.len()))
            })
            .max_by_key(|(_, remaining)| *remaining)
            .map(|(id, _)| id)?;

        self.start_hedge_for(original_id, min_remaining)
    }

    pub(super) fn pause_remaining(&self) -> RangeSet {
        let mut remaining = RangeSet::from_ranges(self.pending.iter().copied());
        for attempt in self.active.values() {
            if let Some(range) = attempt.remaining_range() {
                remaining.insert(range);
            }
        }
        for &range in self.completed.ranges() {
            remaining.subtract(range);
        }
        remaining
    }

    fn insert_attempt(&mut self, range: ByteRange, role: AttemptRole) -> ActiveAttempt {
        let id = AttemptId(self.next_attempt);
        self.next_attempt += 1;
        let attempt = ActiveAttempt {
            id,
            range,
            current: range.start(),
            stop_at: range.end(),
            role,
        };
        self.active.insert(id, attempt);
        attempt
    }

    fn handle_retryable_failure(&mut self, attempt: ActiveAttempt) -> Option<ByteRange> {
        match attempt.role {
            AttemptRole::Normal => self.requeue_remaining(attempt),
            AttemptRole::HedgeOriginal { group } | AttemptRole::HedgeDuplicate { group } => {
                let Some(hedge) = self.hedges.remove(&group) else {
                    return self.requeue_remaining(attempt);
                };

                let survivor_id = if attempt.id == hedge.original {
                    hedge.duplicate
                } else {
                    hedge.original
                };

                if let Some(survivor) = self.active.get_mut(&survivor_id) {
                    survivor.role = AttemptRole::Normal;
                    return None;
                }

                self.requeue_remaining(attempt)
            }
        }
    }

    fn handle_unrecoverable_failure(&mut self, id: AttemptId) -> bool {
        let Some(attempt) = self.active.remove(&id) else {
            return false;
        };

        let group = match attempt.role {
            AttemptRole::Normal => return false,
            AttemptRole::HedgeOriginal { group } | AttemptRole::HedgeDuplicate { group } => group,
        };

        let Some(hedge) = self.hedges.remove(&group) else {
            return false;
        };
        let survivor_id = if id == hedge.original {
            hedge.duplicate
        } else {
            hedge.original
        };

        if let Some(survivor) = self.active.get_mut(&survivor_id) {
            survivor.role = AttemptRole::Normal;
            return true;
        }

        false
    }

    fn requeue_remaining(&mut self, attempt: ActiveAttempt) -> Option<ByteRange> {
        let remaining = attempt.remaining_range()?;
        self.pending.push_front(remaining);
        Some(remaining)
    }

    fn mark_hedge_winner(&mut self, group: HedgeGroupId, winner: AttemptId) -> Option<AttemptId> {
        let hedge = self.hedges.get_mut(&group)?;
        hedge.winner = Some(winner);
        if winner == hedge.original {
            Some(hedge.duplicate)
        } else {
            Some(hedge.original)
        }
    }

    fn remove_from_hedge_group(&mut self, attempt: ActiveAttempt) {
        let group = match attempt.role {
            AttemptRole::Normal => return,
            AttemptRole::HedgeOriginal { group } | AttemptRole::HedgeDuplicate { group } => group,
        };

        let should_remove = self
            .hedges
            .get(&group)
            .is_none_or(|hedge| hedge.winner.is_some());
        if should_remove {
            self.hedges.remove(&group);
        }
    }
}

fn align_up(value: u64, align: u64) -> u64 {
    let rem = value % align;
    if rem == 0 {
        value
    } else {
        value + (align - rem)
    }
}

fn normalize_pending(
    total_bytes: u64,
    pending: impl IntoIterator<Item = ByteRange>,
) -> VecDeque<ByteRange> {
    let Some(total_range) = ByteRange::new(0, total_bytes) else {
        return VecDeque::new();
    };

    let mut ranges = pending
        .into_iter()
        .filter_map(|range| range.intersection(total_range))
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| range.start());

    let mut merged: Vec<ByteRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && last.intersection(range).is_some()
        {
            *last = ByteRange::new(last.start().min(range.start()), last.end().max(range.end()))
                .expect("merged overlapping ranges must be non-empty");
            continue;
        }
        merged.push(range);
    }

    merged.into()
}

fn clip_completed_to_total(total_bytes: u64, completed: RangeSet) -> RangeSet {
    let Some(total_range) = ByteRange::new(0, total_bytes) else {
        return RangeSet::new();
    };

    RangeSet::from_ranges(
        completed
            .ranges()
            .iter()
            .filter_map(|range| range.intersection(total_range)),
    )
}

#[cfg(test)]
mod tests {
    use super::{AttemptFailure, RangeScheduler};
    use crate::engine::http::events::{SchedulerAction, WorkerEvent, WorkerFailure};
    use crate::engine::http::ranges::{ByteRange, RangeSet};

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    #[test]
    fn starts_attempts_from_normalized_pending_ranges() {
        let mut scheduler = RangeScheduler::new(100, [range(50, 100), range(0, 25), range(20, 50)]);

        let first = scheduler.start_next_attempt().unwrap();

        assert_eq!(first.range(), range(0, 50));
        assert_eq!(
            scheduler.pending_ranges().collect::<Vec<_>>(),
            vec![range(50, 100)]
        );
        assert_eq!(scheduler.active_len(), 1);
    }

    #[test]
    fn scheduler_clips_pending_ranges_to_total_size() {
        let scheduler = RangeScheduler::new(100, [range(50, 150), range(0, 50), range(150, 200)]);

        assert_eq!(
            scheduler.pending_ranges().collect::<Vec<_>>(),
            vec![range(0, 50), range(50, 100)]
        );
    }

    #[test]
    fn scheduler_clips_completed_ranges_to_total_size() {
        let scheduler = RangeScheduler::from_completed(100, RangeSet::from_ranges([range(0, 150)]));

        assert_eq!(scheduler.completed_ranges(), &[range(0, 100)]);
        assert_eq!(scheduler.downloaded_bytes(), 100);
        assert!(scheduler.is_complete());
    }

    #[test]
    fn progress_counts_only_new_completed_bytes() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let first = scheduler.start_next_attempt().unwrap();
        let hedge = scheduler.start_hedge_for(first.id(), 1).unwrap();

        assert_eq!(
            scheduler.record_progress(first.id(), range(0, 60)),
            Some(60)
        );
        assert_eq!(
            scheduler.record_progress(hedge.id(), range(40, 100)),
            Some(40)
        );
        assert_eq!(
            scheduler.record_progress(first.id(), range(20, 80)),
            Some(0)
        );
        assert_eq!(scheduler.downloaded_bytes(), 100);
        assert_eq!(scheduler.completed_ranges(), &[range(0, 100)]);
    }

    #[test]
    fn steal_largest_shortens_victim_and_queues_back_half() {
        let mut scheduler = RangeScheduler::new(200, [range(0, 80), range(80, 200)]);
        let small = scheduler.start_next_attempt().unwrap();
        let large = scheduler.start_next_attempt().unwrap();

        scheduler.record_progress(small.id(), range(0, 40));
        scheduler.record_progress(large.id(), range(80, 100));

        let steal = scheduler.steal_largest(0, 20, 1).unwrap();
        let victim = scheduler.active_attempt(large.id()).unwrap();

        assert_eq!(steal.victim, large.id());
        assert_eq!(steal.stolen, range(150, 200));
        assert_eq!(victim.stop_at(), 150);
        assert_eq!(
            scheduler.pending_ranges().collect::<Vec<_>>(),
            vec![range(150, 200)]
        );
    }

    #[test]
    fn retryable_failure_requeues_remaining_range() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let attempt = scheduler.start_next_attempt().unwrap();
        scheduler.record_progress(attempt.id(), range(0, 40));

        let remaining = scheduler
            .fail_attempt(attempt.id(), AttemptFailure::Retryable)
            .unwrap();

        assert_eq!(remaining, range(40, 100));
        assert_eq!(
            scheduler.pending_ranges().collect::<Vec<_>>(),
            vec![range(40, 100)]
        );
        assert_eq!(scheduler.active_len(), 0);
    }

    #[test]
    fn hedge_winner_returns_loser_to_cancel_and_loser_is_not_requeued() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let original = scheduler.start_next_attempt().unwrap();
        let hedge = scheduler.start_hedge_for(original.id(), 1).unwrap();

        let finish = scheduler.finish_attempt(original.id()).unwrap();

        assert_eq!(finish.cancel_loser, Some(hedge.id()));
        assert_eq!(
            scheduler.fail_attempt(hedge.id(), AttemptFailure::HedgeLost),
            None
        );
        assert_eq!(scheduler.pending_len(), 0);
        assert_eq!(scheduler.active_len(), 0);
    }

    #[test]
    fn pause_remaining_normalizes_pending_and_active_ranges() {
        let mut scheduler = RangeScheduler::new(200, [range(0, 100), range(100, 200)]);
        let first = scheduler.start_next_attempt().unwrap();
        let second = scheduler.start_next_attempt().unwrap();
        let _hedge = scheduler.start_hedge_for(second.id(), 1).unwrap();

        scheduler.record_progress(first.id(), range(0, 80));
        scheduler.record_progress(second.id(), range(100, 130));

        let remaining = scheduler.pause_remaining();

        assert_eq!(remaining.ranges(), &[range(80, 100), range(130, 200)]);
    }

    #[test]
    fn completion_requires_full_coverage() {
        let completed = RangeSet::from_ranges([range(0, 50), range(50, 99)]);
        let mut scheduler = RangeScheduler::from_completed(100, completed);

        assert!(!scheduler.is_complete());
        assert_eq!(
            scheduler.pending_ranges().collect::<Vec<_>>(),
            vec![range(99, 100)]
        );

        let attempt = scheduler.start_next_attempt().unwrap();
        scheduler.record_progress(attempt.id(), range(99, 100));

        assert!(scheduler.is_complete());
    }

    #[test]
    fn worker_event_bytes_written_counts_only_new_progress() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let attempt = scheduler.start_next_attempt().unwrap();

        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::BytesWritten {
                attempt: attempt.id(),
                written: range(0, 60),
            }),
            SchedulerAction::CountedProgress { new_bytes: 60 }
        );
        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::BytesWritten {
                attempt: attempt.id(),
                written: range(20, 50),
            }),
            SchedulerAction::CountedProgress { new_bytes: 0 }
        );
        assert_eq!(scheduler.downloaded_bytes(), 60);
    }

    #[test]
    fn worker_event_data_received_does_not_count_progress() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let attempt = scheduler.start_next_attempt().unwrap();

        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::DataReceived {
                attempt: attempt.id(),
                bytes: 60,
            }),
            SchedulerAction::Nothing
        );
        assert_eq!(scheduler.downloaded_bytes(), 0);
    }

    #[test]
    fn worker_event_finished_can_return_hedge_loser_to_cancel() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let original = scheduler.start_next_attempt().unwrap();
        let hedge = scheduler.start_hedge_for(original.id(), 1).unwrap();

        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::Finished {
                attempt: original.id(),
            }),
            SchedulerAction::CancelAttempt {
                attempt: hedge.id()
            }
        );
    }

    #[test]
    fn worker_event_retryable_failure_requeues_remaining_bytes() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let attempt = scheduler.start_next_attempt().unwrap();
        scheduler.record_progress(attempt.id(), range(0, 25));

        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::Failed {
                attempt: attempt.id(),
                failure: WorkerFailure::Timeout,
            }),
            SchedulerAction::Requeued {
                range: range(25, 100),
                retry_after: None,
            }
        );
        assert_eq!(
            scheduler.pending_ranges().collect::<Vec<_>>(),
            vec![range(25, 100)]
        );
    }

    #[test]
    fn worker_event_hedge_lost_drops_attempt_without_requeue() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let original = scheduler.start_next_attempt().unwrap();
        let hedge = scheduler.start_hedge_for(original.id(), 1).unwrap();
        scheduler.finish_attempt(original.id()).unwrap();

        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::Failed {
                attempt: hedge.id(),
                failure: WorkerFailure::HedgeLost,
            }),
            SchedulerAction::Nothing
        );
        assert_eq!(scheduler.pending_len(), 0);
        assert_eq!(scheduler.active_len(), 0);
    }

    #[test]
    fn retryable_failure_in_hedge_pair_keeps_survivor_as_normal_attempt() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let original = scheduler.start_next_attempt().unwrap();
        let hedge = scheduler.start_hedge_for(original.id(), 1).unwrap();

        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::Failed {
                attempt: hedge.id(),
                failure: WorkerFailure::Timeout,
            }),
            SchedulerAction::Nothing
        );
        assert_eq!(scheduler.pending_len(), 0);
        assert_eq!(scheduler.active_len(), 1);
        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::Finished {
                attempt: original.id(),
            }),
            SchedulerAction::Nothing
        );
    }

    #[test]
    fn unrecoverable_failure_in_hedge_pair_keeps_survivor_as_normal_attempt() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let original = scheduler.start_next_attempt().unwrap();
        let hedge = scheduler.start_hedge_for(original.id(), 1).unwrap();

        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::Failed {
                attempt: hedge.id(),
                failure: WorkerFailure::NonRetryableHttp { status: 404 },
            }),
            SchedulerAction::Nothing
        );
        assert_eq!(scheduler.pending_len(), 0);
        assert_eq!(scheduler.active_len(), 1);
        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::Finished {
                attempt: original.id(),
            }),
            SchedulerAction::Nothing
        );
    }

    #[test]
    fn starts_hedge_for_largest_normal_attempt() {
        let mut scheduler = RangeScheduler::new(300, [range(0, 50), range(50, 300)]);
        let small = scheduler.start_next_attempt().unwrap();
        let large = scheduler.start_next_attempt().unwrap();

        let hedge = scheduler.start_largest_hedge(10).unwrap();

        assert_eq!(hedge.remaining_range(), Some(range(50, 300)));
        assert!(scheduler.start_hedge_for(small.id(), 10).is_some());
        assert!(scheduler.start_hedge_for(large.id(), 10).is_none());
    }

    #[test]
    fn steal_largest_ignores_ranges_that_are_already_hedged() {
        let mut scheduler = RangeScheduler::new(300, [range(0, 100), range(100, 300)]);
        let normal = scheduler.start_next_attempt().unwrap();
        let hedged = scheduler.start_next_attempt().unwrap();
        scheduler.start_hedge_for(hedged.id(), 1).unwrap();

        let steal = scheduler.steal_largest(0, 20, 1).unwrap();

        assert_eq!(steal.victim, normal.id());
    }

    #[test]
    fn worker_event_pause_keeps_attempt_for_snapshot() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let attempt = scheduler.start_next_attempt().unwrap();
        scheduler.record_progress(attempt.id(), range(0, 40));

        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::Paused {
                attempt: attempt.id(),
            }),
            SchedulerAction::PauseDownload
        );
        assert_eq!(scheduler.active_len(), 1);
        assert_eq!(scheduler.pause_remaining().ranges(), &[range(40, 100)]);
    }

    #[test]
    fn worker_event_bad_range_response_fails_download_and_removes_attempt() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let attempt = scheduler.start_next_attempt().unwrap();

        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::Failed {
                attempt: attempt.id(),
                failure: WorkerFailure::BadRangeResponse { status: 200 },
            }),
            SchedulerAction::FailDownload {
                failure: WorkerFailure::BadRangeResponse { status: 200 }
            }
        );
        assert_eq!(scheduler.active_len(), 0);
        assert_eq!(scheduler.pending_len(), 0);
    }

    #[test]
    fn worker_event_for_unknown_attempt_is_reported() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let unknown = super::AttemptId(99);

        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::Paused { attempt: unknown }),
            SchedulerAction::UnknownAttempt { attempt: unknown }
        );
    }
}
