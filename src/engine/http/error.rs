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

//! Chunk-level error classification.
//!
//! `ChunkError` drives the retry logic in the slot runner: Retryable errors get
//! exponential backoff, NonRetryable stops the slot, Fatal stops the whole download,
//! Paused and Killed are control-flow signals not true errors.

use std::time::Duration;

use reqwest::StatusCode;

pub enum ChunkError {
    /// Transient failure -> retry with backoff. `retry_after` is populated from
    /// the Retry-After header on 429.
    Retryable { retry_after: Option<Duration> },
    /// Server refused definitively (403, 404, 410). Retrying won't help.
    NonRetryable,
    /// Local failure (disk full, permission denied). Stops the entire download.
    Fatal(String),
    /// Soft pause requested via CancellationToken -> exit cleanly, save state.
    Paused,
    /// Health monitor killed this connection (too slow) -> retry immediately on
    /// a fresh connection without counting against the retry budget.
    Killed,
}

pub enum ChunkOutcome {
    Finished,
    Paused,
    Failed,
}

pub fn classify_status(status: StatusCode, headers: &reqwest::header::HeaderMap) -> ChunkError {
    match status.as_u16() {
        403 | 404 | 410 => ChunkError::NonRetryable,
        429 => {
            let retry_after = headers
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            ChunkError::Retryable { retry_after }
        }
        500..=599 => ChunkError::Retryable { retry_after: None },
        _ => ChunkError::NonRetryable,
    }
}

pub fn classify_io_error(e: std::io::Error) -> ChunkError {
    match e.kind() {
        std::io::ErrorKind::StorageFull | std::io::ErrorKind::PermissionDenied => {
            ChunkError::Fatal(e.to_string())
        }
        _ => ChunkError::Retryable { retry_after: None },
    }
}
