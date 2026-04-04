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

pub const PORT: u16 = 7373;

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
    pub fn start() -> Self {
        let runtime = Runtime::new().expect("failed to create IPC runtime");
        let (tx, rx) = mpsc::unbounded_channel();
        runtime.spawn(serve(tx));
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
pub async fn serve(tx: mpsc::UnboundedSender<AddDownloadRequest>) {
    let app = Router::new()
        .route("/health", get(health))
        .route("/download", post(add_download))
        .layer(CorsLayer::permissive())
        .with_state(Arc::new(tx));

    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("IPC server could not bind on port {PORT}: {e}");
            return;
        }
    };

    tracing::info!("IPC server listening on 127.0.0.1:{PORT}");
    axum::serve(listener, app).await.ok();
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
