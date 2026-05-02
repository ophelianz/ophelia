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

//! Local HTTP server for browser-extension downloads
//!
//! Bound to 127.0.0.1 only
//! The extension checks GET /health, then POSTs downloads to /download

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tower_http::cors::CorsLayer;

use crate::engine::AddTransferRequest;
use ophelia::service::{OpheliaClient, TransferRequest};

/// Browser-extension request body
#[derive(Debug, Deserialize)]
struct BrowserTransferRequest {
    pub url: String,
    pub filename: Option<String>,
}

/// App-owned IPC handle
///
/// The browser-extension server runs outside the download engine
pub struct IpcServer {
    task: Option<JoinHandle<()>>,
}

impl IpcServer {
    pub fn start(port: u16, runtime: &Handle, client: OpheliaClient) -> Self {
        let task = runtime.spawn(serve(port, client));
        Self { task: Some(task) }
    }

    #[cfg(test)]
    pub fn disabled() -> Self {
        Self { task: None }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[derive(Serialize)]
struct HealthResponse {
    app: &'static str,
    version: &'static str,
}

/// Bind and serve until the process exits
/// Port conflict is logged as a warning
pub async fn serve(port: u16, client: OpheliaClient) {
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("IPC server could not bind on port {port}: {e}");
            return;
        }
    };

    tracing::info!("IPC server listening on 127.0.0.1:{port}");
    serve_listener(listener, client).await;
}

async fn serve_listener(listener: tokio::net::TcpListener, client: OpheliaClient) {
    axum::serve(listener, router(client)).await.ok();
}

fn router(client: OpheliaClient) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/download", post(add_download))
        .layer(CorsLayer::permissive())
        .with_state(Arc::new(client))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        app: "ophelia",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn add_download(
    State(client): State<Arc<OpheliaClient>>,
    Json(req): Json<BrowserTransferRequest>,
) -> StatusCode {
    let request = AddTransferRequest::from_url_with_suggested_filename(req.url, req.filename);
    match client.add(TransferRequest::from_add_request(request)).await {
        Ok(_) => StatusCode::OK,
        Err(error) => {
            tracing::warn!("browser download add failed: {error}");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{TransferStatus, TransferSummary};
    use ophelia::service::OpheliaService;
    use serde_json::json;
    use std::time::Duration;

    async fn spawn_test_server(client: OpheliaClient) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            serve_listener(listener, client).await;
        });
        addr
    }

    fn test_service() -> (tempfile::TempDir, OpheliaService) {
        let dir = tempfile::tempdir().unwrap();
        let paths = ophelia::ProfilePaths::new(
            dir.path().join("downloads.db"),
            dir.path().join("downloads"),
        );
        let settings = ophelia::ServiceSettings {
            default_download_dir: Some(dir.path().join("downloads")),
            ..ophelia::ServiceSettings::default()
        };
        let host = OpheliaService::start_with_settings(
            &tokio::runtime::Handle::current(),
            paths,
            settings,
        )
        .unwrap();
        (dir, host)
    }

    async fn next_transfer_changed(
        mut subscription: ophelia::service::OpheliaSubscription,
    ) -> TransferSummary {
        loop {
            let update = tokio::time::timeout(Duration::from_secs(2), subscription.next_update())
                .await
                .unwrap()
                .unwrap();
            if let Some(snapshot) = update.lifecycle.transfers.summaries().into_iter().next() {
                return snapshot;
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_endpoint_reports_app_and_version() {
        let (_dir, host) = test_service();
        let addr = spawn_test_server(host.client()).await;
        let response = reqwest::get(format!("http://{addr}/health")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(body["app"], "ophelia");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn download_endpoint_adds_through_service_client() {
        let (_dir, host) = test_service();
        let service_client = host.client();
        let subscription = service_client.subscribe().await.unwrap();
        let addr = spawn_test_server(service_client.clone()).await;
        let client = reqwest::Client::new();
        let transfer = tokio::spawn(async move { next_transfer_changed(subscription).await });

        let response = client
            .post(format!("http://{addr}/download"))
            .header("content-type", "application/json")
            .body(
                json!({
                    "url": "https://example.com/video.mp4",
                    "filename": "browser-name.mp4"
                })
                .to_string(),
            )
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let snapshot = transfer.await.unwrap();
        assert_eq!(snapshot.status, TransferStatus::Pending);
        assert_eq!(snapshot.source_label, "https://example.com/video.mp4");
        assert_eq!(
            snapshot
                .destination
                .file_name()
                .and_then(|name| name.to_str()),
            Some("browser-name.mp4")
        );
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn download_endpoint_returns_service_unavailable_when_service_closed() {
        let (_dir, host) = test_service();
        let service_client = host.client();
        host.shutdown().await.unwrap();
        let addr = spawn_test_server(service_client).await;
        let client = reqwest::Client::new();

        let response = client
            .post(format!("http://{addr}/download"))
            .header("content-type", "application/json")
            .body(
                json!({
                    "url": "https://example.com/file.zip",
                    "filename": null
                })
                .to_string(),
            )
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
