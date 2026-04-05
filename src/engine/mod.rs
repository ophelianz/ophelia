pub mod alloc;
pub mod chunk;
pub mod destination;
mod engine;
pub mod http;
mod provider;
pub mod spec;
pub mod state;
pub mod types;

pub use engine::DownloadEngine;
pub use spec::{AddDownloadRequest, DownloadSource, DownloadSpec, RestoredDownload};
pub use types::*;
