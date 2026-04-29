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

//! Per-download configuration for HTTP/HTTPS downloads.
//! Fields here are intentionally HTTP-specific: connection count, stall detection,
//! and retry behavior are concepts that don't apply to all protocols.

use std::path::Path;

use crate::settings::{HttpDownloadOrderingMode, Settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolvedHttpDownloadOrdering {
    Balanced,
    Sequential,
}

#[derive(Debug, Clone)]
pub struct HttpDownloadConfig {
    /// Hard ceiling on parallel connections per download. The actual count is
    /// derived from the sqrt heuristic and clamped to [min_connections, max_connections].
    pub max_connections: usize,
    /// Floor for the sqrt heuristic. Default 1 (heuristic drives everything).
    /// Set higher in tests to force parallel chunks on small files without needing
    /// a large file download.
    pub min_connections: usize,
    pub write_buffer_size: usize,
    pub progress_interval_ms: u64,
    pub stall_timeout_secs: u64,
    pub max_retries_per_chunk: u32,
    /// Minimum bytes required in each half of a potential steal.
    /// A steal requires >= 2× this value remaining. Lowered in tests to exercise
    /// the code path on small files.
    pub min_steal_bytes: u64,
    /// Per-download bandwidth cap in bytes/sec. 0 = unlimited.
    pub speed_limit_bps: u64,
    /// High-level HTTP scheduling mode selected from settings.
    pub ordering_mode: HttpDownloadOrderingMode,
    /// Extension list used when the scheduling mode is file-specific.
    pub sequential_extensions: Vec<String>,
}

impl Default for HttpDownloadConfig {
    fn default() -> Self {
        Self {
            max_connections: 8,
            min_connections: 1,
            write_buffer_size: 64 * 1024,
            progress_interval_ms: 100,
            stall_timeout_secs: 10,
            max_retries_per_chunk: 3,
            min_steal_bytes: 4 * 1024 * 1024,
            speed_limit_bps: 0,
            ordering_mode: HttpDownloadOrderingMode::Balanced,
            sequential_extensions: crate::settings::default_sequential_download_extensions(),
        }
    }
}

impl HttpDownloadConfig {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            max_connections: settings.max_connections_per_download,
            ordering_mode: settings.http_download_ordering_mode,
            sequential_extensions: settings.sequential_download_extensions.clone(),
            ..Self::default()
        }
    }

    pub(crate) fn resolved_ordering_for_destination(
        &self,
        destination: &Path,
    ) -> ResolvedHttpDownloadOrdering {
        match self.ordering_mode {
            HttpDownloadOrderingMode::Balanced => ResolvedHttpDownloadOrdering::Balanced,
            HttpDownloadOrderingMode::Sequential => ResolvedHttpDownloadOrdering::Sequential,
            HttpDownloadOrderingMode::FileSpecific => {
                if matches_sequential_extension(destination, &self.sequential_extensions) {
                    ResolvedHttpDownloadOrdering::Sequential
                } else {
                    ResolvedHttpDownloadOrdering::Balanced
                }
            }
        }
    }
}

fn matches_sequential_extension(destination: &Path, configured_extensions: &[String]) -> bool {
    let Some(extension) = destination
        .extension()
        .and_then(|ext| ext.to_str())
        .and_then(normalize_extension)
    else {
        return false;
    };

    configured_extensions
        .iter()
        .filter_map(|ext| normalize_extension(ext))
        .any(|candidate| candidate == extension)
}

fn normalize_extension(extension: &str) -> Option<String> {
    let trimmed = extension.trim();
    if trimmed.is_empty() {
        return None;
    }
    let prefixed = if trimmed.starts_with('.') {
        trimmed.to_string()
    } else {
        format!(".{trimmed}")
    };
    Some(prefixed.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::destination::{DestinationOverrides, DestinationPolicy};

    #[test]
    fn ordering_modes_resolve_expected_transfer_order() {
        let cases = [
            (
                HttpDownloadOrderingMode::Balanced,
                "/tmp/movie.mkv",
                ResolvedHttpDownloadOrdering::Balanced,
            ),
            (
                HttpDownloadOrderingMode::Sequential,
                "/tmp/readme.txt",
                ResolvedHttpDownloadOrdering::Sequential,
            ),
            (
                HttpDownloadOrderingMode::FileSpecific,
                "/tmp/movie.MKV",
                ResolvedHttpDownloadOrdering::Sequential,
            ),
            (
                HttpDownloadOrderingMode::FileSpecific,
                "/tmp/readme.txt",
                ResolvedHttpDownloadOrdering::Balanced,
            ),
        ];

        for (ordering_mode, path, expected) in cases {
            let config = HttpDownloadConfig {
                ordering_mode,
                sequential_extensions: vec!["mkv".into()],
                ..HttpDownloadConfig::default()
            };
            assert_eq!(
                config.resolved_ordering_for_destination(Path::new(path)),
                expected
            );
        }
    }

    #[test]
    fn explicit_filename_override_beats_server_filename_for_file_specific_mode() {
        let settings = Settings {
            http_download_ordering_mode: HttpDownloadOrderingMode::FileSpecific,
            sequential_download_extensions: vec![".mkv".into()],
            default_download_dir: Some(Path::new("/tmp/downloads").to_path_buf()),
            ..Settings::default()
        };
        let config = HttpDownloadConfig::from_settings(&settings);
        let policy = DestinationPolicy::with_overrides(
            &settings,
            DestinationOverrides {
                explicit_directory: None,
                explicit_filename: Some("notes.txt".into()),
            },
        );
        let resolved = policy.resolve("https://example.com/download", Some("movie.mkv"));

        assert_eq!(
            config.resolved_ordering_for_destination(&resolved.destination),
            ResolvedHttpDownloadOrdering::Balanced
        );
    }

    #[test]
    fn server_filename_refinement_controls_file_specific_mode_without_explicit_filename() {
        let settings = Settings {
            http_download_ordering_mode: HttpDownloadOrderingMode::FileSpecific,
            sequential_download_extensions: vec![".mkv".into()],
            default_download_dir: Some(Path::new("/tmp/downloads").to_path_buf()),
            ..Settings::default()
        };
        let config = HttpDownloadConfig::from_settings(&settings);
        let policy = DestinationPolicy::with_overrides(
            &settings,
            DestinationOverrides {
                explicit_directory: Some(Path::new("/tmp/media").to_path_buf()),
                explicit_filename: None,
            },
        );
        let resolved = policy.resolve("https://example.com/download", Some("movie.mkv"));

        assert_eq!(resolved.destination, Path::new("/tmp/media/movie.mkv"));
        assert_eq!(
            config.resolved_ordering_for_destination(&resolved.destination),
            ResolvedHttpDownloadOrdering::Sequential
        );
    }
}
