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

//! Downloads one byte range
//!
//! Sends one ranged HTTP request, hands buffered bytes to the disk writer,
//! and reports events back to the runner

#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::disk_writer::{RangeWriteJob, RangeWriteResult, RangeWriteSender};
use super::events::{WorkerEvent, WorkerFailure};
use super::ranges::ByteRange;
use super::scheduler::ActiveAttempt;
use super::throttle::Throttle;

pub(super) struct RangeWorkerConfig {
    pub(super) client: Arc<reqwest::Client>,
    pub(super) url: String,
    pub(super) attempt: ActiveAttempt,
    pub(super) live_stop_at: Arc<AtomicU64>,
    pub(super) write_jobs: RangeWriteSender,
    pub(super) write_buffer_size: usize,
    pub(super) stall_timeout: Duration,
    pub(super) pause_token: CancellationToken,
    pub(super) health_retry_token: CancellationToken,
    pub(super) hedge_lost_token: CancellationToken,
    pub(super) throttle: Arc<Throttle>,
    pub(super) events: mpsc::Sender<WorkerEvent>,
}

pub(super) async fn run_range_worker(config: RangeWorkerConfig) {
    let Some(remaining) = config.attempt.remaining_range() else {
        send_event(
            &config.events,
            WorkerEvent::Finished {
                attempt: config.attempt.id(),
            },
        )
        .await;
        return;
    };

    let Some(request_range) = current_request_range(remaining, config.live_stop_at.as_ref()) else {
        send_event(
            &config.events,
            WorkerEvent::Finished {
                attempt: config.attempt.id(),
            },
        )
        .await;
        return;
    };

    RangeWorker::new(config, request_range).run().await;
}

struct RangeWorker {
    config: RangeWorkerConfig,
    request_range: ByteRange,
    offset: u64,
    buffer: Vec<u8>,
    flush_size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerSignal {
    Pause,
    HedgeLost,
    HealthRetry,
}

enum ReadOutcome<T> {
    Bytes(T),
    EndOfBody,
    Done,
}

impl RangeWorker {
    fn new(config: RangeWorkerConfig, request_range: ByteRange) -> Self {
        let flush_size = config.write_buffer_size.max(1);
        let buffer = Vec::with_capacity(config.write_buffer_size);
        Self {
            config,
            request_range,
            offset: request_range.start(),
            buffer,
            flush_size,
        }
    }

    async fn run(mut self) {
        let Some(response) = self.request_response().await else {
            return;
        };

        if let Some(failure) = response_failure(&response, self.request_range) {
            self.fail(failure).await;
            return;
        }

        self.stream_body(response).await;
    }

    async fn request_response(&self) -> Option<reqwest::Response> {
        let range_header = format!(
            "bytes={}-{}",
            self.request_range.start(),
            self.request_range.end() - 1
        );

        tokio::select! {
            biased;
            _ = self.config.pause_token.cancelled() => {
                self.report_signal(WorkerSignal::Pause).await;
                None
            }
            _ = self.config.hedge_lost_token.cancelled() => {
                self.report_signal(WorkerSignal::HedgeLost).await;
                None
            }
            _ = self.config.health_retry_token.cancelled() => {
                self.report_signal(WorkerSignal::HealthRetry).await;
                None
            }
            result = self.config.client
                .get(&self.config.url)
                .header("Range", range_header)
                .send() => self.request_result(result).await,
        }
    }

    async fn stream_body(&mut self, response: reqwest::Response) {
        let mut stream = response.bytes_stream();

        while !self.buffer_has_reached_live_stop() {
            let read = tokio::select! {
                biased;
                _ = self.config.pause_token.cancelled() => {
                    self.stop_after_flush(WorkerSignal::Pause).await;
                    return;
                }
                _ = self.config.hedge_lost_token.cancelled() => {
                    self.stop_after_flush(WorkerSignal::HedgeLost).await;
                    return;
                }
                _ = self.config.health_retry_token.cancelled() => {
                    self.stop_after_flush(WorkerSignal::HealthRetry).await;
                    return;
                }
                result = tokio::time::timeout(self.config.stall_timeout, stream.next()) => result,
            };

            let should_stop = match self.read_outcome(read).await {
                ReadOutcome::Bytes(bytes) => self.handle_bytes(&bytes).await,
                ReadOutcome::EndOfBody => break,
                ReadOutcome::Done => return,
            };
            if should_stop {
                return;
            }
        }

        self.finish_after_body().await;
    }

