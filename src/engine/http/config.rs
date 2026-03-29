//! Per-download configuration for HTTP/HTTPS downloads.
//! Fields here are intentionally HTTP-specific: connection count, stall detection,
//! and retry behavior are concepts that don't apply to all protocols.

pub struct HttpDownloadConfig {
    pub max_connections: usize,
    pub write_buffer_size: usize,
    pub progress_interval_ms: u64,
    pub stall_timeout_secs: u64,
    pub max_retries_per_chunk: u32,
}

impl Default for HttpDownloadConfig {
    fn default() -> Self {
        Self {
            max_connections: 8,
            write_buffer_size: 64 * 1024,
            progress_interval_ms: 100,
            stall_timeout_secs: 30,
            max_retries_per_chunk: 5,
        }
    }
}
