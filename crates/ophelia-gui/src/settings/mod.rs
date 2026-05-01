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

//! Persistent user settings.
//!
//! Stored as JSON at the platform's standard application-config location:
//!
//! - macOS: `~/Library/Application Support/Ophelia/settings.json`
//! - Linux: `$XDG_CONFIG_HOME/Ophelia/settings.json` or `~/.config/Ophelia/settings.json`
//! - Windows: `%APPDATA%\\Ophelia\\settings.json`
//!
//! Missing file or parse errors silently fall back to defaults so a fresh
//! install or a corrupted file never blocks startup.
//!
//! Writes are atomic: content goes to `settings.json.tmp` first, then
//! renamed over the real file so a crash mid-write can't corrupt it.

mod destination_presets;

use std::path::PathBuf;

use ophelia::engine::{
    CollisionPolicy, CoreConfig, CorePaths, DestinationPolicyConfig, DestinationRuleConfig,
    HttpCoreConfig, HttpOrderingMode,
};
use serde::{Deserialize, Serialize};

use crate::build_info::{BuildInfo, ReleaseChannel};
use crate::platform::paths::{
    app_config_dir, app_data_dir, default_download_dir, legacy_app_support_dir,
};

pub use destination_presets::default_destination_rules;
use destination_presets::suggested_destination_rule_icon_name as suggested_icon_name;

pub const DEFAULT_IPC_PORT: u16 = 7373;
pub const DEFAULT_LANGUAGE: &str = "en";
pub const SUPPORTED_LANGUAGES: &[(&str, &str)] = &[("en", "English"), ("zh-CN", "简体中文")];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CollisionStrategy {
    #[default]
    Rename,
    Replace,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HttpDownloadOrderingMode {
    #[default]
    Balanced,
    FileSpecific,
    Sequential,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Nightly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DestinationRule {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub target_dir: PathBuf,
    pub extensions: Vec<String>,
    #[serde(default)]
    pub icon_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub max_connections_per_server: usize,
    pub max_connections_per_download: usize,
    pub max_concurrent_downloads: usize,
    #[serde(default = "default_language")]
    pub language: String,
    pub default_download_dir: Option<PathBuf>,
    /// Global bandwidth cap across all concurrent downloads. 0 = unlimited.
    pub global_speed_limit_bps: u64,
    /// Localhost port used by the browser-extension IPC server.
    pub ipc_port: u16,
    /// How automatically-routed downloads behave when the destination already exists.
    pub collision_strategy: CollisionStrategy,
    /// Master switch for extension-based destination routing.
    pub destination_rules_enabled: bool,
    /// First-match-wins routing rules for automatically chosen destinations.
    pub destination_rules: Vec<DestinationRule>,
    /// HTTP download ordering mode.
    pub http_download_ordering_mode: HttpDownloadOrderingMode,
    /// Extension list used by file-specific HTTP ordering.
    pub sequential_download_extensions: Vec<String>,
    /// Master switch for in-app popup notifications.
    pub notifications_enabled: bool,
    /// Whether production builds should automatically poll for updates.
    pub auto_update_enabled: bool,
    /// User-selected update feed channel.
    pub update_channel: UpdateChannel,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_connections_per_server: 4,
            max_connections_per_download: 8,
            max_concurrent_downloads: 3,
            language: default_language(),
            default_download_dir: None,
            global_speed_limit_bps: 0,
            ipc_port: DEFAULT_IPC_PORT,
            collision_strategy: CollisionStrategy::Rename,
            destination_rules_enabled: true,
            destination_rules: default_destination_rules(&default_download_root()),
            http_download_ordering_mode: HttpDownloadOrderingMode::FileSpecific,
            sequential_download_extensions: default_sequential_download_extensions(),
            notifications_enabled: true,
            auto_update_enabled: true,
            update_channel: default_update_channel(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let mut settings: Self = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        settings.language = canonical_language(settings.language.as_str()).to_string();
        settings
    }

    /// Persist to disk atomically & creates parent directories if needed
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)
    }

    pub fn download_dir(&self) -> PathBuf {
        if let Some(ref dir) = self.default_download_dir {
            return dir.clone();
        }
        default_download_root()
    }

    pub fn resolved_language(&self) -> &str {
        canonical_language(self.language.as_str())
    }

    pub fn core_config(&self) -> CoreConfig {
        CoreConfig {
            max_concurrent_downloads: self.max_concurrent_downloads,
            global_speed_limit_bps: self.global_speed_limit_bps,
            http: HttpCoreConfig {
                max_connections_per_server: self.max_connections_per_server,
                max_connections_per_download: self.max_connections_per_download,
                ordering_mode: self.http_download_ordering_mode.into(),
                sequential_extensions: self.sequential_download_extensions.clone(),
            },
            destination: DestinationPolicyConfig {
                default_download_dir: self.download_dir(),
                collision_strategy: self.collision_strategy.into(),
                rules_enabled: self.destination_rules_enabled,
                rules: self
                    .destination_rules
                    .iter()
                    .map(DestinationRuleConfig::from)
                    .collect(),
            },
        }
    }

    pub fn core_paths(&self) -> CorePaths {
        CorePaths::new(
            app_data_dir().join("Ophelia").join("downloads.db"),
            Some(
                legacy_app_support_dir()
                    .join("Ophelia")
                    .join("downloads.db"),
            ),
            self.download_dir(),
        )
    }

    fn path() -> PathBuf {
        app_config_dir().join("Ophelia").join("settings.json")
    }
}

