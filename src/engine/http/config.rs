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

//! Per-download configuration for HTTP/HTTPS downloads.
//! Fields here are intentionally HTTP-specific: connection count, stall detection,
//! and retry behavior are concepts that don't apply to all protocols.

use crate::settings::Settings;

#[derive(Debug, Clone)]
pub struct HttpDownloadConfig {
    /// Hard ceiling on parallel connections per download. The actual count is
    /// derived from the sqrt heuristic and clamped to [min_connections, max_connections].
    pub max_connections: usize,
    /// Floor for the sqrt heuristic. Default 1 (heuristic drives everything).
    /// Set higher in tests to force parallel chunks on small files without needing
    /// a large file download.
    pub min_connections: usize,
    pub write_buffer_size: usize,
    pub progress_interval_ms: u64,
    pub stall_timeout_secs: u64,
    pub max_retries_per_chunk: u32,
    /// Minimum bytes required in each half of a potential steal.
    /// A steal requires >= 2× this value remaining. Lowered in tests to exercise
    /// the code path on small files.
    pub min_steal_bytes: u64,
    /// Per-download bandwidth cap in bytes/sec. 0 = unlimited.
    pub speed_limit_bps: u64,
}

impl Default for HttpDownloadConfig {
    fn default() -> Self {
        Self {
            max_connections: 8,
            min_connections: 1,
            write_buffer_size: 64 * 1024,
            progress_interval_ms: 100,
            stall_timeout_secs: 10,
            max_retries_per_chunk: 3,
            min_steal_bytes: 4 * 1024 * 1024,
            speed_limit_bps: 0,
        }
    }
}

impl HttpDownloadConfig {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            max_connections: settings.max_connections_per_download,
            ..Self::default()
        }
    }
}
