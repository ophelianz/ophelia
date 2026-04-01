use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Token bucket rate limiter. `limit_bps = 0` means unlimited.
///
/// Tokens refill continuously based on elapsed wall time, capped at 1 second
/// of burst. `consume` returns how long the caller should sleep before
/// proceeding, the lock is dropped before the caller sleeps.
#[derive(Debug)]
pub struct TokenBucket {
    limit_bps: u64,
    inner: Mutex<TbInner>,
}

#[derive(Debug)]
struct TbInner {
    available: f64,
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(limit_bps: u64) -> Self {
        Self {
            limit_bps,
            inner: Mutex::new(TbInner {
                available: limit_bps as f64,
                last_refill: Instant::now(),
            }),
        }
    }

    /// Consume `bytes` tokens. Returns the duration to sleep to stay within
    /// the rate limit. Returns `Duration::ZERO` when unlimited or when tokens
    /// are available.
    pub fn consume(&self, bytes: u64) -> Duration {
        if self.limit_bps == 0 {
            return Duration::ZERO;
        }
        let mut inner = self.inner.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(inner.last_refill).as_secs_f64();
        inner.last_refill = now;
        // Refill up to 1 second's worth (max burst = 1s of bandwidth).
        inner.available = (inner.available + elapsed * self.limit_bps as f64)
            .min(self.limit_bps as f64);
        inner.available -= bytes as f64;
        if inner.available < 0.0 {
            Duration::from_secs_f64((-inner.available) / self.limit_bps as f64)
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
        self.per_download.consume(bytes).max(self.global.consume(bytes))
    }
}