impl From<CollisionStrategy> for CollisionPolicy {
    fn from(strategy: CollisionStrategy) -> Self {
        match strategy {
            CollisionStrategy::Rename => Self::Rename,
            CollisionStrategy::Replace => Self::Replace,
        }
    }
}

impl From<HttpDownloadOrderingMode> for HttpOrderingMode {
    fn from(mode: HttpDownloadOrderingMode) -> Self {
        match mode {
            HttpDownloadOrderingMode::Balanced => Self::Balanced,
            HttpDownloadOrderingMode::FileSpecific => Self::FileSpecific,
            HttpDownloadOrderingMode::Sequential => Self::Sequential,
        }
    }
}

impl From<&DestinationRule> for DestinationRuleConfig {
    fn from(rule: &DestinationRule) -> Self {
        Self {
            enabled: rule.enabled,
            target_dir: rule.target_dir.clone(),
            extensions: rule.extensions.clone(),
        }
    }
}

fn default_language() -> String {
    DEFAULT_LANGUAGE.to_string()
}

pub fn canonical_language(language: &str) -> &'static str {
    SUPPORTED_LANGUAGES
        .iter()
        .find_map(|(code, _)| (*code == language).then_some(*code))
        .unwrap_or(DEFAULT_LANGUAGE)
}

pub fn suggested_destination_rule_icon_name(label: &str, extensions: &[String]) -> &'static str {
    suggested_icon_name(label, extensions)
}

pub fn default_sequential_download_extensions() -> Vec<String> {
    [".mkv"].into_iter().map(str::to_string).collect()
}

pub fn default_update_channel() -> UpdateChannel {
    match BuildInfo::current().channel {
        ReleaseChannel::Nightly => UpdateChannel::Nightly,
        _ => UpdateChannel::Stable,
    }
}

