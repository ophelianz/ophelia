use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineConfig {
    pub max_concurrent_downloads: usize,
    pub global_speed_limit_bps: u64,
    pub http: HttpEngineConfig,
    pub destination: DestinationPolicyConfig,
}

impl EngineConfig {
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

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_concurrent_downloads: 4,
            global_speed_limit_bps: 0,
            http: HttpEngineConfig::default(),
            destination: DestinationPolicyConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpEngineConfig {
    pub max_connections_per_server: usize,
    pub max_connections_per_download: usize,
    pub ordering_mode: HttpOrderingMode,
    pub sequential_extensions: Vec<String>,
}

impl Default for HttpEngineConfig {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServiceSettings {
    pub max_connections_per_server: usize,
    pub max_connections_per_download: usize,
    #[serde(alias = "max_concurrent_downloads")]
    pub max_concurrent_transfers: usize,
    pub default_download_dir: Option<PathBuf>,
    pub global_speed_limit_bps: u64,
    #[serde(alias = "collision_strategy")]
    pub collision_policy: CollisionPolicy,
    pub destination_rules_enabled: bool,
    pub destination_rules: Vec<ServiceDestinationRule>,
    #[serde(alias = "http_download_ordering_mode")]
    pub http_ordering_mode: HttpOrderingMode,
    pub sequential_download_extensions: Vec<String>,
}

impl ServiceSettings {
    pub fn load(paths: &ProfilePaths) -> Self {
        std::fs::read_to_string(&paths.settings_path)
            .ok()
            .and_then(|body| serde_json::from_str(&body).ok())
            .unwrap_or_else(|| Self::default_for_paths(paths))
    }

