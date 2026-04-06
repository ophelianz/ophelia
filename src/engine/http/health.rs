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

//! Health monitor - kills connections that fall below 50% of mean speed.
//!
//! Ticks every 1 second. Per-slot speed is smoothed with EMA (α=0.3) so a single
//! slow tick doesn't trigger a kill;
//! similar approach as Surge's SpeedCalc and aria2's per-connection speed tracking.
//!
//! Grace period: 5 seconds after activation before a slot is eligible.
//! Time-based (not bytes-based) so slow connections that haven't downloaded
//! 1MB yet are still protected during TCP slow-start.
//!
//! Requires >= 2 eligible slots to have a meaningful mean to compare against.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const GRACE_MS: u64 = 5_000;
const SLOW_FACTOR: f64 = 0.5;
const EMA_ALPHA: f64 = 0.3;

/// Returns the current time as milliseconds since the Unix epoch.
/// Stored in `slot_activation` arrays to track per-attempt start times.
pub fn activation_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

pub fn spawn_health_monitor(
    counters: Arc<Vec<AtomicU64>>,
    kills: Arc<Vec<Mutex<CancellationToken>>>,
    active: Arc<Mutex<HashSet<usize>>>,
    activation: Arc<Vec<AtomicU64>>,
    pause_token: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut prev: Vec<u64> = vec![0u64; counters.len()];
        let mut ema: Vec<f64> = vec![0.0f64; counters.len()];

        loop {
            tokio::select! {
                biased;
                _ = pause_token.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }

            let active_set = active.lock().unwrap().clone();
            if active_set.len() < 2 {
                continue;
            }

            // Update EMA for every active slot.
            for &i in &active_set {
                let current = counters[i].load(Ordering::Relaxed);
                let delta = current.saturating_sub(prev[i]) as f64;
                prev[i] = current;
                ema[i] = (1.0 - EMA_ALPHA) * ema[i] + EMA_ALPHA * delta;
            }

            // Eligible = past the grace period for this attempt.
            let now = activation_now();
            let eligible: Vec<(usize, f64)> = active_set
                .iter()
                .filter(|&&i| now.saturating_sub(activation[i].load(Ordering::Relaxed)) >= GRACE_MS)
                .map(|&i| (i, ema[i]))
                .collect();

            if eligible.len() < 2 {
                continue;
            }

            let sum: f64 = eligible.iter().map(|(_, s)| s).sum();
            let mean = sum / eligible.len() as f64;
            if mean < 1.0 {
                continue;
            }

            for (i, speed) in eligible.iter().filter(|(_, s)| *s < mean * SLOW_FACTOR) {
                tracing::debug!(
                    slot = i,
                    speed = *speed as u64,
                    mean = mean as u64,
                    "health monitor killing slow worker"
                );
                kills[*i].lock().unwrap().cancel();
            }
        }
    })
}
