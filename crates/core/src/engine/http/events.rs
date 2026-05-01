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

//! Worker events for chunked HTTP downloads
//!
//! Workers report what happened and the scheduler decides what to do next

#![allow(dead_code)]

use std::time::Duration;

use super::{
    ranges::ByteRange,
    scheduler::{AttemptFailure, AttemptId},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WorkerEvent {
    DataReceived {
        attempt: AttemptId,
        bytes: u64,
    },
    BytesWritten {
        attempt: AttemptId,
        written: ByteRange,
    },
    Finished {
        attempt: AttemptId,
    },
    Paused {
        attempt: AttemptId,
    },
    Failed {
        attempt: AttemptId,
        failure: WorkerFailure,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum WorkerFailure {
    HealthRetry,
    Timeout,
    RetryableHttp { retry_after: Option<Duration> },
    RetryableIo { message: String },
    BadRangeResponse { status: u16 },
    NonRetryableHttp { status: u16 },
    FatalIo { message: String },
    HedgeLost,
}

impl WorkerFailure {
    pub(super) fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RetryableHttp { retry_after } => *retry_after,
            _ => None,
        }
    }

    pub(super) fn attempt_failure(&self) -> Option<AttemptFailure> {
        match self {
            Self::HealthRetry
            | Self::Timeout
            | Self::RetryableHttp { .. }
            | Self::RetryableIo { .. } => Some(AttemptFailure::Retryable),
            Self::HedgeLost => Some(AttemptFailure::HedgeLost),
            Self::BadRangeResponse { .. }
            | Self::NonRetryableHttp { .. }
            | Self::FatalIo { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SchedulerAction {
    Nothing,
    CountedProgress {
        new_bytes: u64,
    },
    CancelAttempt {
        attempt: AttemptId,
    },
    Requeued {
        range: ByteRange,
        retry_after: Option<Duration>,
    },
    PauseDownload,
    FailDownload {
        failure: WorkerFailure,
    },
    UnknownAttempt {
        attempt: AttemptId,
    },
}

#[cfg(test)]
mod tests {
    use super::WorkerFailure;
    use crate::engine::http::scheduler::AttemptFailure;

    #[test]
    fn retryable_worker_failures_map_to_retryable_attempt_failure() {
        assert_eq!(
            WorkerFailure::HealthRetry.attempt_failure(),
            Some(AttemptFailure::Retryable)
        );
        assert_eq!(
            WorkerFailure::Timeout.attempt_failure(),
            Some(AttemptFailure::Retryable)
        );
        assert_eq!(
            WorkerFailure::RetryableHttp { retry_after: None }.attempt_failure(),
            Some(AttemptFailure::Retryable)
        );
        assert_eq!(
            WorkerFailure::RetryableIo {
                message: "interrupted".to_string()
            }
            .attempt_failure(),
            Some(AttemptFailure::Retryable)
        );
    }

    #[test]
    fn hedge_lost_maps_to_hedge_lost_attempt_failure() {
        assert_eq!(
            WorkerFailure::HedgeLost.attempt_failure(),
            Some(AttemptFailure::HedgeLost)
        );
    }

    #[test]
    fn fatal_worker_failures_do_not_retry() {
        assert_eq!(
            WorkerFailure::BadRangeResponse { status: 200 }.attempt_failure(),
            None
        );
        assert_eq!(
            WorkerFailure::NonRetryableHttp { status: 404 }.attempt_failure(),
            None
        );
        assert_eq!(
            WorkerFailure::FatalIo {
                message: "disk full".to_string()
            }
            .attempt_failure(),
            None
        );
    }
}
