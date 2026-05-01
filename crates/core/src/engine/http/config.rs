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

//! Per-download settings for HTTP/HTTPS downloads

use std::path::Path;

use crate::config::{HttpEngineConfig, HttpOrderingMode, default_sequential_download_extensions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RangeOrdering {
    Balanced,
    Sequential,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HttpRangeStrategyConfig {
    pub stealing: bool,
    pub hedging: bool,
    pub health_retry: bool,
}

impl HttpRangeStrategyConfig {
    #[allow(dead_code)] // test and future advanced-mode hook
    pub const fn live_balancer() -> Self {
        Self {
            stealing: true,
            hedging: true,
            health_retry: true,
        }
    }

    pub(crate) const fn can_create_extra_work(self) -> bool {
        self.stealing || self.hedging
    }
}

#[derive(Debug, Clone)]
pub struct HttpDownloadConfig {
    /// Max parallel range requests for balanced downloads
    /// Sequential downloads force this to one in `task.rs`
    pub max_connections: usize,
    /// Min initial work units for balanced downloads
    /// Tests raise this to force parallel requests on small files
    pub min_connections: usize,
    pub write_buffer_size: usize,
    pub progress_interval_ms: u64,
    pub stall_timeout_secs: u64,
    pub max_retries_per_chunk: u32,
    /// Smallest half we allow when stealing work
    /// Tests lower this to hit the steal path on small files
    pub min_steal_bytes: u64,
    /// Per-download bandwidth cap in bytes/sec
    ///         0 = unlimited
    pub speed_limit_bps: u64,
    /// HTTP range order from settings
    pub ordering_mode: HttpOrderingMode,
    /// Extensions that use sequential range downloads in file-specific mode
    pub sequential_extensions: Vec<String>,
    /// Live strategies for balanced range downloads
    /// Sequential downloads turn these off in `task.rs`
    pub range_strategies: HttpRangeStrategyConfig,
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
            ordering_mode: HttpOrderingMode::Balanced,
            sequential_extensions: default_sequential_download_extensions(),
            range_strategies: HttpRangeStrategyConfig::live_balancer(),
        }
    }
}

impl HttpDownloadConfig {
    pub fn from_engine_config(config: &HttpEngineConfig) -> Self {
        Self {
            max_connections: config.max_connections_per_download,
            ordering_mode: config.ordering_mode,
            sequential_extensions: config.sequential_extensions.clone(),
            ..Self::default()
        }
    }

    pub(crate) fn resolved_ordering_for_destination(&self, destination: &Path) -> RangeOrdering {
        match self.ordering_mode {
            HttpOrderingMode::Balanced => RangeOrdering::Balanced,
            HttpOrderingMode::Sequential => RangeOrdering::Sequential,
            HttpOrderingMode::FileSpecific => {
                if matches_sequential_extension(destination, &self.sequential_extensions) {
                    RangeOrdering::Sequential
                } else {
                    RangeOrdering::Balanced
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
    use crate::config::DestinationPolicyConfig;
    use crate::engine::destination::{DestinationOverrides, DestinationPolicy};

    #[test]
    fn ordering_modes_resolve_expected_transfer_order() {
        let cases = [
            (
                HttpOrderingMode::Balanced,
                "/tmp/movie.mkv",
                RangeOrdering::Balanced,
            ),
            (
                HttpOrderingMode::Sequential,
                "/tmp/readme.txt",
                RangeOrdering::Sequential,
            ),
            (
                HttpOrderingMode::FileSpecific,
                "/tmp/movie.MKV",
                RangeOrdering::Sequential,
            ),
            (
                HttpOrderingMode::FileSpecific,
                "/tmp/readme.txt",
                RangeOrdering::Balanced,
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
        let http_config = HttpEngineConfig {
            ordering_mode: HttpOrderingMode::FileSpecific,
            sequential_extensions: vec![".mkv".into()],
            ..HttpEngineConfig::default()
        };
        let destination_config = DestinationPolicyConfig {
            default_download_dir: Path::new("/tmp/downloads").to_path_buf(),
            ..DestinationPolicyConfig::default()
        };
        let config = HttpDownloadConfig::from_engine_config(&http_config);
        let policy = DestinationPolicy::with_overrides(
            &destination_config,
            DestinationOverrides {
                explicit_directory: None,
                explicit_filename: Some("notes.txt".into()),
            },
        );
        let resolved = policy.resolve("https://example.com/download", Some("movie.mkv"));

        assert_eq!(
            config.resolved_ordering_for_destination(&resolved.destination),
            RangeOrdering::Balanced
        );
    }

    #[test]
    fn server_filename_refinement_controls_file_specific_mode_without_explicit_filename() {
        let http_config = HttpEngineConfig {
            ordering_mode: HttpOrderingMode::FileSpecific,
            sequential_extensions: vec![".mkv".into()],
            ..HttpEngineConfig::default()
        };
        let destination_config = DestinationPolicyConfig {
            default_download_dir: Path::new("/tmp/downloads").to_path_buf(),
            ..DestinationPolicyConfig::default()
        };
        let config = HttpDownloadConfig::from_engine_config(&http_config);
        let policy = DestinationPolicy::with_overrides(
            &destination_config,
            DestinationOverrides {
                explicit_directory: Some(Path::new("/tmp/media").to_path_buf()),
                explicit_filename: None,
            },
        );
        let resolved = policy.resolve("https://example.com/download", Some("movie.mkv"));

        assert_eq!(resolved.destination, Path::new("/tmp/media/movie.mkv"));
        assert_eq!(
            config.resolved_ordering_for_destination(&resolved.destination),
            RangeOrdering::Sequential
        );
    }

    #[test]
    fn bare_range_strategy_config_is_off() {
        assert_eq!(
            HttpRangeStrategyConfig::default(),
            HttpRangeStrategyConfig {
                stealing: false,
                hedging: false,
                health_retry: false,
            }
        );
    }

    #[test]
    fn http_downloads_default_to_live_balancer() {
        assert!(
            HttpDownloadConfig::default()
                .range_strategies
                .can_create_extra_work()
        );
        assert_eq!(
            HttpDownloadConfig::default().range_strategies,
            HttpRangeStrategyConfig::live_balancer()
        );
    }

    #[test]
    fn live_balancer_enables_all_live_strategies() {
        assert_eq!(
            HttpRangeStrategyConfig::live_balancer(),
            HttpRangeStrategyConfig {
                stealing: true,
                hedging: true,
                health_retry: true,
            }
        );
        assert!(HttpRangeStrategyConfig::live_balancer().can_create_extra_work());
    }
}
