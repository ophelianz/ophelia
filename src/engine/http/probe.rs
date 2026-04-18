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

//! Server capability probe.
//!
//! A single GET with `Range: bytes=0-0` tells us three things:
//!   - 206 Partial Content -> server supports range requests (parallel chunks OK)
//!   - Content-Range header -> total file size
//!   - Content-Disposition header -> server-suggested filename (used instead of the URL path)

use reqwest::StatusCode;

pub struct ProbeResult {
    pub content_length: Option<u64>,
    pub accepts_ranges: bool,
    /// Filename suggested by the server via `Content-Disposition: attachment; filename="..."`.
    /// `None` if the header is absent or unparseable.
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

/// Extracts the filename from a `Content-Disposition` header value.
///
/// Handles `filename="foo.pdf"` (quoted) and `filename=foo.pdf` (unquoted).
/// Strips path separators to prevent path traversal.
fn parse_content_disposition_filename(header: &str) -> Option<String> {
    for part in header.split(';') {
        let part = part.trim();
        if let Some(val) = part.strip_prefix("filename=") {
            let name = val.trim().trim_matches('"');
            if !name.is_empty() {
                return Some(sanitize_filename(name));
            }
        }
    }
    None
}

fn sanitize_filename(name: &str) -> String {
    // Drop path separators and null bytes to prevent directory traversal.
    // Take only the last component in case the server sends a full path.
    let base = name
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(name);
    base.chars().filter(|&c| c != '\0').collect()
}
