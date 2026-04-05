/***************************************************
** This file is part of Ophelia, distributed under the
** terms of the GPL License, version 3 or later.
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs, do no evil and behave plz )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

//! Per-chunk download worker.
//!
//! `download_chunk` owns one HTTP range request from byte_start to chunk_end.
//! It streams bytes into a write buffer and flushes with `write_at` (pwrite).
//! Three signals are checked on every iteration via a biased select!:
//!   1. pause_token  → flush buffer, return Paused (offsets saved = disk state)
//!   2. kill_token   → flush buffer, return Killed (health monitor, retry fresh)
//!   3. stream.next() inside a stall timeout → normal data or error

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use super::error::{ChunkError, classify_io_error, classify_status};
use super::throttle::Throttle;

pub async fn download_chunk(
    client: &reqwest::Client,
    url: &str,
    chunk_start: u64,
    chunk_end: u64,
    resume_from: u64,
    file: &std::fs::File,
    counters: &[AtomicU64],
    index: usize,
    write_buffer_size: usize,
    stall_timeout: Duration,
    pause_token: &CancellationToken,
    kill_token: &CancellationToken,
    throttle: &Throttle,
) -> Result<(), ChunkError> {
    let byte_start = chunk_start + resume_from;
    // After a work steal, the victim re-enters with byte_start past its new (shrunk)
    // end. The bytes are already on disk from the previous request - return early.
    if byte_start >= chunk_end {
        return Ok(());
    }
    let range = format!("bytes={}-{}", byte_start, chunk_end - 1);

    let response = client
        .get(url)
        .header("Range", range)
        .send()
        .await
        .map_err(|_| ChunkError::Retryable { retry_after: None })?;

    if !response.status().is_success() {
        return Err(classify_status(response.status(), response.headers()));
    }

    let mut stream = response.bytes_stream();
    let mut offset = byte_start;
    let mut buffer = Vec::with_capacity(write_buffer_size);

    loop {
        tokio::select! {
            biased;
            _ = pause_token.cancelled() => {
                if !buffer.is_empty() {
                    write_at(file, &buffer, offset).map_err(classify_io_error)?;
                    counters[index].fetch_add(buffer.len() as u64, Ordering::Relaxed);
                }
                return Err(ChunkError::Paused);
            }
            _ = kill_token.cancelled() => {
                if !buffer.is_empty() {
                    write_at(file, &buffer, offset).map_err(classify_io_error)?;
                    counters[index].fetch_add(buffer.len() as u64, Ordering::Relaxed);
                }
                return Err(ChunkError::Killed);
            }
            result = tokio::time::timeout(stall_timeout, stream.next()) => {
                match result {
                    Err(_elapsed) => return Err(ChunkError::Retryable { retry_after: None }),
                    Ok(None) => break,
                    Ok(Some(Err(_))) => return Err(ChunkError::Retryable { retry_after: None }),
                    Ok(Some(Ok(bytes))) => {
                        let wait = throttle.consume(bytes.len() as u64);
                        buffer.extend_from_slice(&bytes);
                        if !wait.is_zero() {
                            tokio::select! {
                                biased;
                                _ = pause_token.cancelled() => {
                                    if !buffer.is_empty() {
                                        write_at(file, &buffer, offset).map_err(classify_io_error)?;
                                        counters[index].fetch_add(buffer.len() as u64, Ordering::Relaxed);
                                    }
                                    return Err(ChunkError::Paused);
                                }
                                _ = kill_token.cancelled() => {
                                    if !buffer.is_empty() {
                                        write_at(file, &buffer, offset).map_err(classify_io_error)?;
                                        counters[index].fetch_add(buffer.len() as u64, Ordering::Relaxed);
                                    }
                                    return Err(ChunkError::Killed);
                                }
                                _ = tokio::time::sleep(wait) => {}
                            }
                        }
                        if buffer.len() >= write_buffer_size {
                            write_at(file, &buffer, offset).map_err(classify_io_error)?;
                            offset += buffer.len() as u64;
                            counters[index].fetch_add(buffer.len() as u64, Ordering::Relaxed);
                            buffer.clear();
                        }
                    }
                }
            }
        }
    }

    if !buffer.is_empty() {
        write_at(file, &buffer, offset).map_err(classify_io_error)?;
        counters[index].fetch_add(buffer.len() as u64, Ordering::Relaxed);
    }

    Ok(())
}

// `write_at` is pwrite - writes `buf` at `offset` without moving the file cursor.
// This is what makes concurrent multi-chunk writes into the same file safe.

#[cfg(unix)]
pub fn write_at(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
pub fn write_at(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    file.seek_write(buf, offset)?;
    Ok(())
}