    async fn handle_bytes(&mut self, bytes: &[u8]) -> bool {
        self.data_received(bytes.len()).await;
        self.accept_bytes(bytes).await
    }

    async fn request_result(
        &self,
        result: Result<reqwest::Response, reqwest::Error>,
    ) -> Option<reqwest::Response> {
        match result {
            Ok(response) => Some(response),
            Err(error) => {
                self.fail(request_error_failure(&error)).await;
                None
            }
        }
    }

    async fn read_outcome<T>(
        &self,
        read: Result<Option<Result<T, reqwest::Error>>, tokio::time::error::Elapsed>,
    ) -> ReadOutcome<T> {
        match read {
            Err(_elapsed) => {
                self.fail(WorkerFailure::Timeout).await;
                ReadOutcome::Done
            }
            Ok(None) => ReadOutcome::EndOfBody,
            Ok(Some(Err(error))) => {
                self.fail(request_error_failure(&error)).await;
                ReadOutcome::Done
            }
            Ok(Some(Ok(bytes))) => ReadOutcome::Bytes(bytes),
        }
    }

    async fn accept_bytes(&mut self, bytes: &[u8]) -> bool {
        match self.append_bytes(bytes) {
            AppendOutcome::Accepted => {}
            AppendOutcome::ReachedStop => {
                self.finish_after_flush().await;
                return true;
            }
            AppendOutcome::BadResponse => {
                self.fail(WorkerFailure::BadRangeResponse { status: 206 })
                    .await;
                return true;
            }
        }

        let wait = self.config.throttle.consume(bytes.len() as u64);
        if !wait.is_zero() && self.sleep_or_stop(wait).await {
            return true;
        }

        self.buffer.len() >= self.flush_size && self.flush_buffer().await.is_err()
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> AppendOutcome {
        append_bytes_with_live_stop(
            self.request_range,
            self.live_stop(),
            self.offset,
            &mut self.buffer,
            bytes,
        )
    }

    async fn sleep_or_stop(&mut self, wait: Duration) -> bool {
        tokio::select! {
            biased;
            _ = self.config.pause_token.cancelled() => {
                self.stop_after_flush(WorkerSignal::Pause).await;
                true
            }
            _ = self.config.hedge_lost_token.cancelled() => {
                self.stop_after_flush(WorkerSignal::HedgeLost).await;
                true
            }
            _ = self.config.health_retry_token.cancelled() => {
                self.stop_after_flush(WorkerSignal::HealthRetry).await;
                true
            }
            _ = tokio::time::sleep(wait) => {
                if self.buffer_has_reached_live_stop() {
                    self.finish_after_flush().await;
                    true
                } else {
                    false
                }
            }
        }
    }

    async fn finish_after_body(&mut self) {
        if self.flush_buffer().await.is_err() {
            return;
        }

        if self.offset >= self.live_stop() {
            self.finish().await;
        } else {
            self.fail(WorkerFailure::BadRangeResponse { status: 206 })
                .await;
        }
    }

    async fn finish_after_flush(&mut self) {
        if self.flush_buffer().await.is_ok() {
            self.finish().await;
        }
    }

    async fn stop_after_flush(&mut self, signal: WorkerSignal) {
        if self.flush_buffer().await.is_ok() {
            self.report_signal(signal).await;
        }
    }

    fn buffer_has_reached_live_stop(&self) -> bool {
        self.offset.saturating_add(self.buffer.len() as u64) >= self.live_stop()
    }

    fn live_stop(&self) -> u64 {
        current_stop_at(self.request_range, self.config.live_stop_at.as_ref())
    }

    async fn flush_buffer(&mut self) -> Result<(), ()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let writable_len = self
            .live_stop()
            .saturating_sub(self.offset)
            .min(self.buffer.len() as u64) as usize;
        if writable_len == 0 {
            self.buffer.clear();
            return Ok(());
        }

        let Some(written) = ByteRange::from_len(self.offset, writable_len as u64) else {
            self.fail(WorkerFailure::FatalIo {
                message: "write range overflow".to_string(),
            })
            .await;
            return Err(());
        };

        let bytes = self.take_writable_bytes(writable_len);
        let (reply_tx, reply_rx) = oneshot::channel();
        let job = RangeWriteJob::new(self.config.attempt.id(), self.offset, bytes, reply_tx);
        if self.config.write_jobs.send(job).await.is_err() {
            self.fail(WorkerFailure::FatalIo {
                message: "disk writer stopped".to_string(),
            })
            .await;
            return Err(());
        }

        match reply_rx.await {
            Ok(RangeWriteResult::Written { range, .. }) => {
                debug_assert_eq!(range, written);
                self.event(WorkerEvent::BytesWritten {
                    attempt: self.config.attempt.id(),
                    written: range,
                })
                .await;
                self.offset = range.end();
                Ok(())
            }
            Ok(RangeWriteResult::Failed { failure, .. }) => {
                self.fail(failure).await;
                Err(())
            }
            Err(_closed) => {
                self.fail(WorkerFailure::FatalIo {
                    message: "disk writer dropped write result".to_string(),
                })
                .await;
                Err(())
            }
        }
    }

