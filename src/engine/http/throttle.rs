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

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Token bucket rate limiter. `limit_bps = 0` means unlimited.
///
/// Tokens refill continuously based on elapsed wall time, capped at 1 second
/// of burst. `consume` returns how long the caller should sleep before
/// proceeding, the lock is dropped before the caller sleeps.
#[derive(Debug)]
pub struct TokenBucket {
    inner: Mutex<TbInner>,
}

#[derive(Debug)]
struct TbInner {
    limit_bps: u64,
    available: f64,
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(limit_bps: u64) -> Self {
        Self {
            inner: Mutex::new(TbInner {
                limit_bps,
                available: limit_bps as f64,
                last_refill: Instant::now(),
            }),
        }
    }

    pub fn set_limit(&self, limit_bps: u64) {
        let mut inner = self.inner.lock().unwrap();
        Self::refill(&mut inner);
        inner.limit_bps = limit_bps;
        inner.available = if limit_bps == 0 {
            0.0
        } else {
            inner.available.min(limit_bps as f64)
        };
    }

    fn refill(inner: &mut TbInner) {
        if inner.limit_bps == 0 {
            inner.last_refill = Instant::now();
            return;
        }
        let now = Instant::now();
        let elapsed = now.duration_since(inner.last_refill).as_secs_f64();
        inner.last_refill = now;
        // Refill up to 1 second's worth (max burst = 1s of bandwidth).
        inner.available =
            (inner.available + elapsed * inner.limit_bps as f64).min(inner.limit_bps as f64);
    }

    /// Consume `bytes` tokens. Returns the duration to sleep to stay within
    /// the rate limit. Returns `Duration::ZERO` when unlimited or when tokens
    /// are available.
    pub fn consume(&self, bytes: u64) -> Duration {
        let mut inner = self.inner.lock().unwrap();
        if inner.limit_bps == 0 {
            return Duration::ZERO;
        }
        Self::refill(&mut inner);
        inner.available -= bytes as f64;
        if inner.available < 0.0 {
            Duration::from_secs_f64((-inner.available) / inner.limit_bps as f64)
        } else {
            Duration::ZERO
        }
    }
}

/// Pairs a per-download bucket with the global bucket.
/// `consume` returns the larger of the two required waits so both limits
/// are respected simultaneously.
pub struct Throttle {
    pub per_download: Arc<TokenBucket>,
    pub global: Arc<TokenBucket>,
}

impl Throttle {
    pub fn consume(&self, bytes: u64) -> Duration {
        self.per_download
            .consume(bytes)
            .max(self.global.consume(bytes))
    }
}
