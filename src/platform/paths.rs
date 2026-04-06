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

//! Shared platform-aware application paths.
//!
//! This module is for OS-facing path policy shared across subsystems:
//! config/data directories, default downloads, and app log locations.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // target-specific variants are exercised by tests even when not built on this host OS
pub(crate) enum AppPlatform {
    MacOs,
    Linux,
    Windows,
}

pub(crate) fn current_platform() -> AppPlatform {
    #[cfg(target_os = "macos")]
    {
        AppPlatform::MacOs
    }
    #[cfg(target_os = "windows")]
    {
        AppPlatform::Windows
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        AppPlatform::Linux
    }
}

pub(crate) fn app_config_dir() -> PathBuf {
    app_config_dir_for(
        current_platform(),
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("APPDATA").map(PathBuf::from),
        std::env::var_os("USERPROFILE").map(PathBuf::from),
    )
}

pub(crate) fn app_data_dir() -> PathBuf {
    app_data_dir_for(
        current_platform(),
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        std::env::var_os("APPDATA").map(PathBuf::from),
        std::env::var_os("USERPROFILE").map(PathBuf::from),
    )
}

pub(crate) fn default_download_dir() -> PathBuf {
    default_download_dir_for(
        current_platform(),
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("USERPROFILE").map(PathBuf::from),
    )
}

#[allow(dead_code)] // used by the app binary logging path, but not by the library target
pub(crate) fn app_log_dir() -> PathBuf {
    app_log_dir_for(
        current_platform(),
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        std::env::var_os("APPDATA").map(PathBuf::from),
        std::env::var_os("USERPROFILE").map(PathBuf::from),
    )
}

pub(crate) fn legacy_app_support_dir() -> PathBuf {
    legacy_app_support_dir_for(std::env::var_os("HOME").map(PathBuf::from))
}

fn app_config_dir_for(
    platform: AppPlatform,
    home: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
    appdata: Option<PathBuf>,
    userprofile: Option<PathBuf>,
) -> PathBuf {
    match platform {
        AppPlatform::MacOs => legacy_app_support_dir_for(home),
        AppPlatform::Linux => xdg_config_home.unwrap_or_else(|| {
            home.map(|home| home.join(".config"))
                .unwrap_or_else(|| PathBuf::from("."))
        }),
        AppPlatform::Windows => appdata
            .or_else(|| userprofile.map(|profile| profile.join("AppData").join("Roaming")))
            .unwrap_or_else(|| PathBuf::from(".")),
    }
}

fn app_data_dir_for(
    platform: AppPlatform,
    home: Option<PathBuf>,
    xdg_data_home: Option<PathBuf>,
    localappdata: Option<PathBuf>,
    appdata: Option<PathBuf>,
    userprofile: Option<PathBuf>,
) -> PathBuf {
    match platform {
        AppPlatform::MacOs => legacy_app_support_dir_for(home),
        AppPlatform::Linux => xdg_data_home.unwrap_or_else(|| {
            home.map(|home| home.join(".local").join("share"))
                .unwrap_or_else(|| PathBuf::from("."))
        }),
        AppPlatform::Windows => localappdata
            .or_else(|| userprofile.map(|profile| profile.join("AppData").join("Local")))
            .or(appdata)
            .unwrap_or_else(|| PathBuf::from(".")),
    }
}

fn default_download_dir_for(
    platform: AppPlatform,
    home: Option<PathBuf>,
    userprofile: Option<PathBuf>,
) -> PathBuf {
    match platform {
        AppPlatform::MacOs | AppPlatform::Linux => home
            .map(|home| home.join("Downloads"))
            .unwrap_or_else(|| PathBuf::from(".")),
        AppPlatform::Windows => userprofile
            .or(home)
            .map(|profile| profile.join("Downloads"))
            .unwrap_or_else(|| PathBuf::from(".")),
    }
}