    fn take_writable_bytes(&mut self, writable_len: usize) -> Vec<u8> {
        if writable_len == self.buffer.len() {
            return std::mem::take(&mut self.buffer);
        }

        let bytes = self.buffer[..writable_len].to_vec();
        self.buffer.clear();
        bytes
    }

    async fn finish(&self) {
        self.event(WorkerEvent::Finished {
            attempt: self.config.attempt.id(),
        })
        .await;
    }

    async fn data_received(&self, bytes: usize) {
        self.event(WorkerEvent::DataReceived {
            attempt: self.config.attempt.id(),
            bytes: bytes as u64,
        })
        .await;
    }

    async fn fail(&self, failure: WorkerFailure) {
        send_failure(&self.config.events, self.config.attempt, failure).await;
    }

    async fn report_signal(&self, signal: WorkerSignal) {
        match signal {
            WorkerSignal::Pause => {
                self.event(WorkerEvent::Paused {
                    attempt: self.config.attempt.id(),
                })
                .await
            }
            WorkerSignal::HedgeLost => self.fail(WorkerFailure::HedgeLost).await,
            WorkerSignal::HealthRetry => self.fail(WorkerFailure::HealthRetry).await,
        }
    }

    async fn event(&self, event: WorkerEvent) {
        send_event(&self.config.events, event).await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppendOutcome {
    Accepted,
    ReachedStop,
    BadResponse,
}

fn append_bytes_with_live_stop(
    request_range: ByteRange,
    live_stop: u64,
    offset: u64,
    buffer: &mut Vec<u8>,
    bytes: &[u8],
) -> AppendOutcome {
    let Some(buffered_end) = offset.checked_add(buffer.len() as u64) else {
        return AppendOutcome::BadResponse;
    };
    if buffered_end >= live_stop {
        return AppendOutcome::ReachedStop;
    }

    let allowed = live_stop - buffered_end;
    if bytes.len() as u64 <= allowed {
        buffer.extend_from_slice(bytes);
        return AppendOutcome::Accepted;
    }

    if live_stop < request_range.end() {
        let allowed = allowed as usize;
        buffer.extend_from_slice(&bytes[..allowed]);
        AppendOutcome::ReachedStop
    } else {
        AppendOutcome::BadResponse
    }
}

fn current_request_range(remaining: ByteRange, live_stop_at: &AtomicU64) -> Option<ByteRange> {
    ByteRange::new(
        remaining.start(),
        current_stop_at(remaining, live_stop_at).min(remaining.end()),
    )
}

fn current_stop_at(request_range: ByteRange, live_stop_at: &AtomicU64) -> u64 {
    live_stop_at
        .load(Ordering::Acquire)
        .min(request_range.end())
        .max(request_range.start())
}

fn response_failure(
    response: &reqwest::Response,
    request_range: ByteRange,
) -> Option<WorkerFailure> {
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        return (!content_range_matches(response.headers(), request_range)).then_some(
            WorkerFailure::BadRangeResponse {
                status: response.status().as_u16(),
            },
        );
    }

    Some(if response.status().is_success() {
        WorkerFailure::BadRangeResponse {
            status: response.status().as_u16(),
        }
    } else {
        status_failure(response.status(), response.headers())
    })
}

fn content_range_matches(headers: &reqwest::header::HeaderMap, request_range: ByteRange) -> bool {
    let Some(value) = headers
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };

