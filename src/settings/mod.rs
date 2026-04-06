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

use serde::{Deserialize, Serialize};

use crate::platform::paths::{app_config_dir, default_download_dir};

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
            destination_rules_enabled: false,
            destination_rules: default_destination_rules(&default_download_root()),
        }
    }
}

impl Settings {
    /// Load from disk, returning defaults on any error.
    pub fn load() -> Self {
        let mut settings: Self = std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        settings.language = canonical_language(settings.language.as_str()).to_string();
        settings
    }

    /// Persist to disk atomically. Creates parent directories if needed.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)
    }

    /// Resolved destination directory: user preference, then ~/Downloads, then cwd.
    pub fn download_dir(&self) -> PathBuf {
        if let Some(ref dir) = self.default_download_dir {
            return dir.clone();
        }
        default_download_root()
    }

    pub fn resolved_language(&self) -> &str {
        canonical_language(self.language.as_str())
    }

    fn path() -> PathBuf {
        app_config_dir().join("Ophelia").join("settings.json")
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
        assert!(!settings.destination_rules_enabled);
        assert!(!settings.destination_rules.is_empty());
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
        assert!(!settings.destination_rules_enabled);
        assert!(!settings.destination_rules.is_empty());
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
}
