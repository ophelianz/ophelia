//! Provider-neutral add/restore request shapes used by the engine surface.
//!
//! Ophelia currently supports only HTTP, but the top-level engine API should not
//! have to grow a new method or command variant for every provider.

use std::path::{Path, PathBuf};

use crate::engine::http::HttpDownloadConfig;
use crate::engine::types::{ChunkSnapshot, DownloadId};

/// Provider-neutral download request used by the engine surface.
#[derive(Debug, Clone)]
pub struct DownloadSpec {
    pub destination: PathBuf,
    pub source: DownloadSource,
}

impl DownloadSpec {
    pub fn http(url: String, destination: PathBuf, config: HttpDownloadConfig) -> Self {
        Self {
            destination,
            source: DownloadSource::Http { url, config },
        }
    }

    pub fn destination(&self) -> &Path {
        &self.destination
    }

    pub fn url(&self) -> &str {
        self.source.url()
    }
}

#[derive(Debug, Clone)]
pub enum DownloadSource {
    Http {
        url: String,
        config: HttpDownloadConfig,
    },
}

impl DownloadSource {
    pub fn url(&self) -> &str {
        match self {
            Self::Http { url, .. } => url,
        }
    }
}

/// Restored download state fed back into the engine on startup.
#[derive(Debug, Clone)]
pub struct RestoredDownload {
    pub id: DownloadId,
    pub spec: DownloadSpec,
    pub chunks: Vec<ChunkSnapshot>,
}

impl RestoredDownload {
    pub fn http(
        id: DownloadId,
        url: String,
        destination: PathBuf,
        config: HttpDownloadConfig,
        chunks: Vec<ChunkSnapshot>,
    ) -> Self {
        Self {
            id,
            spec: DownloadSpec::http(url, destination, config),
            chunks,
        }
    }
}
