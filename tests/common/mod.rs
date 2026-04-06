/***************************************************
** This file is part of Ophelia, distributed under the
** terms of the GPL License, version 3 or later.
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( tests, plz pass )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::sync::{Semaphore, mpsc};

use ophelia::engine::destination::DestinationPolicy;
use ophelia::engine::http::TokenBucket;
use ophelia::engine::types::{DownloadStatus, ProgressUpdate, TaskRuntimeUpdate};
use ophelia::settings::Settings;

pub fn unlimited_semaphore() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(Semaphore::MAX_PERMITS))
}

pub fn unlimited_throttle() -> Arc<TokenBucket> {
    Arc::new(TokenBucket::new(0))
}

pub fn exact_destination_policy(destination: &std::path::Path) -> DestinationPolicy {
    DestinationPolicy::for_resolved_destination(&Settings::default(), destination)
}

pub fn runtime_updates_channel() -> (
    mpsc::UnboundedSender<TaskRuntimeUpdate>,
    mpsc::UnboundedReceiver<TaskRuntimeUpdate>,
) {
    mpsc::unbounded_channel()
}

pub fn test_data(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 256) as u8).collect()
}

pub fn sha256(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

pub async fn drain_progress(
    rx: &mut mpsc::UnboundedReceiver<ProgressUpdate>,
) -> Vec<ProgressUpdate> {
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut updates = vec![];
    while let Ok(update) = rx.try_recv() {
        updates.push(update);
    }
    updates
}

pub fn last_status(updates: &[ProgressUpdate]) -> Option<DownloadStatus> {
    updates.last().map(|u| u.status)
}

// ---------------------------------------------------------------------------
// Wiremock responders
// ---------------------------------------------------------------------------

use wiremock::{Respond, ResponseTemplate};

#[allow(dead_code)] // shared test helper used selectively by integration test targets
pub struct RangeResponder {
    pub data: Vec<u8>,
}

impl Respond for RangeResponder {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let range_header = request
            .headers
            .get("range")
            .expect("missing range header")
            .to_str()
            .unwrap();
        let range = range_header.strip_prefix("bytes=").unwrap();
        let parts: Vec<&str> = range.split('-').collect();
        let start: usize = parts[0].parse().unwrap();
        let end: usize = parts[1].parse::<usize>().unwrap() + 1;
        let content_range = format!("bytes {}-{}/{}", start, end - 1, self.data.len());
        ResponseTemplate::new(206)
            .set_body_bytes(self.data[start..end].to_vec())
            .insert_header("content-range", content_range.as_str())
    }
}

#[allow(dead_code)] // shared test helper used selectively by integration test targets
pub struct SlowRangeResponder {
    pub data: Vec<u8>,
    pub delay: Duration,
}

impl Respond for SlowRangeResponder {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let range_header = request
            .headers
            .get("range")
            .expect("missing range header")
            .to_str()
            .unwrap();
        let range = range_header.strip_prefix("bytes=").unwrap();
        let parts: Vec<&str> = range.split('-').collect();
        let start: usize = parts[0].parse().unwrap();
        let end: usize = parts[1].parse::<usize>().unwrap() + 1;
        let content_range = format!("bytes {}-{}/{}", start, end - 1, self.data.len());
        ResponseTemplate::new(206)
            .set_delay(self.delay)
            .set_body_bytes(self.data[start..end].to_vec())
            .insert_header("content-range", content_range.as_str())
    }
}
