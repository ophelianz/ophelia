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

//! Provider-neutral add/restore request shapes used by the engine surface.
//!
//! Ophelia currently supports only HTTP, but the top-level engine API should not
//! have to grow a new method or command variant for every provider.

use std::io;
use std::path::{Path, PathBuf};

use crate::engine::destination::{
    DestinationPolicy, fallback_filename_from_url, preview_auto_destination,
};
use crate::engine::http::HttpDownloadConfig;
use crate::engine::types::{
    DownloadId, PersistedDownloadSource, ProviderResumeData, SavedDownload, TransferControlSupport,
};
use crate::settings::Settings;

/// Backend-facing add request before a final destination path is chosen.
#[derive(Debug, Clone)]
pub struct AddDownloadRequest {
    pub source: AddDownloadSource,
    pub suggested_filename: Option<String>,
}

impl AddDownloadRequest {
    pub fn from_url(url: String) -> Self {
        Self {
            source: AddDownloadSource::Url(url),
            suggested_filename: None,
        }
    }

    pub fn from_url_with_suggested_filename(
        url: String,
        suggested_filename: Option<String>,
    ) -> Self {
        Self {
            source: AddDownloadSource::Url(url),
            suggested_filename: suggested_filename.and_then(normalize_suggested_filename),
        }
    }

    pub fn url(&self) -> &str {
        self.source.url()
    }

    pub fn preview_destination(&self, settings: &Settings) -> PathBuf {
        preview_auto_destination(self.url(), self.suggested_filename.as_deref(), settings)
    }

