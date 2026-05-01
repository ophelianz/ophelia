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

//! Checks what the server supports
//!
//! Sends one GET with `Range: bytes=0-0`
//!   - 206 Partial Content -> server supports range requests
//!   - Content-Range on a 206, or Content-Length on a 200 -> total file size
//!   - Content-Disposition -> server-suggested filename

use reqwest::StatusCode;

use crate::engine::destination::normalize_filename_component;

pub struct ProbeResult {
    pub content_length: Option<u64>,
    pub accepts_ranges: bool,
    /// Filename from `Content-Disposition: attachment; filename="..."`
    /// `None` if the header is missing or unusable
    pub filename: Option<String>,
}

pub async fn probe(client: &reqwest::Client, url: &str) -> Result<ProbeResult, reqwest::Error> {
    let response = client.get(url).header("Range", "bytes=0-0").send().await?;

    let filename = response
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_content_disposition_filename);

    if response.status() == StatusCode::PARTIAL_CONTENT {
        let total = response
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split('/').last())
            .and_then(|v| v.parse::<u64>().ok());
        Ok(ProbeResult {
            content_length: total,
            accepts_ranges: true,
            filename,
        })
    } else {
        Ok(ProbeResult {
            content_length: response.content_length(),
            accepts_ranges: false,
            filename,
        })
    }
}

/// Extracts the filename from `Content-Disposition`
///
/// Handles `filename="foo.pdf"` and `filename=foo.pdf`
/// Strips path separators
fn parse_content_disposition_filename(header: &str) -> Option<String> {
    for part in header.split(';') {
        let part = part.trim();
        if let Some(val) = part.strip_prefix("filename=") {
            let name = val.trim().trim_matches('"');
            if !name.is_empty() {
                return normalize_filename_component(name);
            }
        }
    }
    None
}
