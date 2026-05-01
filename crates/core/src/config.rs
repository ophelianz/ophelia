use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreConfig {
    pub max_concurrent_downloads: usize,
    pub global_speed_limit_bps: u64,
    pub http: HttpCoreConfig,
    pub destination: DestinationPolicyConfig,
}

impl CoreConfig {
    pub fn default_with_download_dir(default_download_dir: impl Into<PathBuf>) -> Self {
        Self {
            destination: DestinationPolicyConfig {
                default_download_dir: default_download_dir.into(),
                ..DestinationPolicyConfig::default()
            },
            ..Self::default()
        }
    }
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            max_concurrent_downloads: 4,
            global_speed_limit_bps: 0,
            http: HttpCoreConfig::default(),
            destination: DestinationPolicyConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpCoreConfig {
    pub max_connections_per_server: usize,
    pub max_connections_per_download: usize,
    pub ordering_mode: HttpOrderingMode,
    pub sequential_extensions: Vec<String>,
}

impl Default for HttpCoreConfig {
    fn default() -> Self {
        Self {
            max_connections_per_server: 4,
            max_connections_per_download: 8,
            ordering_mode: HttpOrderingMode::Balanced,
            sequential_extensions: default_sequential_download_extensions(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpOrderingMode {
    Balanced,
    FileSpecific,
    Sequential,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationPolicyConfig {
    pub default_download_dir: PathBuf,
    pub collision_strategy: CollisionPolicy,
    pub rules_enabled: bool,
    pub rules: Vec<DestinationRuleConfig>,
}

impl Default for DestinationPolicyConfig {
    fn default() -> Self {
        Self {
            default_download_dir: PathBuf::from("."),
            collision_strategy: CollisionPolicy::Rename,
            rules_enabled: true,
            rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollisionPolicy {
    Rename,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationRuleConfig {
    pub enabled: bool,
    pub target_dir: PathBuf,
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePaths {
    pub database_path: PathBuf,
    pub legacy_database_path: Option<PathBuf>,
    pub default_download_dir: PathBuf,
}

impl CorePaths {
    pub fn new(
        database_path: impl Into<PathBuf>,
        legacy_database_path: Option<PathBuf>,
        default_download_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            database_path: database_path.into(),
            legacy_database_path,
            default_download_dir: default_download_dir.into(),
        }
    }
}

pub fn default_sequential_download_extensions() -> Vec<String> {
    [
        ".zip", ".rar", ".7z", ".tar", ".gz", ".bz2", ".xz", ".tgz", ".mp4", ".mkv", ".mov",
        ".avi", ".webm", ".m4v", ".wmv",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}
