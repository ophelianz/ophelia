//! Local HTTP server that lets the browser extension hand downloads to Ophelia.
//!
//! Bound to 127.0.0.1 only
//! The extension discovers it via GET /health, then POSTs downloads to /download.

use axum::{extract::State, http::StatusCode, routing::{get, post}, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;

pub const PORT: u16 = 7373;

/// A download request sent by the browser extension.
#[derive(Debug, Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    pub filename: Option<String>,
}

#[derive(Serialize)]
struct HealthResponse {
    app: &'static str,
    version: &'static str,
}

/// Bind and serve until the process exits. Soft-fails on port conflict (logged as warn).
pub async fn serve(tx: mpsc::UnboundedSender<DownloadRequest>) {
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
    State(tx): State<Arc<mpsc::UnboundedSender<DownloadRequest>>>,
    Json(req): Json<DownloadRequest>,
) -> StatusCode {
    if tx.send(req).is_ok() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
