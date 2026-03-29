//! HTTP/HTTPS protocol implementation.
//! Everything in this module is specific to HTTP: range probing, chunked
//! parallel requests, and HTTP-specific download configuration.

pub mod config;
pub mod task;

pub use config::HttpDownloadConfig;
pub use task::download_task;
