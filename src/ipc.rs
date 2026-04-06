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

//! Local HTTP server that lets the browser extension hand downloads to Ophelia.
//!
//! Bound to 127.0.0.1 only
//! The extension discovers it via GET /health, then POSTs downloads to /download.

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;

use crate::engine::AddDownloadRequest;

/// Browser-extension transport payload.
#[derive(Debug, Deserialize)]
struct BrowserDownloadRequest {
    pub url: String,
    pub filename: Option<String>,
}

/// App-owned IPC ingress handle.
///
/// The browser-extension transport runs on its own runtime so the download
/// engine does not have to own ingress lifecycles directly.
pub struct IpcServer {
    #[allow(dead_code)] // held to keep the runtime and server alive
    runtime: Runtime,
    rx: mpsc::UnboundedReceiver<AddDownloadRequest>,
}

impl IpcServer {
    pub fn start(port: u16) -> Self {
        let runtime = Runtime::new().expect("failed to create IPC runtime");
        let (tx, rx) = mpsc::unbounded_channel();
        runtime.spawn(serve(port, tx));
        Self { runtime, rx }
    }

    pub fn try_recv(&mut self) -> Option<AddDownloadRequest> {
        self.rx.try_recv().ok()
    }
}

#[derive(Serialize)]
struct HealthResponse {
    app: &'static str,
    version: &'static str,
}

/// Bind and serve until the process exits. Soft-fails on port conflict (logged as warn).
pub async fn serve(port: u16, tx: mpsc::UnboundedSender<AddDownloadRequest>) {
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("IPC server could not bind on port {port}: {e}");
            return;
        }
    };

    tracing::info!("IPC server listening on 127.0.0.1:{port}");
    serve_listener(listener, tx).await;
}

async fn serve_listener(
    listener: tokio::net::TcpListener,
    tx: mpsc::UnboundedSender<AddDownloadRequest>,
) {
    axum::serve(listener, router(tx)).await.ok();
}

fn router(tx: mpsc::UnboundedSender<AddDownloadRequest>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/download", post(add_download))
        .layer(CorsLayer::permissive())
        .with_state(Arc::new(tx))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        app: "ophelia",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn add_download(
    State(tx): State<Arc<mpsc::UnboundedSender<AddDownloadRequest>>>,
    Json(req): Json<BrowserDownloadRequest>,
) -> StatusCode {
    let request = AddDownloadRequest::from_url_with_suggested_filename(req.url, req.filename);
    if tx.send(request).is_ok() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use serde_json::json;
    use std::path::PathBuf;

    async fn spawn_test_server(
        tx: mpsc::UnboundedSender<AddDownloadRequest>,
    ) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            serve_listener(listener, tx).await;
        });
        addr
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn health_endpoint_reports_app_and_version() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let addr = spawn_test_server(tx).await;

        let response = reqwest::get(format!("http://{addr}/health")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value =
            serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(body["app"], "ophelia");
        assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn download_endpoint_normalizes_browser_payload_into_add_request() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let addr = spawn_test_server(tx).await;
        let client = reqwest::Client::new();

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
        let request = rx.recv().await.unwrap();
        assert_eq!(request.url(), "https://example.com/video.mp4");
        assert_eq!(
            request.suggested_filename.as_deref(),
            Some("browser-name.mp4")
        );
        assert_eq!(
            request.preview_destination(&Settings {
                default_download_dir: Some(PathBuf::from("/tmp/downloads")),
                ..Settings::default()
            }),
            PathBuf::from("/tmp/downloads/browser-name.mp4")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn download_endpoint_returns_service_unavailable_when_receiver_closed() {
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx);
        let addr = spawn_test_server(tx).await;
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
