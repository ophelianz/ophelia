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

//! Add and restore request types for the engine
//!
//! HTTP is the only source today, but the engine can carry source-specific data

use std::io;
use std::path::{Path, PathBuf};

use crate::config::{DestinationPolicyConfig, EngineConfig};
use crate::engine::destination::{
    DestinationOverrides, DestinationPolicy, fallback_filename_from_url,
    normalize_filename_component, preview_auto_destination,
};
use crate::engine::http::HttpDownloadConfig;
use crate::engine::types::{
    PersistedDownloadSource, ProviderResumeData, SavedDownload, TransferChunkMapState,
    TransferControlSupport, TransferId,
};

/// Add request before the final path is chosen
#[derive(Debug, Clone)]
pub struct AddTransferRequest {
    pub source: AddTransferSource,
    pub suggested_filename: Option<String>,
}

impl AddTransferRequest {
    pub fn from_url(url: String) -> Self {
        Self {
            source: AddTransferSource::Url(url),
            suggested_filename: None,
        }
    }

    pub fn from_url_with_suggested_filename(
        url: String,
        suggested_filename: Option<String>,
    ) -> Self {
        Self {
            source: AddTransferSource::Url(url),
            suggested_filename: suggested_filename.and_then(normalize_suggested_filename),
        }
    }

    pub fn url(&self) -> &str {
        self.source.url()
    }

    pub fn preview_destination(&self, config: &DestinationPolicyConfig) -> PathBuf {
        preview_auto_destination(self.url(), self.suggested_filename.as_deref(), config)
    }

    pub fn into_spec(self, config: &EngineConfig) -> io::Result<DownloadSpec> {
        match self.source {
            AddTransferSource::Url(url) => {
                DownloadSpec::from_auto_request(url, self.suggested_filename, config)
            }
        }
    }

    pub fn display_filename_hint(&self) -> String {
        self.suggested_filename
            .clone()
            .unwrap_or_else(|| fallback_filename_from_url(self.url()))
    }
}

#[derive(Debug, Clone)]
pub enum AddTransferSource {
    Url(String),
}

impl AddTransferSource {
    pub fn url(&self) -> &str {
        match self {
            Self::Url(url) => url,
        }
    }
}

/// Download request used by the engine
#[derive(Debug, Clone)]
pub struct DownloadSpec {
    pub destination: PathBuf,
    destination_policy: DestinationPolicy,
    pub source: DownloadSource,
}

impl DownloadSpec {
    pub fn http(
        url: String,
        destination: PathBuf,
        destination_policy: DestinationPolicy,
        config: HttpDownloadConfig,
    ) -> Self {
        Self {
            destination,
            destination_policy,
            source: DownloadSource::Http { url, config },
        }
    }

    pub fn from_auto_request(
        url: String,
        suggested_filename: Option<String>,
        config: &EngineConfig,
    ) -> io::Result<Self> {
        let destination_policy = DestinationPolicy::automatic(&config.destination);
        let destination = destination_policy
            .resolve_checked(&url, suggested_filename.as_deref())?
            .destination;
        Ok(Self::http(
            url,
            destination,
            destination_policy,
            HttpDownloadConfig::from_engine_config(&config.http),
        ))
    }

    pub fn from_user_input(
        url: String,
        typed_destination: PathBuf,
        config: &EngineConfig,
    ) -> io::Result<Self> {
        let auto_preview = preview_auto_destination(&url, None, &config.destination);
        let overrides =
            DestinationOverrides::from_user_destination(&typed_destination, &auto_preview)?;
        let destination_policy = DestinationPolicy::with_overrides(&config.destination, overrides);
        let destination = destination_policy.resolve_checked(&url, None)?.destination;
        Ok(Self::http(
            url,
            destination,
            destination_policy,
            HttpDownloadConfig::from_engine_config(&config.http),
        ))
    }

    pub fn destination(&self) -> &Path {
        &self.destination
    }

    pub fn destination_policy(&self) -> &DestinationPolicy {
        &self.destination_policy
    }

    pub fn update_destination(&mut self, destination: PathBuf) {
        self.destination = destination;
    }

    pub fn url(&self) -> &str {
        self.source.url()
    }

    pub fn provider_kind(&self) -> &'static str {
        self.source.provider_kind()
    }

    pub fn source_label(&self) -> &str {
        self.source.source_label()
    }

    pub fn control_support(&self) -> TransferControlSupport {
        self.source.control_support()
    }

    pub fn active_chunk_map_state(&self) -> TransferChunkMapState {
        self.source.active_chunk_map_state()
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

    pub fn provider_kind(&self) -> &'static str {
        match self {
            Self::Http { .. } => "http",
        }
    }

    pub fn source_label(&self) -> &str {
        match self {
            Self::Http { url, .. } => url,
        }
    }

    pub fn control_support(&self) -> TransferControlSupport {
        match self {
            Self::Http { .. } => TransferControlSupport::all(),
        }
    }

    pub fn active_chunk_map_state(&self) -> TransferChunkMapState {
        match self {
            Self::Http { .. } => TransferChunkMapState::Loading,
        }
    }
}

/// Download restored from saved state on startup
#[derive(Debug, Clone)]
pub struct RestoredDownload {
    pub id: TransferId,
    pub spec: DownloadSpec,
    pub resume_data: Option<ProviderResumeData>,
}

impl RestoredDownload {
    pub fn http(
        id: TransferId,
        url: String,
        destination: PathBuf,
        destination_config: &DestinationPolicyConfig,
        config: HttpDownloadConfig,
        resume_data: Option<ProviderResumeData>,
    ) -> Self {
        Self {
            id,
            spec: DownloadSpec::http(
                url,
                destination.clone(),
                DestinationPolicy::for_resolved_destination(destination_config, &destination),
                config,
            ),
            resume_data,
        }
    }

