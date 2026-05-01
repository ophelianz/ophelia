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

//! One blocking disk writer for one range download
//!
//! Range workers send owned buffers here and wait for confirmation

use std::fs::File;
use std::io;

use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use super::events::{WorkerEvent, WorkerFailure};
use super::ranges::ByteRange;
use super::scheduler::AttemptId;

const WRITE_JOB_CAPACITY: usize = 256;

pub(super) type RangeWriteSender = mpsc::Sender<RangeWriteJob>;

pub(super) struct RangeDiskWriter {
    jobs: RangeWriteSender,
    handle: JoinHandle<Vec<RangeWriteResult>>,
}

pub(super) struct RangeWriteJob {
    attempt: AttemptId,
    offset: u64,
    bytes: Vec<u8>,
    reply: oneshot::Sender<RangeWriteResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RangeWriteResult {
    Written {
        attempt: AttemptId,
        range: ByteRange,
    },
    Failed {
        attempt: AttemptId,
        failure: WorkerFailure,
    },
}

impl RangeDiskWriter {
    pub(super) fn spawn(file: File) -> Self {
        let (jobs, rx) = mpsc::channel(WRITE_JOB_CAPACITY);
        let handle = tokio::task::spawn_blocking(move || run_writer(file, rx));
        Self { jobs, handle }
    }

    pub(super) fn sender(&self) -> RangeWriteSender {
        self.jobs.clone()
    }

    pub(super) async fn shutdown(self) -> Vec<RangeWriteResult> {
        drop(self.jobs);
        match self.handle.await {
            Ok(orphaned) => orphaned,
            Err(error) => {
                tracing::error!(?error, "range disk writer task failed");
                Vec::new()
            }
        }
    }
}

impl RangeWriteJob {
    pub(super) fn new(
        attempt: AttemptId,
        offset: u64,
        bytes: Vec<u8>,
        reply: oneshot::Sender<RangeWriteResult>,
    ) -> Self {
        Self {
            attempt,
            offset,
            bytes,
            reply,
        }
    }
}

impl RangeWriteResult {
    pub(super) fn into_worker_event(self) -> WorkerEvent {
        match self {
            Self::Written { attempt, range } => WorkerEvent::BytesWritten {
                attempt,
                written: range,
            },
            Self::Failed { attempt, failure } => WorkerEvent::Failed { attempt, failure },
        }
    }
}

fn run_writer(mut file: File, mut jobs: mpsc::Receiver<RangeWriteJob>) -> Vec<RangeWriteResult> {
    let mut orphaned = Vec::new();
    while let Some(job) = jobs.blocking_recv() {
        let result = write_job(&mut file, &job);
        if job.reply.send(result.clone()).is_err() {
            orphaned.push(result);
        }
    }
    orphaned
}

fn write_job(file: &mut File, job: &RangeWriteJob) -> RangeWriteResult {
    let Some(range) = ByteRange::from_len(job.offset, job.bytes.len() as u64) else {
        return RangeWriteResult::Failed {
            attempt: job.attempt,
            failure: WorkerFailure::FatalIo {
                message: "write range overflow".to_string(),
            },
        };
    };

    match write_all_at(file, &job.bytes, job.offset) {
        Ok(()) => RangeWriteResult::Written {
            attempt: job.attempt,
            range,
        },
        Err(error) => RangeWriteResult::Failed {
            attempt: job.attempt,
            failure: failure_from_io(error),
        },
    }
}

fn failure_from_io(error: io::Error) -> WorkerFailure {
    match error.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::PermissionDenied => WorkerFailure::FatalIo {
            message: error.to_string(),
        },
        _ => WorkerFailure::RetryableIo {
            message: error.to_string(),
        },
    }
}

#[cfg(unix)]
fn write_all_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
fn write_all_at(file: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;

    while !buf.is_empty() {
        let written = file.seek_write(buf, offset)?;
        if written == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        offset = offset.saturating_add(written as u64);
        buf = &buf[written..];
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;
    use crate::engine::http::ranges::ByteRange;
    use crate::engine::http::scheduler::RangeScheduler;

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    fn attempt() -> AttemptId {
        let mut scheduler = RangeScheduler::new(8, [range(0, 8)]);
        scheduler.start_next_attempt().unwrap().id()
    }

    fn test_file() -> (tempfile::TempDir, File) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("part.bin");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        (dir, file)
    }

    #[tokio::test]
    async fn disk_writer_writes_at_offsets_and_reports_range() {
        let (dir, file) = test_file();
        let writer = RangeDiskWriter::spawn(file);
        let jobs = writer.sender();
        let attempt = attempt();
        let (reply_tx, reply_rx) = oneshot::channel();

        jobs.send(RangeWriteJob::new(attempt, 4, b"efgh".to_vec(), reply_tx))
            .await
            .unwrap();

        assert_eq!(
            reply_rx.await.unwrap(),
            RangeWriteResult::Written {
                attempt,
                range: range(4, 8),
            }
        );
        drop(jobs);
        assert!(writer.shutdown().await.is_empty());

        let mut written = Vec::new();
        File::open(dir.path().join("part.bin"))
            .unwrap()
            .read_to_end(&mut written)
            .unwrap();
        assert_eq!(&written[4..8], b"efgh");
    }

    #[tokio::test]
    async fn disk_writer_returns_orphaned_result_when_worker_is_gone() {
        let (_dir, file) = test_file();
        let writer = RangeDiskWriter::spawn(file);
        let jobs = writer.sender();
        let attempt = attempt();
        let (reply_tx, reply_rx) = oneshot::channel();

        jobs.send(RangeWriteJob::new(attempt, 0, b"abcd".to_vec(), reply_tx))
            .await
            .unwrap();
        drop(reply_rx);
        drop(jobs);

        assert_eq!(
            writer.shutdown().await,
            vec![RangeWriteResult::Written {
                attempt,
                range: range(0, 4),
            }]
        );
    }

    #[tokio::test]
    async fn disk_writer_shuts_down_when_sender_closes() {
        let (_dir, file) = test_file();
        let writer = RangeDiskWriter::spawn(file);
        let jobs = writer.sender();

        drop(jobs);

        assert!(writer.shutdown().await.is_empty());
    }

    #[test]
    fn disk_writer_maps_disk_full_and_permission_errors_to_fatal() {
        for kind in [io::ErrorKind::StorageFull, io::ErrorKind::PermissionDenied] {
            assert!(matches!(
                failure_from_io(io::Error::from(kind)),
                WorkerFailure::FatalIo { .. }
            ));
        }
    }

    #[test]
    fn disk_writer_maps_other_io_errors_to_retryable() {
        assert!(matches!(
            failure_from_io(io::Error::from(io::ErrorKind::Interrupted)),
            WorkerFailure::RetryableIo { .. }
        ));
    }
}
