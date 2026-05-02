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

use std::collections::VecDeque;

use bitvec::prelude::{BitVec, Lsb0};

use super::events::{SchedulerAction, WorkerEvent, WorkerFailure};
use super::ranges::{ByteRange, RangeSet};

const NO_INDEX: usize = usize::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct AttemptId {
    slot: u32,
    generation: u32,
}

impl AttemptId {
    fn new(slot: usize, generation: u32) -> Self {
        Self {
            slot: slot as u32,
            generation,
        }
    }

    fn index(self) -> AttemptIndex {
        AttemptIndex(self.slot as usize)
    }

    pub(super) fn slot(self) -> usize {
        self.slot as usize
    }

    pub(super) fn generation(self) -> u32 {
        self.generation
    }

    #[cfg(test)]
    fn stale_for_test(slot: usize) -> Self {
        Self::new(slot, 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AttemptIndex(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct HedgeGroupId {
    slot: u32,
    generation: u32,
}

impl HedgeGroupId {
    fn new(slot: usize, generation: u32) -> Self {
        Self {
            slot: slot as u32,
            generation,
        }
    }

    fn index(self) -> HedgeGroupIndex {
        HedgeGroupIndex(self.slot as usize)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HedgeGroupIndex(usize);

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

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttemptRoleCode {
    Normal,
    HedgeOriginal,
    HedgeDuplicate,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HedgeWinnerCode {
    None,
    Original,
    Duplicate,
}

#[derive(Debug, Clone, Default)]
struct PendingRangeTable {
    starts: Vec<u64>,
    ends: Vec<u64>,
    queued: VecDeque<PendingRangeIndex>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingRangeIndex(usize);

#[derive(Debug, Clone, Default)]
struct AttemptTable {
    starts: Vec<u64>,
    ends: Vec<u64>,
    currents: Vec<u64>,
    stop_ats: Vec<u64>,
    generations: Vec<u32>,
    roles: Vec<AttemptRoleCode>,
    hedge_group_slots: Vec<usize>,
    hedge_group_generations: Vec<u32>,
    active: BitVec<usize, Lsb0>,
    active_rows: Vec<AttemptIndex>,
    active_positions: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
struct HedgeGroupTable {
    originals: Vec<AttemptId>,
    duplicates: Vec<AttemptId>,
    winners: Vec<HedgeWinnerCode>,
    generations: Vec<u32>,
    active: BitVec<usize, Lsb0>,
}

#[derive(Debug, Clone)]
pub(super) struct RangeScheduler {
    total_bytes: u64,
    pending: PendingRangeTable,
    completed: RangeSet,
    active: AttemptTable,
    hedges: HedgeGroupTable,
}

impl PendingRangeTable {
    fn from_ranges(ranges: VecDeque<ByteRange>) -> Self {
        let mut table = Self::default();
        for range in ranges {
            table.push_back(range);
        }
        table
    }

    fn push_front(&mut self, range: ByteRange) {
        let index = self.push(range);
        self.queued.push_front(index);
    }

    fn push_back(&mut self, range: ByteRange) {
        let index = self.push(range);
        self.queued.push_back(index);
    }

    fn pop_front(&mut self) -> Option<ByteRange> {
        let index = self.queued.pop_front()?;
        self.range(index)
    }

    fn len(&self) -> usize {
        self.queued.len()
    }

    fn iter(&self) -> impl Iterator<Item = ByteRange> + '_ {
        self.queued.iter().filter_map(|&index| self.range(index))
    }

    fn push(&mut self, range: ByteRange) -> PendingRangeIndex {
        let index = PendingRangeIndex(self.starts.len());
        self.starts.push(range.start());
        self.ends.push(range.end());
        index
    }

    fn range(&self, index: PendingRangeIndex) -> Option<ByteRange> {
        ByteRange::new(self.starts[index.0], self.ends[index.0])
    }
}

impl AttemptTable {
    fn active_len(&self) -> usize {
        self.active_rows.len()
    }

    fn contains(&self, id: AttemptId) -> bool {
        self.index_for(id).is_some()
    }

    fn get(&self, id: AttemptId) -> Option<ActiveAttempt> {
        self.attempt_at(self.index_for(id)?)
    }

    fn insert(&mut self, range: ByteRange, role: AttemptRole) -> ActiveAttempt {
        let generation = 1;
        let index = AttemptIndex(self.starts.len());
        let id = AttemptId::new(index.0, generation);
        let (role_code, group) = role_parts(role);

        self.starts.push(range.start());
        self.ends.push(range.end());
        self.currents.push(range.start());
        self.stop_ats.push(range.end());
        self.generations.push(generation);
        self.roles.push(role_code);
        self.hedge_group_slots
            .push(group.map_or(NO_INDEX, |group| group.index().0));
        self.hedge_group_generations
            .push(group.map_or(0, |group| group.generation));
        self.active.push(true);
        self.active_positions.push(self.active_rows.len());
        self.active_rows.push(index);

        ActiveAttempt {
            id,
            range,
            current: range.start(),
            stop_at: range.end(),
            role,
        }
    }

    fn remove(&mut self, id: AttemptId) -> Option<ActiveAttempt> {
        let index = self.index_for(id)?;
        let attempt = self.attempt_at(index)?;
        self.active.set(index.0, false);
        self.remove_active_index(index);
        Some(attempt)
    }

    fn record_written(&mut self, id: AttemptId, written: ByteRange) -> Option<ByteRange> {
        let index = self.index_for(id)?;
        let stop_at = self.stop_ats[index.0];
        let allowed = ByteRange::new(self.starts[index.0], stop_at)?;
        let written = written.intersection(allowed)?;
        self.currents[index.0] = self.currents[index.0].max(written.end()).min(stop_at);
        Some(written)
    }

    fn set_stop_at(&mut self, id: AttemptId, stop_at: u64) -> Option<()> {
        let index = self.index_for(id)?;
        self.stop_ats[index.0] = stop_at;
        self.currents[index.0] = self.currents[index.0].min(stop_at);
        Some(())
    }

    fn set_role(&mut self, id: AttemptId, role: AttemptRole) -> Option<()> {
        let index = self.index_for(id)?;
        let (role_code, group) = role_parts(role);
        self.roles[index.0] = role_code;
        self.hedge_group_slots[index.0] = group.map_or(NO_INDEX, |group| group.index().0);
        self.hedge_group_generations[index.0] = group.map_or(0, |group| group.generation);
        Some(())
    }

    fn iter_active(&self) -> impl Iterator<Item = ActiveAttempt> + '_ {
        self.active_rows
            .iter()
            .filter_map(|&index| self.attempt_at(index))
    }

    fn index_for(&self, id: AttemptId) -> Option<AttemptIndex> {
        let index = id.index();
        if self.generations.get(index.0).copied()? != id.generation {
            return None;
        }
        self.active
            .get(index.0)
            .is_some_and(|bit| *bit)
            .then_some(index)
    }

    fn attempt_at(&self, index: AttemptIndex) -> Option<ActiveAttempt> {
        let range = ByteRange::new(self.starts[index.0], self.ends[index.0])?;
        Some(ActiveAttempt {
            id: AttemptId::new(index.0, self.generations[index.0]),
            range,
            current: self.currents[index.0],
            stop_at: self.stop_ats[index.0],
            role: self.role_at(index)?,
        })
    }

    fn role_at(&self, index: AttemptIndex) -> Option<AttemptRole> {
        let group = || {
            let slot = self.hedge_group_slots[index.0];
            (slot != NO_INDEX)
                .then(|| HedgeGroupId::new(slot, self.hedge_group_generations[index.0]))
        };
        Some(match self.roles[index.0] {
            AttemptRoleCode::Normal => AttemptRole::Normal,
            AttemptRoleCode::HedgeOriginal => AttemptRole::HedgeOriginal { group: group()? },
            AttemptRoleCode::HedgeDuplicate => AttemptRole::HedgeDuplicate { group: group()? },
        })
    }

    fn remove_active_index(&mut self, index: AttemptIndex) {
        let pos = self.active_positions[index.0];
        self.active_rows.swap_remove(pos);
        if let Some(&moved) = self.active_rows.get(pos) {
            self.active_positions[moved.0] = pos;
        }
        self.active_positions[index.0] = NO_INDEX;
    }
}

impl HedgeGroupTable {
    fn next_id(&self) -> HedgeGroupId {
        HedgeGroupId::new(self.originals.len(), 1)
    }

    fn insert(&mut self, original: AttemptId, duplicate: AttemptId) -> HedgeGroupId {
        let generation = 1;
        let index = HedgeGroupIndex(self.originals.len());
        let id = HedgeGroupId::new(index.0, generation);
        self.originals.push(original);
        self.duplicates.push(duplicate);
        self.winners.push(HedgeWinnerCode::None);
        self.generations.push(generation);
        self.active.push(true);
        id
    }

    fn get(&self, id: HedgeGroupId) -> Option<HedgeGroup> {
        let index = self.index_for(id)?;
        Some(self.group_at(index))
    }

    fn remove(&mut self, id: HedgeGroupId) -> Option<HedgeGroup> {
        let index = self.index_for(id)?;
        let group = self.group_at(index);
        self.active.set(index.0, false);
        Some(group)
    }

    fn mark_winner(&mut self, id: HedgeGroupId, winner: AttemptId) -> Option<AttemptId> {
        let index = self.index_for(id)?;
        if winner == self.originals[index.0] {
            self.winners[index.0] = HedgeWinnerCode::Original;
            Some(self.duplicates[index.0])
        } else if winner == self.duplicates[index.0] {
            self.winners[index.0] = HedgeWinnerCode::Duplicate;
            Some(self.originals[index.0])
        } else {
            None
        }
    }

    fn index_for(&self, id: HedgeGroupId) -> Option<HedgeGroupIndex> {
        let index = id.index();
        if self.generations.get(index.0).copied()? != id.generation {
            return None;
        }
        self.active
            .get(index.0)
            .is_some_and(|bit| *bit)
            .then_some(index)
    }

    fn group_at(&self, index: HedgeGroupIndex) -> HedgeGroup {
        let original = self.originals[index.0];
        let duplicate = self.duplicates[index.0];
        let winner = match self.winners[index.0] {
            HedgeWinnerCode::None => None,
            HedgeWinnerCode::Original => Some(original),
            HedgeWinnerCode::Duplicate => Some(duplicate),
        };
        HedgeGroup {
            original,
            duplicate,
            winner,
        }
    }
}

fn role_parts(role: AttemptRole) -> (AttemptRoleCode, Option<HedgeGroupId>) {
    match role {
        AttemptRole::Normal => (AttemptRoleCode::Normal, None),
        AttemptRole::HedgeOriginal { group } => (AttemptRoleCode::HedgeOriginal, Some(group)),
        AttemptRole::HedgeDuplicate { group } => (AttemptRoleCode::HedgeDuplicate, Some(group)),
    }
}

impl RangeScheduler {
    pub(super) fn new(total_bytes: u64, pending: impl IntoIterator<Item = ByteRange>) -> Self {
        let pending = PendingRangeTable::from_ranges(normalize_pending(total_bytes, pending));
        Self {
            total_bytes,
            pending,
            completed: RangeSet::new(),
            active: AttemptTable::default(),
            hedges: HedgeGroupTable::default(),
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
            pending: PendingRangeTable::from_ranges(pending),
            completed,
            active: AttemptTable::default(),
            hedges: HedgeGroupTable::default(),
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
        self.active.active_len()
    }

    pub(super) fn pending_ranges(&self) -> impl Iterator<Item = ByteRange> + '_ {
        self.pending.iter()
    }

    pub(super) fn completed_ranges(&self) -> &[ByteRange] {
        self.completed.ranges()
    }

    pub(super) fn active_attempt(&self, id: AttemptId) -> Option<ActiveAttempt> {
        self.active.get(id)
    }

    pub(super) fn apply_worker_event(&mut self, event: WorkerEvent) -> SchedulerAction {
        match event {
            WorkerEvent::DataReceived { attempt, .. } => {
                if self.active.contains(attempt) {
                    SchedulerAction::Nothing
                } else {
                    SchedulerAction::UnknownAttempt { attempt }
                }
            }
            WorkerEvent::BytesWritten { attempt, written } => {
                let known = self.active.contains(attempt);
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
                if self.active.contains(attempt) {
                    SchedulerAction::PauseDownload
                } else {
                    SchedulerAction::UnknownAttempt { attempt }
                }
            }
            WorkerEvent::Failed { attempt, failure } => {
                let known = self.active.contains(attempt);
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
        Some(self.active.insert(range, AttemptRole::Normal))
    }

    fn record_progress(&mut self, id: AttemptId, written: ByteRange) -> Option<u64> {
        let written = self.active.record_written(id, written)?;
        Some(self.completed.insert_and_count_new(written))
    }

    fn finish_attempt(&mut self, id: AttemptId) -> Option<FinishResult> {
        let attempt = self.active.remove(id)?;
        let cancel_loser = match attempt.role {
            AttemptRole::Normal => None,
            AttemptRole::HedgeOriginal { group } | AttemptRole::HedgeDuplicate { group } => {
                self.mark_hedge_winner(group, id)
            }
        };
        Some(FinishResult { cancel_loser })
    }

    fn fail_attempt(&mut self, id: AttemptId, failure: AttemptFailure) -> Option<ByteRange> {
        let attempt = self.active.remove(id)?;
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
        let (victim_id, victim, stealable_start, stealable) = self
            .active
            .iter_active()
            .filter_map(|attempt| {
                if !matches!(attempt.role, AttemptRole::Normal) {
                    return None;
                }

                let stealable_start = attempt
                    .current
                    .saturating_add(safe_zone)
                    .min(attempt.stop_at);
                let stealable = attempt.stop_at.saturating_sub(stealable_start);
                (stealable >= 2 * min_steal_bytes).then_some((
                    attempt.id,
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
        self.active.set_stop_at(victim_id, midpoint)?;
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
        let original = self.active.get(original_id)?;
        if !matches!(original.role, AttemptRole::Normal) {
            return None;
        }

        let remaining = original.remaining_range()?;
        if remaining.len() < min_remaining {
            return None;
        }

        let group = self.hedges.next_id();

        let duplicate = self
            .active
            .insert(remaining, AttemptRole::HedgeDuplicate { group });
        self.active
            .set_role(original_id, AttemptRole::HedgeOriginal { group })?;
        self.hedges.insert(original_id, duplicate.id);

        Some(duplicate)
    }

    pub(super) fn start_largest_hedge(&mut self, min_remaining: u64) -> Option<ActiveAttempt> {
        let original_id = self
            .active
            .iter_active()
            .filter(|attempt| matches!(attempt.role, AttemptRole::Normal))
            .filter_map(|attempt| {
                let remaining = attempt.remaining_range()?;
                (remaining.len() >= min_remaining).then_some((attempt.id, remaining.len()))
            })
            .max_by_key(|(_, remaining)| *remaining)
            .map(|(id, _)| id)?;

        self.start_hedge_for(original_id, min_remaining)
    }

    pub(super) fn pause_remaining(&self) -> RangeSet {
        let mut remaining = RangeSet::from_ranges(self.pending.iter());
        for attempt in self.active.iter_active() {
            if let Some(range) = attempt.remaining_range() {
                remaining.insert(range);
            }
        }
        for &range in self.completed.ranges() {
            remaining.subtract(range);
        }
        remaining
    }

    fn handle_retryable_failure(&mut self, attempt: ActiveAttempt) -> Option<ByteRange> {
        match attempt.role {
            AttemptRole::Normal => self.requeue_remaining(attempt),
            AttemptRole::HedgeOriginal { group } | AttemptRole::HedgeDuplicate { group } => {
                let Some(hedge) = self.hedges.remove(group) else {
                    return self.requeue_remaining(attempt);
                };

                let survivor_id = if attempt.id == hedge.original {
                    hedge.duplicate
                } else {
                    hedge.original
                };

                if self
                    .active
                    .set_role(survivor_id, AttemptRole::Normal)
                    .is_some()
                {
                    return None;
                }

                self.requeue_remaining(attempt)
            }
        }
    }

    fn handle_unrecoverable_failure(&mut self, id: AttemptId) -> bool {
        let Some(attempt) = self.active.remove(id) else {
            return false;
        };

        let group = match attempt.role {
            AttemptRole::Normal => return false,
            AttemptRole::HedgeOriginal { group } | AttemptRole::HedgeDuplicate { group } => group,
        };

        let Some(hedge) = self.hedges.remove(group) else {
            return false;
        };
        let survivor_id = if id == hedge.original {
            hedge.duplicate
        } else {
            hedge.original
        };

        if self
            .active
            .set_role(survivor_id, AttemptRole::Normal)
            .is_some()
        {
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
        self.hedges.mark_winner(group, winner)
    }

    fn remove_from_hedge_group(&mut self, attempt: ActiveAttempt) {
        let group = match attempt.role {
            AttemptRole::Normal => return,
            AttemptRole::HedgeOriginal { group } | AttemptRole::HedgeDuplicate { group } => group,
        };

        let should_remove = self
            .hedges
            .get(group)
            .is_none_or(|hedge| hedge.winner.is_some());
        if should_remove {
            self.hedges.remove(group);
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
        let unknown = super::AttemptId::stale_for_test(99);

        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::Paused { attempt: unknown }),
            SchedulerAction::UnknownAttempt { attempt: unknown }
        );
    }

    #[test]
    fn worker_event_for_stale_attempt_generation_is_reported() {
        let mut scheduler = RangeScheduler::new(100, [range(0, 100)]);
        let attempt = scheduler.start_next_attempt().unwrap();
        let stale = super::AttemptId {
            slot: attempt.id().slot,
            generation: attempt.id().generation + 1,
        };

        assert_eq!(
            scheduler.apply_worker_event(WorkerEvent::DataReceived {
                attempt: stale,
                bytes: 1,
            }),
            SchedulerAction::UnknownAttempt { attempt: stale }
        );
    }
}
