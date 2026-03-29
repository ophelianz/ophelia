use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Respond, ResponseTemplate};

use ophelia::engine::http::{download_task, HttpDownloadConfig};
use ophelia::engine::types::{DownloadId, DownloadStatus, ProgressUpdate};

fn test_data(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 256) as u8).collect()
}

fn sha256(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

async fn drain_progress(
    rx: &mut mpsc::UnboundedReceiver<ProgressUpdate>,
) -> Vec<ProgressUpdate> {
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    let mut updates = vec![];
    while let Ok(update) = rx.try_recv() {
        updates.push(update);
    }
    updates
}

fn last_status(updates: &[ProgressUpdate]) -> Option<DownloadStatus> {
    updates.last().map(|u| u.status)
}

struct RangeResponder {
    data: Vec<u8>,
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

#[tokio::test(flavor = "multi_thread")]
async fn parallel_download_with_range_support() {
    let data = test_data(10_000);
    let expected_hash = sha256(&data);

    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(RangeResponder { data: data.clone() })
        .mount(&server)
        .await;

    let url = format!("{}/file.bin", server.uri());
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, mut rx) = mpsc::unbounded_channel();
    download_task(DownloadId(0), url, dest.clone(), HttpDownloadConfig::default(), tx).await;

    let updates = drain_progress(&mut rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Finished));

    let downloaded = std::fs::read(&dest).unwrap();
    assert_eq!(downloaded.len(), data.len());
    assert_eq!(sha256(&downloaded), expected_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn single_stream_fallback_no_range_support() {
    let data = test_data(5_000);
    let expected_hash = sha256(&data);

    let server = MockServer::start().await;

    // Server ignores Range header and returns 200 with full body
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
        .mount(&server)
        .await;

    let url = format!("{}/file.bin", server.uri());
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, mut rx) = mpsc::unbounded_channel();
    download_task(DownloadId(0), url, dest.clone(), HttpDownloadConfig::default(), tx).await;

    let updates = drain_progress(&mut rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Finished));

    let downloaded = std::fs::read(&dest).unwrap();
    assert_eq!(sha256(&downloaded), expected_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn fallback_when_no_content_length() {
    let data = test_data(3_000);
    let expected_hash = sha256(&data);

    let server = MockServer::start().await;

    // Server returns 200 with chunked transfer (no content-length)
    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(data.clone()))
        .mount(&server)
        .await;

    let url = format!("{}/file.bin", server.uri());
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, mut rx) = mpsc::unbounded_channel();
    download_task(DownloadId(0), url, dest.clone(), HttpDownloadConfig::default(), tx).await;

    let updates = drain_progress(&mut rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Finished));

    let downloaded = std::fs::read(&dest).unwrap();
    assert_eq!(sha256(&downloaded), expected_hash);
}

#[tokio::test(flavor = "multi_thread")]
async fn error_on_server_down() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, mut rx) = mpsc::unbounded_channel();
    download_task(
        DownloadId(0),
        "http://127.0.0.1:1".to_string(),
        dest,
        HttpDownloadConfig::default(),
        tx,
    )
    .await;

    let updates = drain_progress(&mut rx).await;
    assert_eq!(last_status(&updates), Some(DownloadStatus::Error));
}

#[tokio::test(flavor = "multi_thread")]
async fn progress_reports_increasing_bytes() {
    let data = test_data(50_000);

    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(RangeResponder { data: data.clone() })
        .mount(&server)
        .await;

    let url = format!("{}/file.bin", server.uri());
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("file.bin");

    let (tx, mut rx) = mpsc::unbounded_channel();
    download_task(DownloadId(0), url, dest, HttpDownloadConfig::default(), tx).await;

    let updates = drain_progress(&mut rx).await;

    let downloading_updates: Vec<&ProgressUpdate> = updates
        .iter()
        .filter(|u| u.status == DownloadStatus::Downloading)
        .collect();

    for window in downloading_updates.windows(2) {
        assert!(window[1].downloaded_bytes >= window[0].downloaded_bytes);
    }
}