    pub fn from_saved(saved: &SavedDownload, config: &EngineConfig) -> Self {
        match &saved.source {
            PersistedDownloadSource::Http { url } => Self::http(
                saved.id,
                url.clone(),
                saved.destination.clone(),
                &config.destination,
                HttpDownloadConfig::from_engine_config(&config.http),
                saved.resume_data.clone(),
            ),
        }
    }
}

fn normalize_suggested_filename(filename: String) -> Option<String> {
    normalize_filename_component(&filename)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DestinationRuleConfig, HttpOrderingMode};
    use crate::engine::types::PersistedDownloadSource;

    #[test]
    fn from_auto_request_uses_engine_config_for_http_defaults() {
        let config = EngineConfig {
            http: crate::config::HttpEngineConfig {
                max_connections_per_download: 3,
                ordering_mode: HttpOrderingMode::Sequential,
                sequential_extensions: vec![".mkv".into()],
                ..crate::config::HttpEngineConfig::default()
            },
            ..EngineConfig::default()
        };

        let spec = DownloadSpec::from_auto_request(
            "https://example.com/file.bin".to_string(),
            None,
            &config,
        )
        .unwrap();

        match spec.source {
            DownloadSource::Http { url, config } => {
                assert_eq!(url, "https://example.com/file.bin");
                assert_eq!(config.max_connections, 3);
                assert_eq!(config.ordering_mode, HttpOrderingMode::Sequential);
                assert_eq!(config.sequential_extensions, vec![".mkv"]);
            }
        }
    }

    #[test]
    fn restored_download_from_saved_rebuilds_provider_config_from_engine_config() {
        let config = EngineConfig {
            http: crate::config::HttpEngineConfig {
                max_connections_per_download: 5,
                ..crate::config::HttpEngineConfig::default()
            },
            ..EngineConfig::default()
        };
        let saved = SavedDownload {
            id: TransferId(42),
            source: PersistedDownloadSource::Http {
                url: "https://example.com/archive.zip".to_string(),
            },
            destination: PathBuf::from("/tmp/archive.zip"),
            downloaded_bytes: 0,
            total_bytes: None,
            resume_data: None,
        };

        let restored = RestoredDownload::from_saved(&saved, &config);

        assert_eq!(restored.id, TransferId(42));
        match &restored.spec.source {
            DownloadSource::Http { url, config } => {
                assert_eq!(url, "https://example.com/archive.zip");
                assert_eq!(config.max_connections, 5);
            }
        }
        assert_eq!(restored.spec.destination(), Path::new("/tmp/archive.zip"));
    }

    #[test]
    fn add_request_sanitizes_suggested_filename_components() {
        let request = AddTransferRequest::from_url_with_suggested_filename(
            "https://example.com/file.bin".to_string(),
            Some("../nested/browser-name.zip\0".to_string()),
        );
        let config = DestinationPolicyConfig {
            default_download_dir: PathBuf::from("/tmp/downloads"),
            rules_enabled: false,
            ..DestinationPolicyConfig::default()
        };

        assert_eq!(
            request.suggested_filename.as_deref(),
            Some("browser-name.zip")
        );
        assert_eq!(
            request.preview_destination(&config),
            PathBuf::from("/tmp/downloads/browser-name.zip")
        );
    }

    #[test]
    fn add_request_drops_suggested_parent_directory_filename() {
        let request = AddTransferRequest::from_url_with_suggested_filename(
            "https://example.com/file.bin".to_string(),
            Some("..".to_string()),
        );

        assert_eq!(request.suggested_filename, None);
        assert_eq!(request.display_filename_hint(), "file.bin");
    }

    #[test]
    fn from_user_input_changing_only_filename_reroutes_directory_by_extension() {
        let config = EngineConfig {
            destination: DestinationPolicyConfig {
                default_download_dir: PathBuf::from("/tmp/Downloads"),
                rules_enabled: true,
                rules: vec![
                    DestinationRuleConfig {
                        enabled: true,
                        target_dir: PathBuf::from("/tmp/Music"),
                        extensions: vec![".mp3".into()],
                    },
                    DestinationRuleConfig {
                        enabled: true,
                        target_dir: PathBuf::from("/tmp/Videos"),
                        extensions: vec![".mkv".into()],
                    },
                ],
                ..DestinationPolicyConfig::default()
            },
            ..EngineConfig::default()
        };

        let spec = DownloadSpec::from_user_input(
            "https://example.com/song.mp3".to_string(),
            PathBuf::from("/tmp/Music/movie.mkv"),
            &config,
        )
        .unwrap();

        assert_eq!(spec.destination(), Path::new("/tmp/Videos/movie.mkv"));
    }

    #[test]
    fn from_user_input_changing_only_directory_keeps_filename_automatic() {
        let config = EngineConfig {
            destination: DestinationPolicyConfig {
                default_download_dir: PathBuf::from("/tmp/Downloads"),
                ..DestinationPolicyConfig::default()
            },
            ..EngineConfig::default()
        };

        let spec = DownloadSpec::from_user_input(
            "https://example.com/song.mp3".to_string(),
            PathBuf::from("/tmp/Custom/song.mp3"),
            &config,
        )
        .unwrap();

        assert_eq!(spec.destination(), Path::new("/tmp/Custom/song.mp3"));
    }

    #[test]
    fn from_user_input_rejects_empty_destination() {
        let error = DownloadSpec::from_user_input(
            "https://example.com/song.mp3".to_string(),
            PathBuf::new(),
            &EngineConfig::default(),
        )
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