    pub fn save(&self, paths: &ProfilePaths) -> io::Result<()> {
        if let Some(parent) = paths.settings_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let tmp = paths.settings_path.with_extension("json.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(&tmp, &paths.settings_path)
    }

    pub fn default_for_paths(paths: &ProfilePaths) -> Self {
        Self {
            default_download_dir: None,
            destination_rules: default_destination_rules(&paths.default_download_dir),
            ..Self::default()
        }
    }

    pub fn download_dir(&self, paths: &ProfilePaths) -> PathBuf {
        self.default_download_dir
            .clone()
            .unwrap_or_else(|| paths.default_download_dir.clone())
    }

    pub fn to_engine_config(&self, paths: &ProfilePaths) -> EngineConfig {
        EngineConfig {
            max_concurrent_downloads: self.max_concurrent_transfers,
            global_speed_limit_bps: self.global_speed_limit_bps,
            http: HttpEngineConfig {
                max_connections_per_server: self.max_connections_per_server,
                max_connections_per_download: self.max_connections_per_download,
                ordering_mode: self.http_ordering_mode,
                sequential_extensions: self.sequential_download_extensions.clone(),
            },
            destination: DestinationPolicyConfig {
                default_download_dir: self.download_dir(paths),
                collision_strategy: self.collision_policy,
                rules_enabled: self.destination_rules_enabled,
                rules: self
                    .destination_rules
                    .iter()
                    .map(DestinationRuleConfig::from)
                    .collect(),
            },
        }
    }
}

impl Default for ServiceSettings {
    fn default() -> Self {
        let download_dir = default_download_dir();
        Self {
            max_connections_per_server: 4,
            max_connections_per_download: 8,
            max_concurrent_transfers: 3,
            default_download_dir: None,
            global_speed_limit_bps: 0,
            collision_policy: CollisionPolicy::Rename,
            destination_rules_enabled: true,
            destination_rules: default_destination_rules(&download_dir),
            http_ordering_mode: HttpOrderingMode::FileSpecific,
            sequential_download_extensions: vec![".mkv".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceDestinationRule {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub target_dir: PathBuf,
    pub extensions: Vec<String>,
    #[serde(default)]
    pub icon_name: Option<String>,
}

impl From<&ServiceDestinationRule> for DestinationRuleConfig {
    fn from(rule: &ServiceDestinationRule) -> Self {
        Self {
            enabled: rule.enabled,
            target_dir: rule.target_dir.clone(),
            extensions: rule.extensions.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfilePaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub database_path: PathBuf,
    pub settings_path: PathBuf,
    pub service_lock_path: PathBuf,
    pub default_download_dir: PathBuf,
}

impl ProfilePaths {
    pub fn new(
        database_path: impl Into<PathBuf>,
        default_download_dir: impl Into<PathBuf>,
    ) -> Self {
        let database_path = database_path.into();
        let data_dir = database_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            config_dir: data_dir.clone(),
            data_dir: data_dir.clone(),
            logs_dir: data_dir.join("logs"),
            database_path,
            settings_path: data_dir.join("settings.json"),
            service_lock_path: data_dir.join("ophelia.service.lock"),
            default_download_dir: default_download_dir.into(),
        }
    }

    pub fn default_profile() -> Self {
        let config_dir = app_config_dir().join("Ophelia");
        let data_dir = app_data_dir().join("Ophelia");
        Self {
            config_dir: config_dir.clone(),
            data_dir: data_dir.clone(),
            logs_dir: app_log_dir(),
            database_path: data_dir.join("downloads.db"),
            settings_path: config_dir.join("settings.json"),
            service_lock_path: data_dir.join("ophelia.service.lock"),
            default_download_dir: default_download_dir(),
        }
    }

    pub fn from_dirs(
        config_dir: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        default_download_dir: impl Into<PathBuf>,
    ) -> Self {
        let config_dir = config_dir.into();
        let data_dir = data_dir.into();
        Self {
            config_dir: config_dir.clone(),
            data_dir: data_dir.clone(),
            logs_dir: data_dir.join("logs"),
            database_path: data_dir.join("downloads.db"),
            settings_path: config_dir.join("settings.json"),
            service_lock_path: data_dir.join("ophelia.service.lock"),
            default_download_dir: default_download_dir.into(),
        }
    }
}

struct DestinationRulePreset {
    id: &'static str,
    label: &'static str,
    folder_name: &'static str,
    icon_name: &'static str,
    extensions: &'static [&'static str],
}

const DESTINATION_RULE_PRESETS: &[DestinationRulePreset] = &[
    DestinationRulePreset {
        id: "archive",
        label: "Archives",
        folder_name: "Archives",
        icon_name: "archive",
        extensions: &[".zip", ".rar", ".7z", ".tar", ".gz", ".bz2", ".xz", ".tgz"],
    },
    DestinationRulePreset {
        id: "audio",
        label: "Music",
        folder_name: "Music",
        icon_name: "audio",
        extensions: &[".mp3", ".flac", ".wav", ".aac", ".ogg", ".m4a", ".opus"],
    },
    DestinationRulePreset {
        id: "book",
        label: "Books",
        folder_name: "Books",
        icon_name: "book",
        extensions: &[".epub", ".mobi", ".azw3", ".fb2"],
    },
    DestinationRulePreset {
        id: "code",
        label: "Code",
        folder_name: "Code",
        icon_name: "code",
        extensions: &[
            ".rs", ".js", ".ts", ".tsx", ".jsx", ".py", ".go", ".java", ".c", ".cpp", ".h", ".hpp",
            ".json", ".yaml", ".yml", ".toml", ".sh", ".css",
        ],
    },
    DestinationRulePreset {
        id: "document",
        label: "Documents",
        folder_name: "Documents",
        icon_name: "document",
        extensions: &[".pdf", ".doc", ".docx", ".txt", ".rtf", ".md"],
    },
    DestinationRulePreset {
        id: "executable",
        label: "Executables",
        folder_name: "Executables",
        icon_name: "executable",
        extensions: &[
            ".exe",
            ".msi",
            ".dmg",
            ".pkg",
            ".appimage",
            ".deb",
            ".rpm",
            ".apk",
        ],
    },
    DestinationRulePreset {
        id: "font",
        label: "Fonts",
        folder_name: "Fonts",
        icon_name: "font",
        extensions: &[".ttf", ".otf", ".woff", ".woff2"],
    },
    DestinationRulePreset {
        id: "image",
        label: "Images",
        folder_name: "Images",
        icon_name: "image",
        extensions: &[
            ".png", ".jpg", ".jpeg", ".gif", ".webp", ".heic", ".avif", ".bmp", ".tiff",
        ],
    },
    DestinationRulePreset {
        id: "key",
        label: "Keys",
        folder_name: "Keys",
        icon_name: "key",
        extensions: &[".pem", ".pub", ".p12", ".pfx", ".crt", ".cer", ".asc"],
    },
    DestinationRulePreset {
        id: "mail",
        label: "Mail",
        folder_name: "Mail",
        icon_name: "mail",
        extensions: &[".eml", ".mbox", ".msg"],
    },
    DestinationRulePreset {
        id: "presentation",
        label: "Presentations",
        folder_name: "Presentations",
        icon_name: "presentation",
        extensions: &[".ppt", ".pptx", ".odp"],
    },
    DestinationRulePreset {
        id: "spreadsheet",
        label: "Spreadsheets",
        folder_name: "Spreadsheets",
        icon_name: "spreadsheet",
        extensions: &[".csv", ".tsv", ".xls", ".xlsx", ".ods"],
    },
    DestinationRulePreset {
        id: "vector",
        label: "Vectors",
        folder_name: "Vectors",
        icon_name: "vector",
        extensions: &[".svg", ".ai", ".eps"],
    },
    DestinationRulePreset {
        id: "video",
        label: "Videos",
        folder_name: "Videos",
        icon_name: "video",
        extensions: &[".mp4", ".mkv", ".mov", ".avi", ".webm", ".m4v", ".wmv"],
    },
    DestinationRulePreset {
        id: "web",
        label: "Web",
        folder_name: "Web",
        icon_name: "web",
        extensions: &[".html", ".htm", ".mhtml", ".webloc", ".url"],
    },
];

const DEFAULT_DESTINATION_RULE_PRESET_IDS: &[&str] = &[
    "archive",
    "audio",
    "document",
    "executable",
    "image",
    "video",
];

pub fn default_destination_rules(base_dir: &Path) -> Vec<ServiceDestinationRule> {
    DESTINATION_RULE_PRESETS
        .iter()
        .filter(|preset| DEFAULT_DESTINATION_RULE_PRESET_IDS.contains(&preset.id))
        .map(|preset| ServiceDestinationRule {
            id: preset.id.to_string(),
            label: preset.label.to_string(),
            enabled: true,
            target_dir: base_dir.join(preset.folder_name),
            extensions: preset
                .extensions
                .iter()
                .map(|ext| ext.to_string())
                .collect(),
            icon_name: Some(preset.icon_name.to_string()),
        })
        .collect()
}

pub fn suggested_destination_rule_icon_name(label: &str, extensions: &[String]) -> &'static str {
    for extension in extensions
        .iter()
        .filter_map(|ext| normalize_rule_extension(ext))
    {
        if let Some(preset) = DESTINATION_RULE_PRESETS.iter().find(|preset| {
            preset
                .extensions
                .iter()
                .filter_map(|candidate| normalize_rule_extension(candidate))
                .any(|candidate| candidate == extension)
        }) {
            return preset.icon_name;
        }
    }

    let normalized_label = label.trim().to_ascii_lowercase();
    if normalized_label.is_empty() {
        return "default";
    }

    DESTINATION_RULE_PRESETS
        .iter()
        .find(|preset| {
            normalized_label.contains(&preset.id.to_ascii_lowercase())
                || normalized_label.contains(&preset.label.to_ascii_lowercase())
                || normalized_label
                    .contains(&preset.label.trim_end_matches('s').to_ascii_lowercase())
        })
        .map(|preset| preset.icon_name)
        .unwrap_or("default")
}

fn normalize_rule_extension(extension: &str) -> Option<String> {
    let trimmed = extension.trim();
    if trimmed.is_empty() {
        None
    } else if trimmed.starts_with('.') {
        Some(trimmed.to_ascii_lowercase())
    } else {
        Some(format!(".{}", trimmed.to_ascii_lowercase()))
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

fn app_config_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        legacy_app_support_dir()
    }

    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .map(PathBuf::from)
                    .map(|profile| profile.join("AppData").join("Roaming"))
            })
            .unwrap_or_else(|| PathBuf::from("."))
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".config"))
            })
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

fn app_data_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        legacy_app_support_dir()
    }

    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("APPDATA"))
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .map(PathBuf::from)
                    .map(|profile| profile.join("AppData").join("Local"))
            })
            .unwrap_or_else(|| PathBuf::from("."))
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local").join("share"))
            })
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

fn app_log_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library")
            .join("Logs")
            .join("Ophelia")
    }

    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .map(PathBuf::from)
                    .map(|profile| profile.join("AppData").join("Roaming"))
            })
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Ophelia")
            .join("Logs")
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local").join("share"))
            })
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ophelia")
            .join("logs")
    }
}

fn legacy_app_support_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library")
        .join("Application Support")
}

fn default_download_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .map(|profile| profile.join("Downloads"))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Downloads"))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}