    parse_content_range(value).is_some_and(|(start, end)| {
        start == request_range.start() && end == request_range.end().saturating_sub(1)
    })
}

fn parse_content_range(value: &str) -> Option<(u64, u64)> {
    let value = value.trim();
    let range = value.strip_prefix("bytes ")?;
    let (range, _total) = range.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((start.trim().parse().ok()?, end.trim().parse().ok()?))
}

async fn send_failure(
    events: &mpsc::Sender<WorkerEvent>,
    attempt: ActiveAttempt,
    failure: WorkerFailure,
) {
    send_event(
        events,
        WorkerEvent::Failed {
            attempt: attempt.id(),
            failure,
        },
    )
    .await;
}

async fn send_event(events: &mpsc::Sender<WorkerEvent>, event: WorkerEvent) {
    let _ = events.send(event).await;
}

fn request_error_failure(error: &reqwest::Error) -> WorkerFailure {
    if error.is_timeout() {
        WorkerFailure::Timeout
    } else {
        WorkerFailure::RetryableHttp { retry_after: None }
    }
}

fn status_failure(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
) -> WorkerFailure {
    match status.as_u16() {
        429 => WorkerFailure::RetryableHttp {
            retry_after: headers
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(Duration::from_secs),
        },
        500..=599 => WorkerFailure::RetryableHttp { retry_after: None },
        code => WorkerFailure::NonRetryableHttp { status: code },
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::{
        AppendOutcome, RangeWorker, RangeWorkerConfig, WorkerSignal, append_bytes_with_live_stop,
        run_range_worker,
    };
    use crate::engine::http::{
        disk_writer::RangeDiskWriter,
        events::{WorkerEvent, WorkerFailure},
        ranges::ByteRange,
        scheduler::RangeScheduler,
        throttle::{Throttle, TokenBucket},
    };

    fn range(start: u64, end: u64) -> ByteRange {
        ByteRange::new(start, end).unwrap()
    }

    fn throttle() -> Arc<Throttle> {
        Arc::new(Throttle {
            per_download: Arc::new(TokenBucket::new(0)),
            global: Arc::new(TokenBucket::new(0)),
        })
    }

    fn test_file() -> (tempfile::TempDir, std::fs::File) {
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

    fn events_from(rx: &mut mpsc::Receiver<WorkerEvent>) -> Vec<WorkerEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    fn worker_config(
        url: String,
        events: mpsc::Sender<WorkerEvent>,
    ) -> (RangeWorkerConfig, tempfile::TempDir, RangeDiskWriter) {
        let (dir, file) = test_file();
        let writer = RangeDiskWriter::spawn(file);
        let mut scheduler = RangeScheduler::new(8, [range(0, 8)]);
        let attempt = scheduler.start_next_attempt().unwrap();
        let config = RangeWorkerConfig {
            client: Arc::new(reqwest::Client::new()),
            url,
            attempt,
            live_stop_at: Arc::new(AtomicU64::new(attempt.stop_at())),
            write_jobs: writer.sender(),
            write_buffer_size: 4,
            stall_timeout: Duration::from_secs(5),
            pause_token: CancellationToken::new(),
            health_retry_token: CancellationToken::new(),
            hedge_lost_token: CancellationToken::new(),
            throttle: throttle(),
            events,
        };
        (config, dir, writer)
    }

    #[test]
    fn append_bytes_stops_at_live_stop_after_steal() {
        let request_range = range(0, 8);
        let mut buffer = Vec::new();

        let outcome = append_bytes_with_live_stop(request_range, 4, 0, &mut buffer, b"abcdefgh");

        assert_eq!(outcome, AppendOutcome::ReachedStop);
        assert_eq!(buffer, b"abcd");
    }

    #[test]
    fn append_bytes_rejects_long_response_without_live_stop_shrink() {
        let request_range = range(0, 8);
        let mut buffer = Vec::new();

        let outcome = append_bytes_with_live_stop(request_range, 8, 0, &mut buffer, b"abcdefghi");

        assert_eq!(outcome, AppendOutcome::BadResponse);
        assert!(buffer.is_empty());
    }

    #[tokio::test]
    async fn flush_buffer_trims_to_live_stop_before_write() {
        let (tx, mut rx) = mpsc::channel(256);
        let (config, dir, writer) =
            worker_config("http://example.invalid/file.bin".to_string(), tx);
        config.live_stop_at.store(4, Ordering::Release);
        let mut worker = RangeWorker::new(config, range(0, 8));
        worker.buffer = b"abcdefgh".to_vec();

        worker.flush_buffer().await.unwrap();
        assert_eq!(worker.offset, 4);
        assert!(worker.buffer.is_empty());
        drop(worker);
        assert!(writer.shutdown().await.is_empty());
        assert!(matches!(
            events_from(&mut rx).as_slice(),
            [WorkerEvent::BytesWritten {
                written,
                ..
            }] if *written == range(0, 4)
        ));

        let mut written = Vec::new();
        std::fs::File::open(dir.path().join("part.bin"))
            .unwrap()
            .read_to_end(&mut written)
            .unwrap();
        assert_eq!(written, b"abcd");
    }

    #[tokio::test]
    async fn pause_flushes_buffer_before_paused_event() {
        let (tx, mut rx) = mpsc::channel(256);
        let (config, dir, writer) =
            worker_config("http://example.invalid/file.bin".to_string(), tx);
        let mut worker = RangeWorker::new(config, range(0, 8));
        worker.buffer = b"abcd".to_vec();

        worker.stop_after_flush(WorkerSignal::Pause).await;
        assert_eq!(worker.offset, 4);
        drop(worker);
        assert!(writer.shutdown().await.is_empty());

        assert!(matches!(
            events_from(&mut rx).as_slice(),
            [
                WorkerEvent::BytesWritten { written, .. },
                WorkerEvent::Paused { .. }
            ] if *written == range(0, 4)
        ));

        let mut written = Vec::new();
        std::fs::File::open(dir.path().join("part.bin"))
            .unwrap()
            .read_to_end(&mut written)
            .unwrap();
        assert_eq!(written, b"abcd");
    }

    #[tokio::test]
    async fn hedge_lost_flushes_buffer_before_failure_event() {
        let (tx, mut rx) = mpsc::channel(256);
        let (config, dir, writer) =
            worker_config("http://example.invalid/file.bin".to_string(), tx);
        let mut worker = RangeWorker::new(config, range(0, 8));
        worker.buffer = b"abcd".to_vec();

        worker.stop_after_flush(WorkerSignal::HedgeLost).await;
        assert_eq!(worker.offset, 4);
        drop(worker);
        assert!(writer.shutdown().await.is_empty());

        assert!(matches!(
            events_from(&mut rx).as_slice(),
            [
                WorkerEvent::BytesWritten { written, .. },
                WorkerEvent::Failed {
                    failure: WorkerFailure::HedgeLost,
                    ..
                }
            ] if *written == range(0, 4)
        ));

        let mut written = Vec::new();
        std::fs::File::open(dir.path().join("part.bin"))
            .unwrap()
            .read_to_end(&mut written)
            .unwrap();
        assert_eq!(written, b"abcd");
    }

    #[tokio::test]
    async fn health_retry_flushes_buffer_before_failure_event() {
        let (tx, mut rx) = mpsc::channel(256);
        let (config, dir, writer) =
            worker_config("http://example.invalid/file.bin".to_string(), tx);
        let mut worker = RangeWorker::new(config, range(0, 8));
        worker.buffer = b"abcd".to_vec();

        worker.stop_after_flush(WorkerSignal::HealthRetry).await;
        assert_eq!(worker.offset, 4);
        drop(worker);
        assert!(writer.shutdown().await.is_empty());

        assert!(matches!(
            events_from(&mut rx).as_slice(),
            [
                WorkerEvent::BytesWritten { written, .. },
                WorkerEvent::Failed {
                    failure: WorkerFailure::HealthRetry,
                    ..
                }
            ] if *written == range(0, 4)
        ));

        let mut written = Vec::new();
        std::fs::File::open(dir.path().join("part.bin"))
            .unwrap()
            .read_to_end(&mut written)
            .unwrap();
        assert_eq!(written, b"abcd");
    }

    #[tokio::test]
    async fn range_worker_writes_bytes_and_reports_finished() {
        let data = b"abcdefgh".to_vec();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/file.bin"))
            .respond_with(
                ResponseTemplate::new(206)
                    .set_body_bytes(data.clone())
                    .insert_header("content-range", "bytes 0-7/8"),
            )
            .mount(&server)
            .await;

        let (tx, mut rx) = mpsc::channel(256);
        let (config, dir, writer) = worker_config(format!("{}/file.bin", server.uri()), tx);
        run_range_worker(config).await;
        assert!(writer.shutdown().await.is_empty());

        let events = events_from(&mut rx);
        assert!(matches!(
            events.first(),
            Some(WorkerEvent::DataReceived { bytes, .. }) if *bytes == 8
        ));
        assert!(matches!(
            events.get(1),
            Some(WorkerEvent::BytesWritten {
                written,
                ..
            }) if *written == range(0, 8)
        ));
        assert!(matches!(events.last(), Some(WorkerEvent::Finished { .. })));

        let mut written = Vec::new();
        std::fs::File::open(dir.path().join("part.bin"))
            .unwrap()
            .read_to_end(&mut written)
            .unwrap();
        assert_eq!(written, data);
    }

    #[tokio::test]
    async fn range_worker_rejects_successful_non_partial_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/file.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"abcdefgh".to_vec()))
            .mount(&server)
            .await;

        let (tx, mut rx) = mpsc::channel(256);
        let (config, _dir, writer) = worker_config(format!("{}/file.bin", server.uri()), tx);
        run_range_worker(config).await;
        assert!(writer.shutdown().await.is_empty());

        assert!(matches!(
            events_from(&mut rx).as_slice(),
            [WorkerEvent::Failed {
                failure: WorkerFailure::BadRangeResponse { status: 200 },
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn range_worker_rejects_wrong_content_range() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/file.bin"))
            .respond_with(
                ResponseTemplate::new(206)
                    .set_body_bytes(b"abcdefgh".to_vec())
                    .insert_header("content-range", "bytes 8-15/16"),
            )
            .mount(&server)
            .await;

        let (tx, mut rx) = mpsc::channel(256);
        let (config, _dir, writer) = worker_config(format!("{}/file.bin", server.uri()), tx);
        run_range_worker(config).await;
        assert!(writer.shutdown().await.is_empty());

        assert!(matches!(
            events_from(&mut rx).as_slice(),
            [WorkerEvent::Failed {
                failure: WorkerFailure::BadRangeResponse { status: 206 },
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn range_worker_rejects_missing_content_range() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/file.bin"))
            .respond_with(ResponseTemplate::new(206).set_body_bytes(b"abcdefgh".to_vec()))
            .mount(&server)
            .await;

        let (tx, mut rx) = mpsc::channel(256);
        let (config, _dir, writer) = worker_config(format!("{}/file.bin", server.uri()), tx);
        run_range_worker(config).await;
        assert!(writer.shutdown().await.is_empty());

        assert!(matches!(
            events_from(&mut rx).as_slice(),
            [WorkerEvent::Failed {
                failure: WorkerFailure::BadRangeResponse { status: 206 },
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn range_worker_rejects_short_partial_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/file.bin"))
            .respond_with(
                ResponseTemplate::new(206)
                    .set_body_bytes(b"abcd".to_vec())
                    .insert_header("content-range", "bytes 0-7/8"),
            )
            .mount(&server)
            .await;

        let (tx, mut rx) = mpsc::channel(256);
        let (config, _dir, writer) = worker_config(format!("{}/file.bin", server.uri()), tx);
        run_range_worker(config).await;
        assert!(writer.shutdown().await.is_empty());

        let events = events_from(&mut rx);
        assert!(matches!(
            events.last(),
            Some(WorkerEvent::Failed {
                failure: WorkerFailure::BadRangeResponse { status: 206 },
                ..
            })
        ));
    }

    #[tokio::test]
    async fn range_worker_rejects_long_partial_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/file.bin"))
            .respond_with(
                ResponseTemplate::new(206)
                    .set_body_bytes(b"abcdefghi".to_vec())
                    .insert_header("content-range", "bytes 0-7/8"),
            )
            .mount(&server)
            .await;

        let (tx, mut rx) = mpsc::channel(256);
        let (config, _dir, writer) = worker_config(format!("{}/file.bin", server.uri()), tx);
        run_range_worker(config).await;
        assert!(writer.shutdown().await.is_empty());

        let events = events_from(&mut rx);
        assert!(matches!(
            events.last(),
            Some(WorkerEvent::Failed {
                failure: WorkerFailure::BadRangeResponse { status: 206 },
                ..
            })
        ));
    }

    #[tokio::test]
    async fn range_worker_reports_pause_before_request() {
        let server = MockServer::start().await;
        let (tx, mut rx) = mpsc::channel(256);
        let (config, _dir, writer) = worker_config(format!("{}/file.bin", server.uri()), tx);
        config.pause_token.cancel();

        run_range_worker(config).await;
        assert!(writer.shutdown().await.is_empty());

        assert!(matches!(
            events_from(&mut rx).as_slice(),
            [WorkerEvent::Paused { .. }]
        ));
    }

    #[tokio::test]
    async fn range_worker_reports_hedge_lost_before_request() {
        let server = MockServer::start().await;
        let (tx, mut rx) = mpsc::channel(256);
        let (config, _dir, writer) = worker_config(format!("{}/file.bin", server.uri()), tx);
        config.hedge_lost_token.cancel();

        run_range_worker(config).await;
        assert!(writer.shutdown().await.is_empty());

        assert!(matches!(
            events_from(&mut rx).as_slice(),
            [WorkerEvent::Failed {
                failure: WorkerFailure::HedgeLost,
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn range_worker_reports_health_retry_before_request() {
        let server = MockServer::start().await;
        let (tx, mut rx) = mpsc::channel(256);
        let (config, _dir, writer) = worker_config(format!("{}/file.bin", server.uri()), tx);
        config.health_retry_token.cancel();

        run_range_worker(config).await;
        assert!(writer.shutdown().await.is_empty());

        assert!(matches!(
            events_from(&mut rx).as_slice(),
            [WorkerEvent::Failed {
                failure: WorkerFailure::HealthRetry,
                ..
            }]
        ));
    }
}