fn default_download_root() -> PathBuf {
    default_download_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_include_default_ipc_port() {
        assert_eq!(Settings::default().ipc_port, DEFAULT_IPC_PORT);
        assert_eq!(Settings::default().language, DEFAULT_LANGUAGE);
        assert!(
            Settings::default()
                .destination_rules
                .iter()
                .any(|rule| rule.icon_name.as_deref() == Some("video"))
        );
        assert!(
            !Settings::default()
                .destination_rules
                .iter()
                .any(|rule| rule.id == "code")
        );
        assert!(!Settings::default().destination_rules.is_empty());
        assert!(Settings::default().destination_rules_enabled);
        assert_eq!(
            Settings::default().http_download_ordering_mode,
            HttpDownloadOrderingMode::FileSpecific
        );
        assert_eq!(
            Settings::default().sequential_download_extensions,
            default_sequential_download_extensions()
        );
        assert!(Settings::default().notifications_enabled);
        assert!(Settings::default().auto_update_enabled);
        assert_eq!(Settings::default().update_channel, default_update_channel());
    }

    #[test]
    fn missing_ipc_port_deserializes_to_default() {
        let settings: Settings = serde_json::from_str(
            r#"{
                "max_connections_per_server": 6,
                "max_connections_per_download": 10,
                "max_concurrent_downloads": 4,
                "default_download_dir": null,
                "global_speed_limit_bps": 0
            }"#,
        )
        .unwrap();

        assert_eq!(settings.ipc_port, DEFAULT_IPC_PORT);
        assert_eq!(settings.language, DEFAULT_LANGUAGE);
        assert_eq!(settings.max_connections_per_server, 6);
        assert_eq!(settings.collision_strategy, CollisionStrategy::Rename);
        assert!(settings.destination_rules_enabled);
        assert!(!settings.destination_rules.is_empty());
        assert_eq!(
            settings.http_download_ordering_mode,
            HttpDownloadOrderingMode::FileSpecific
        );
        assert_eq!(
            settings.sequential_download_extensions,
            default_sequential_download_extensions()
        );
        assert!(settings.notifications_enabled);
        assert!(settings.auto_update_enabled);
        assert_eq!(settings.update_channel, default_update_channel());
    }

    #[test]
    fn missing_destination_rule_fields_deserialize_to_safe_defaults() {
        let settings: Settings = serde_json::from_str(
            r#"{
                "ipc_port": 8123
            }"#,
        )
        .unwrap();

        assert_eq!(settings.ipc_port, 8123);
        assert_eq!(settings.language, DEFAULT_LANGUAGE);
        assert_eq!(settings.collision_strategy, CollisionStrategy::Rename);
        assert!(settings.destination_rules_enabled);
        assert!(!settings.destination_rules.is_empty());
        assert_eq!(
            settings.http_download_ordering_mode,
            HttpDownloadOrderingMode::FileSpecific
        );
        assert_eq!(
            settings.sequential_download_extensions,
            default_sequential_download_extensions()
        );
        assert!(settings.notifications_enabled);
    }

    #[test]
    fn unsupported_language_falls_back_to_english() {
        assert_eq!(canonical_language("fr"), DEFAULT_LANGUAGE);
        assert_eq!(canonical_language("zh-CN"), "zh-CN");
    }

    #[test]
    fn legacy_destination_rules_without_icon_name_deserialize() {
        let settings: Settings = serde_json::from_str(
            r#"{
                "destination_rules": [
                    {
                        "id": "music",
                        "label": "Music",
                        "enabled": true,
                        "target_dir": "/tmp/music",
                        "extensions": [".mp3"]
                    }
                ]
            }"#,
        )
        .unwrap();

        assert_eq!(settings.destination_rules.len(), 1);
        assert_eq!(settings.destination_rules[0].icon_name, None);
    }

    #[test]
    fn suggested_icon_name_prefers_matching_extension() {
        let icon = suggested_destination_rule_icon_name("Media", &[".mkv".into()]);

        assert_eq!(icon, "video");
    }

    #[test]
    fn explicit_http_ordering_settings_deserialize() {
        let settings: Settings = serde_json::from_str(
            r#"{
                "http_download_ordering_mode": "file_specific",
                "sequential_download_extensions": [".MKV"]
            }"#,
        )
        .unwrap();

        assert_eq!(
            settings.http_download_ordering_mode,
            HttpDownloadOrderingMode::FileSpecific
        );
        assert_eq!(settings.sequential_download_extensions, vec![".MKV"]);
    }

    #[test]
    fn settings_map_to_core_config_without_ui_only_rule_fields() {
        let settings = Settings {
            max_connections_per_server: 6,
            max_connections_per_download: 9,
            max_concurrent_downloads: 5,
            default_download_dir: Some(PathBuf::from("/tmp/downloads")),
            global_speed_limit_bps: 128,
            collision_strategy: CollisionStrategy::Replace,
            destination_rules_enabled: true,
            destination_rules: vec![DestinationRule {
                id: "video".into(),
                label: "Video".into(),
                enabled: true,
                target_dir: PathBuf::from("/tmp/video"),
                extensions: vec![".mkv".into()],
                icon_name: Some("video".into()),
            }],
            http_download_ordering_mode: HttpDownloadOrderingMode::Sequential,
            sequential_download_extensions: vec![".iso".into()],
            ..Settings::default()
        };

        let config = settings.core_config();

        assert_eq!(config.max_concurrent_downloads, 5);
        assert_eq!(config.global_speed_limit_bps, 128);
        assert_eq!(config.http.max_connections_per_server, 6);
        assert_eq!(config.http.max_connections_per_download, 9);
        assert_eq!(config.http.ordering_mode, HttpOrderingMode::Sequential);
        assert_eq!(config.http.sequential_extensions, vec![".iso"]);
        assert_eq!(
            config.destination.default_download_dir,
            PathBuf::from("/tmp/downloads")
        );
        assert_eq!(
            config.destination.collision_strategy,
            CollisionPolicy::Replace
        );
        assert_eq!(config.destination.rules.len(), 1);
        assert_eq!(
            config.destination.rules[0].target_dir,
            PathBuf::from("/tmp/video")
        );
        assert_eq!(config.destination.rules[0].extensions, vec![".mkv"]);
    }

    #[test]
    fn settings_build_core_paths_for_database_and_default_download_dir() {
        let settings = Settings {
            default_download_dir: Some(PathBuf::from("/tmp/downloads")),
            ..Settings::default()
        };

        let paths = settings.core_paths();

        assert!(paths.database_path.ends_with("Ophelia/downloads.db"));
        assert_eq!(
            paths
                .legacy_database_path
                .as_ref()
                .and_then(|path| path.file_name()),
            Some(std::ffi::OsStr::new("downloads.db"))
        );
        assert_eq!(paths.default_download_dir, PathBuf::from("/tmp/downloads"));
    }

    #[test]
    fn explicit_notifications_setting_deserializes() {
        let settings: Settings = serde_json::from_str(
            r#"{
                "notifications_enabled": false
            }"#,
        )
        .unwrap();

        assert!(!settings.notifications_enabled);
    }
}
