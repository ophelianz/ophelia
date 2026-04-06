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

//! Work stealing and hedging - the two forms of work redistribution.
//!
//! `try_steal` splits the largest active chunk when a worker goes idle.
//! `try_hedge` races a duplicate connection on the same remaining range when
//! nothing is large enough to split. Write-at is idempotent so both workers
//! writing the same bytes is safe; the first to finish snaps the original.
//!
//! When the pending queue empties and a worker finishes, `try_steal` finds the
//! active slot with the most remaining bytes and atomically splits it. The back
//! half becomes a new slot in the pre-allocated array; the victim keeps the front.
//!
//! Key design decisions (cross-referenced with aria2/AB DM/Surge/axel):
//!   - Atomic StopAt: ends[victim_i] is shrunk with Release ordering; make_chunk_fut
//!     reads it with Acquire so the victim sees the new boundary on its next retry.
//!   - Safe zone: bytes within `safe_zone` of the victim's current write position
//!     are excluded. They may be buffered but not yet flushed, stealing them would
//!     produce a harmless but wasteful double Range request.
//!   - 4KB alignment: split points align to block boundaries to avoid mid-block ranges.
//!   - Minimum: each half must have >= `min_steal_bytes` remaining or the steal is
//!     refused. Configured per-download; defaults to 4MB (configurable for tests).

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub fn try_steal(
    starts: &[AtomicU64],
    ends: &[AtomicU64],
    downloaded: &[AtomicU64],
    active: &HashSet<usize>,
    next_slot: &AtomicUsize,
    pending: &mut VecDeque<usize>,
    safe_zone: u64,
    min_steal_bytes: u64,
) {
    const ALIGN: u64 = 4096;

    let victim = active
        .iter()
        .filter_map(|&i| {
            let start = starts[i].load(Ordering::Relaxed);
            let end = ends[i].load(Ordering::Relaxed);
            let dl = downloaded[i].load(Ordering::Relaxed);
            let current = start + dl;
            let stealable_start = (current + safe_zone).min(end);
            let stealable = end.saturating_sub(stealable_start);
            if stealable >= 2 * min_steal_bytes {
                Some((i, stealable_start, stealable, end))
            } else {
                None
            }
        })
        .max_by_key(|&(_, _, stealable, _)| stealable);

    let (victim_i, stealable_start, stealable, victim_end) = match victim {
        Some(v) => v,
        None => return,
    };

    let raw_midpoint = stealable_start + stealable / 2;
    let midpoint = (raw_midpoint + ALIGN - 1) / ALIGN * ALIGN;
    if midpoint >= victim_end || victim_end - midpoint < min_steal_bytes {
        return;
    }

    let slot = next_slot.fetch_add(1, Ordering::Relaxed);
    if slot >= starts.len() {
        next_slot.fetch_sub(1, Ordering::Relaxed);
        return; // pre-allocated budget exhausted
    }

    // Shrink victim's end; Release pairs with Acquire in make_chunk_fut.
    ends[victim_i].store(midpoint, Ordering::Release);
    starts[slot].store(midpoint, Ordering::Relaxed);
    ends[slot].store(victim_end, Ordering::Relaxed);
    // downloaded[slot] starts at 0 (pre-initialised in the main array).

    pending.push_front(slot);
    tracing::debug!(
        victim = victim_i,
        slot,
        midpoint,
        stolen_bytes = victim_end - midpoint,
        "work stolen"
    );
}

/// Races a duplicate connection against the most-behind active slot.
///
/// Called when `try_steal` found nothing large enough to split.
/// Typically at the tail of a download when a fast connection goes idle but the last chunk
/// is too small to bisect cleanly. The hedge writes the same bytes via write_at
/// (idempotent). The caller snaps the original's counter when the hedge finishes
/// and kills it; the original then exits immediately on its next attempt because
/// `byte_start >= chunk_end`.
///
/// Returns `Some((hedge_slot, original_slot))` on success, `None` if no slot
/// has enough remaining bytes or the slot budget is exhausted.
pub fn try_hedge(
    starts: &[AtomicU64],
    ends: &[AtomicU64],
    downloaded: &[AtomicU64],
    active: &HashSet<usize>,
    next_slot: &AtomicUsize,
    pending: &mut VecDeque<usize>,
    min_hedge_remaining: u64,
) -> Option<(usize, usize)> {
    let (original, remaining) = active
        .iter()
        .map(|&i| {
            let remaining = ends[i].load(Ordering::Relaxed).saturating_sub(
                starts[i].load(Ordering::Relaxed) + downloaded[i].load(Ordering::Relaxed),
            );
            (i, remaining)
        })
        .max_by_key(|&(_, r)| r)?;

    if remaining < min_hedge_remaining {
        return None;
    }

    let slot = next_slot.fetch_add(1, Ordering::Relaxed);
    if slot >= starts.len() {
        next_slot.fetch_sub(1, Ordering::Relaxed);
        return None;
    }

    let current_pos =
        starts[original].load(Ordering::Relaxed) + downloaded[original].load(Ordering::Relaxed);
    starts[slot].store(current_pos, Ordering::Relaxed);
    ends[slot].store(ends[original].load(Ordering::Relaxed), Ordering::Relaxed);
    // downloaded[slot] starts at 0 (pre-initialised in the main array).

    pending.push_back(slot);
    tracing::debug!(
        original,
        slot,
        current_pos,
        remaining_bytes = remaining,
        "hedging: racing duplicate connection on remaining range"
    );
    Some((slot, original))
}
