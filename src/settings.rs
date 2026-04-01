//! Persistent user settings.
//!
//! Stored as JSON at `~/Library/Application Support/Ophelia/settings.json`.
//! Missing file or parse errors silently fall back to defaults so a fresh
//! install or a corrupted file never blocks startup.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub max_connections_per_server: usize,
    pub max_connections_per_download: usize,
    pub default_download_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_connections_per_server: 4,
            max_connections_per_download: 8,
            default_download_dir: None,
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

    /// Persist to disk. Creates parent directories if needed.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, json)
    }

    /// Resolved destination directory: user preference, then ~/Downloads, then home.
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