#[allow(dead_code)] // exercised by tests and app_log_dir, but not all targets call it directly
fn app_log_dir_for(
    platform: AppPlatform,
    home: Option<PathBuf>,
    xdg_data_home: Option<PathBuf>,
    appdata: Option<PathBuf>,
    userprofile: Option<PathBuf>,
) -> PathBuf {
    match platform {
        AppPlatform::MacOs => home
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library")
            .join("Logs")
            .join("Ophelia"),
        AppPlatform::Linux => xdg_data_home
            .or_else(|| home.map(|home| home.join(".local").join("share")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ophelia")
            .join("logs"),
        AppPlatform::Windows => appdata
            .or_else(|| userprofile.map(|profile| profile.join("AppData").join("Roaming")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Ophelia")
            .join("Logs"),
    }
}

fn legacy_app_support_dir_for(home: Option<PathBuf>) -> PathBuf {
    home.unwrap_or_else(|| PathBuf::from("."))
        .join("Library")
        .join("Application Support")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_paths_use_application_support_logs_and_downloads() {
        assert_eq!(
            app_config_dir_for(
                AppPlatform::MacOs,
                Some(PathBuf::from("/Users/alex")),
                None,
                None,
                None
            ),
            PathBuf::from("/Users/alex/Library/Application Support")
        );
        assert_eq!(
            app_data_dir_for(
                AppPlatform::MacOs,
                Some(PathBuf::from("/Users/alex")),
                None,
                None,
                None,
                None
            ),
            PathBuf::from("/Users/alex/Library/Application Support")
        );
        assert_eq!(
            app_log_dir_for(
                AppPlatform::MacOs,
                Some(PathBuf::from("/Users/alex")),
                None,
                None,
                None
            ),
            PathBuf::from("/Users/alex/Library/Logs/Ophelia")
        );
        assert_eq!(
            default_download_dir_for(AppPlatform::MacOs, Some(PathBuf::from("/Users/alex")), None),
            PathBuf::from("/Users/alex/Downloads")
        );
    }

    #[test]
    fn linux_paths_prefer_xdg_dirs_and_home_downloads() {
        assert_eq!(
            app_config_dir_for(
                AppPlatform::Linux,
                Some(PathBuf::from("/home/alex")),
                Some(PathBuf::from("/home/alex/.config-alt")),
                None,
                None,
            ),
            PathBuf::from("/home/alex/.config-alt")
        );
        assert_eq!(
            app_data_dir_for(
                AppPlatform::Linux,
                Some(PathBuf::from("/home/alex")),
                Some(PathBuf::from("/home/alex/.local/share-alt")),
                None,
                None,
                None,
            ),
            PathBuf::from("/home/alex/.local/share-alt")
        );
        assert_eq!(
            app_log_dir_for(
                AppPlatform::Linux,
                Some(PathBuf::from("/home/alex")),
                Some(PathBuf::from("/home/alex/.local/share-alt")),
                None,
                None,
            ),
            PathBuf::from("/home/alex/.local/share-alt/ophelia/logs")
        );
        assert_eq!(
            default_download_dir_for(AppPlatform::Linux, Some(PathBuf::from("/home/alex")), None),
            PathBuf::from("/home/alex/Downloads")
        );
    }

    #[test]
    fn linux_paths_fallback_to_home_conventions() {
        assert_eq!(
            app_config_dir_for(
                AppPlatform::Linux,
                Some(PathBuf::from("/home/alex")),
                None,
                None,
                None
            ),
            PathBuf::from("/home/alex/.config")
        );
        assert_eq!(
            app_data_dir_for(
                AppPlatform::Linux,
                Some(PathBuf::from("/home/alex")),
                None,
                None,
                None,
                None
            ),
            PathBuf::from("/home/alex/.local/share")
        );
        assert_eq!(
            app_log_dir_for(
                AppPlatform::Linux,
                Some(PathBuf::from("/home/alex")),
                None,
                None,
                None
            ),
            PathBuf::from("/home/alex/.local/share/ophelia/logs")
        );
    }

    #[test]
    fn windows_paths_prefer_roaming_for_config_local_for_data_and_profile_downloads() {
        assert_eq!(
            app_config_dir_for(
                AppPlatform::Windows,
                None,
                None,
                Some(PathBuf::from(r"C:\Users\Alex\AppData\Roaming")),
                Some(PathBuf::from(r"C:\Users\Alex")),
            ),
            PathBuf::from(r"C:\Users\Alex\AppData\Roaming")
        );
        assert_eq!(
            app_data_dir_for(
                AppPlatform::Windows,
                None,
                None,
                Some(PathBuf::from(r"C:\Users\Alex\AppData\Local")),
                Some(PathBuf::from(r"C:\Users\Alex\AppData\Roaming")),
                Some(PathBuf::from(r"C:\Users\Alex")),
            ),
            PathBuf::from(r"C:\Users\Alex\AppData\Local")
        );
        assert_eq!(
            app_log_dir_for(
                AppPlatform::Windows,
                None,
                None,
                Some(PathBuf::from(r"C:\Users\Alex\AppData\Roaming")),
                Some(PathBuf::from(r"C:\Users\Alex")),
            ),
            PathBuf::from(r"C:\Users\Alex\AppData\Roaming")
                .join("Ophelia")
                .join("Logs")
        );
        assert_eq!(
            default_download_dir_for(
                AppPlatform::Windows,
                None,
                Some(PathBuf::from(r"C:\Users\Alex"))
            ),
            PathBuf::from(r"C:\Users\Alex").join("Downloads")
        );
    }

    #[test]
    fn windows_paths_fallback_to_profile_conventions() {
        assert_eq!(
            app_config_dir_for(
                AppPlatform::Windows,
                None,
                None,
                None,
                Some(PathBuf::from(r"C:\Users\Alex")),
            ),
            PathBuf::from(r"C:\Users\Alex")
                .join("AppData")
                .join("Roaming")
        );
        assert_eq!(
            app_data_dir_for(
                AppPlatform::Windows,
                None,
                None,
                None,
                None,
                Some(PathBuf::from(r"C:\Users\Alex")),
            ),
            PathBuf::from(r"C:\Users\Alex")
                .join("AppData")
                .join("Local")
        );
    }
}
