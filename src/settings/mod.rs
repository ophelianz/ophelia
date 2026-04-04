//! Persistent user settings.
//!
//! Stored as JSON at `~/Library/Application Support/Ophelia/settings.json`.
//! Missing file or parse errors silently fall back to defaults so a fresh
//! install or a corrupted file never blocks startup.
//!
//! Writes are atomic: content goes to `settings.json.tmp` first, then
//! renamed over the real file so a crash mid-write can't corrupt it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const DEFAULT_IPC_PORT: u16 = 7373;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub max_connections_per_server: usize,
    pub max_connections_per_download: usize,
    pub max_concurrent_downloads: usize,
    pub default_download_dir: Option<PathBuf>,
    /// Global bandwidth cap across all concurrent downloads. 0 = unlimited.
    pub global_speed_limit_bps: u64,
    /// Localhost port used by the browser-extension IPC server.
    pub ipc_port: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_connections_per_server: 4,
            max_connections_per_download: 8,
            max_concurrent_downloads: 3,
            default_download_dir: None,
            global_speed_limit_bps: 0,
            ipc_port: DEFAULT_IPC_PORT,
        }
    }
}

impl Settings {
    /// Load from disk, returning defaults on any error.
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
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
        std::env::var("HOME")
            .map(|h| PathBuf::from(h).join("Downloads"))
            .unwrap_or_else(|_| PathBuf::from("."))
    }

    fn path() -> PathBuf {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("Library")
            .join("Application Support")
            .join("Ophelia")
            .join("settings.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_include_default_ipc_port() {
        assert_eq!(Settings::default().ipc_port, DEFAULT_IPC_PORT);
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
        assert_eq!(settings.max_connections_per_server, 6);
    }
}
