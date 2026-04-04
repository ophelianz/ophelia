//! HTTP/HTTPS protocol implementation.
//! Everything in this module is specific to HTTP: range probing, chunked
//! parallel requests, and HTTP-specific download configuration.

pub mod config;
pub mod task;
pub mod throttle;

mod error;
mod health;
mod probe;
mod progress;
mod single;
mod steal;
pub mod worker;

pub use config::HttpDownloadConfig;
pub use task::{TaskFinalState, download_task};
pub use throttle::TokenBucket;