    pub fn into_spec(self, settings: &Settings) -> io::Result<DownloadSpec> {
        match self.source {
            AddDownloadSource::Url(url) => {
                DownloadSpec::from_auto_request(url, self.suggested_filename, settings)
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
pub enum AddDownloadSource {
    Url(String),
}

impl AddDownloadSource {
    pub fn url(&self) -> &str {
        match self {
            Self::Url(url) => url,
        }
    }
}

/// Provider-neutral download request used by the engine surface.
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
        settings: &Settings,
    ) -> io::Result<Self> {
        // Future protocols can branch here on scheme or a richer add request.
        let _scheme = url.split_once(':').map(|(scheme, _)| scheme);
        let destination_policy = DestinationPolicy::automatic(settings);
        let destination = destination_policy
            .resolve_checked(&url, Path::new(""), suggested_filename.as_deref())?
            .destination;
        Ok(Self::http(
            url,
            destination,
            destination_policy,
            HttpDownloadConfig::from_settings(settings),
        ))
    }

    pub fn from_manual_input(
        url: String,
        destination: PathBuf,
        settings: &Settings,
    ) -> io::Result<Self> {
        let _scheme = url.split_once(':').map(|(scheme, _)| scheme);
        let destination_policy = DestinationPolicy::manual();
        let destination = destination_policy
            .resolve_checked(&url, &destination, None)?
            .destination;
        Ok(Self::http(
            url,
            destination,
            destination_policy,
            HttpDownloadConfig::from_settings(settings),
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
}

/// Restored download state fed back into the engine on startup.
#[derive(Debug, Clone)]
pub struct RestoredDownload {
    pub id: DownloadId,
    pub spec: DownloadSpec,
    pub resume_data: Option<ProviderResumeData>,
}

impl RestoredDownload {
    pub fn http(
        id: DownloadId,
        url: String,
        destination: PathBuf,
        config: HttpDownloadConfig,
        resume_data: Option<ProviderResumeData>,
    ) -> Self {
        Self {
            id,
            spec: DownloadSpec::http(url, destination, DestinationPolicy::manual(), config),
            resume_data,
        }
    }

    pub fn from_saved(saved: &SavedDownload, settings: &Settings) -> Self {
        match &saved.source {
            PersistedDownloadSource::Http { url } => Self::http(
                saved.id,
                url.clone(),
                saved.destination.clone(),
                HttpDownloadConfig::from_settings(settings),
                saved.resume_data.clone(),
            ),
        }
    }
}

fn normalize_suggested_filename(filename: String) -> Option<String> {
    let trimmed = filename.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::PersistedDownloadSource;

    #[test]
    fn from_auto_request_uses_settings_for_http_defaults() {
        let settings = Settings {
            max_connections_per_download: 3,
            ..Settings::default()
        };

        let spec = DownloadSpec::from_auto_request(
            "https://example.com/file.bin".to_string(),
            None,
            &settings,
        )
        .unwrap();

        match spec.source {
            DownloadSource::Http { url, config } => {
                assert_eq!(url, "https://example.com/file.bin");
                assert_eq!(config.max_connections, 3);
            }
        }
    }

    #[test]
    fn download_spec_exposes_provider_metadata_and_control_support() {
        let spec = DownloadSpec::http(
            "https://example.com/file.bin".to_string(),
            PathBuf::from("/tmp/file.bin"),
            DestinationPolicy::manual(),
            HttpDownloadConfig::default(),
        );

        assert_eq!(spec.provider_kind(), "http");
        assert_eq!(spec.source_label(), "https://example.com/file.bin");
        let controls = spec.control_support();
        assert!(controls.can_pause);
        assert!(controls.can_resume);
        assert!(controls.can_cancel);
        assert!(controls.can_restore);
    }

    #[test]
    fn restored_download_from_saved_rebuilds_provider_config_from_settings() {
        let settings = Settings {
            max_connections_per_download: 5,
            ..Settings::default()
        };
        let saved = SavedDownload {
            id: DownloadId(42),
            source: PersistedDownloadSource::Http {
                url: "https://example.com/archive.zip".to_string(),
            },
            destination: PathBuf::from("/tmp/archive.zip"),
            downloaded_bytes: 0,
            total_bytes: None,
            resume_data: None,
        };

        let restored = RestoredDownload::from_saved(&saved, &settings);

        assert_eq!(restored.id, DownloadId(42));
        match restored.spec.source {
            DownloadSource::Http { url, config } => {
                assert_eq!(url, "https://example.com/archive.zip");
                assert_eq!(config.max_connections, 5);
            }
        }
    }

    #[test]
    fn persisted_source_exposes_provider_metadata_and_control_support() {
        let source = PersistedDownloadSource::Http {
            url: "https://example.com/archive.zip".to_string(),
        };

        assert_eq!(source.kind(), "http");
        assert_eq!(source.display_label(), "https://example.com/archive.zip");
        let controls = source.control_support();
        assert!(controls.can_pause);
        assert!(controls.can_resume);
        assert!(controls.can_cancel);
        assert!(controls.can_restore);
    }

    #[test]
    fn add_request_preview_destination_prefers_suggested_filename() {
        let request = AddDownloadRequest::from_url_with_suggested_filename(
            "https://example.com/file.bin".to_string(),
            Some("browser-name.zip".to_string()),
        );
        let settings = Settings {
            default_download_dir: Some(PathBuf::from("/tmp/downloads")),
            ..Settings::default()
        };

        let destination = request.preview_destination(&settings);
        assert_eq!(
            destination,
            PathBuf::from("/tmp/downloads/browser-name.zip")
        );
    }

    #[test]
    fn add_request_preview_destination_falls_back_to_url_filename() {
        let request =
            AddDownloadRequest::from_url("https://example.com/path/file.bin?token=abc".to_string());
        let settings = Settings {
            default_download_dir: Some(PathBuf::from("/tmp/downloads")),
            ..Settings::default()
        };

        let destination = request.preview_destination(&settings);
        assert_eq!(destination, PathBuf::from("/tmp/downloads/file.bin"));
    }

    #[test]
    fn add_request_display_filename_hint_prefers_suggested_filename() {
        let request = AddDownloadRequest::from_url_with_suggested_filename(
            "https://example.com/file.bin".to_string(),
            Some("browser-name.zip".to_string()),
        );

        assert_eq!(request.display_filename_hint(), "browser-name.zip");
    }
}
