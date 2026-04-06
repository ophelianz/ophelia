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

//! Background progress reporter.
//!
//! Polls all populated slot counters at `progress_interval_ms` and emits
//! ProgressUpdate messages with an EMA-smoothed speed.
//!
//! Speed is computed from `already_done` (bytes on disk at task start), not
//! from zero, so resumed downloads don't show inflated speed from prior work.
//! Inspired from Surge's `SessionStartBytes` pattern

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::engine::types::{DownloadId, DownloadStatus, ProgressUpdate};

const EMA_ALPHA: f64 = 0.3;
const WINDOW_SECS: f64 = 2.0;

fn update_ema(ema: f64, window_bytes: u64, elapsed: f64) -> f64 {
    let recent = window_bytes as f64 / elapsed;
    let updated = (1.0 - EMA_ALPHA) * ema + EMA_ALPHA * recent;
    if window_bytes == 0 {
        updated * (WINDOW_SECS / elapsed.max(WINDOW_SECS))
    } else {
        updated
    }
}

pub fn spawn_progress_reporter(
    id: DownloadId,
    counters: Arc<Vec<AtomicU64>>,
    slot_count: Arc<AtomicUsize>,
    total_bytes: u64,
    already_done: u64,
    progress_interval_ms: u64,
    progress_tx: mpsc::UnboundedSender<ProgressUpdate>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ema_speed: f64 = 0.0;
        let mut window_start = Instant::now();
        let mut window_bytes: u64 = 0;
        let mut last_total: u64 = already_done;

        loop {
            tokio::time::sleep(Duration::from_millis(progress_interval_ms)).await;

            // Sum only populated slots (initial + any stolen)
            let populated = slot_count.load(Ordering::Relaxed);
            let total_downloaded: u64 = counters[..populated]
                .iter()
                .map(|a| a.load(Ordering::Relaxed))
                .sum();
            let new_bytes = total_downloaded.saturating_sub(last_total);
            last_total = total_downloaded;
            window_bytes += new_bytes;

            let window_elapsed = window_start.elapsed().as_secs_f64();
            if window_elapsed >= WINDOW_SECS {
                ema_speed = update_ema(ema_speed, window_bytes, window_elapsed);
                window_bytes = 0;
                window_start = Instant::now();
            }

            let _ = progress_tx.send(ProgressUpdate {
                id,
                status: DownloadStatus::Downloading,
                downloaded_bytes: total_downloaded,
                total_bytes: Some(total_bytes),
                speed_bytes_per_sec: ema_speed as u64,
            });
            if total_downloaded >= total_bytes {
                break;
            }
        }
    })
}
