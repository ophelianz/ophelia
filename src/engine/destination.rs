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

//! Shared destination resolution and final-file commit behavior.
//!
//! All downloads resolve through the same settings-driven policy. Entry points
//! can optionally provide explicit directory and/or filename overrides, but
//! folder rules and collision handling still live here.

use std::ffi::OsStr;
use std::io::{self, Error, ErrorKind};
use std::path::{Path, PathBuf};

use crate::settings::{CollisionStrategy, DestinationRule, Settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizeStrategy {
    /// Move into a path that was pre-resolved to be unique; fail instead of
    /// clobbering if that path unexpectedly appears before commit.
    MoveNoReplace,
    /// Replace any existing destination only after the download completed
    /// successfully.
    ReplaceExisting,
}

#[derive(Debug, Clone)]
pub struct ResolvedDestination {
    pub destination: PathBuf,
    pub part_path: PathBuf,
    pub finalize_strategy: FinalizeStrategy,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DestinationOverrides {
    pub explicit_directory: Option<PathBuf>,
    pub explicit_filename: Option<String>,
}

impl DestinationOverrides {
    pub fn from_user_destination(
        typed_destination: &Path,
        auto_destination: &Path,
    ) -> io::Result<Self> {
        let typed_filename = usable_filename(typed_destination).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "destination path '{}' must include a filename",
                    typed_destination.display()
                ),
            )
        })?;
        let auto_filename = usable_filename(auto_destination).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "auto destination '{}' must include a filename",
                    auto_destination.display()
                ),
            )
        })?;

        let typed_parent = meaningful_parent(typed_destination);
        let auto_parent = meaningful_parent(auto_destination);

        Ok(Self {
            explicit_directory: match typed_parent {
                Some(parent) if auto_parent.as_deref() != Some(parent.as_path()) => Some(parent),
                _ => None,
            },
            explicit_filename: (typed_filename != auto_filename)
                .then(|| typed_filename.to_string()),
        })
    }

    pub fn for_resolved_destination(destination: &Path) -> Self {
        Self {
            explicit_directory: Some(raw_parent(destination)),
            explicit_filename: usable_filename(destination).map(ToOwned::to_owned),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DestinationPolicy {
    default_download_dir: PathBuf,
    collision_strategy: CollisionStrategy,
    rules_enabled: bool,
    rules: Vec<DestinationRule>,
    overrides: DestinationOverrides,
}

impl DestinationPolicy {
    pub fn automatic(settings: &Settings) -> Self {
        Self::with_overrides(settings, DestinationOverrides::default())
    }

    pub fn with_overrides(settings: &Settings, overrides: DestinationOverrides) -> Self {
        Self {
            default_download_dir: settings.download_dir(),
            collision_strategy: settings.collision_strategy,
            rules_enabled: settings.destination_rules_enabled,
            rules: settings.destination_rules.clone(),
            overrides,
        }
    }

    pub fn for_resolved_destination(settings: &Settings, destination: &Path) -> Self {
        Self::with_overrides(
            settings,
            DestinationOverrides::for_resolved_destination(destination),
        )
    }

    pub fn resolve(&self, url: &str, preferred_filename: Option<&str>) -> ResolvedDestination {
        let filename = self
            .overrides
            .explicit_filename
            .as_deref()
            .and_then(normalize_filename)
            .or_else(|| preferred_filename.and_then(normalize_filename))
            .unwrap_or_else(|| fallback_filename_from_url(url));
        let target_dir = self.target_dir_for(&filename);
        let destination = match self.collision_strategy {
            CollisionStrategy::Rename => {
                unique_destination(join_destination(&target_dir, &filename))
            }
            CollisionStrategy::Replace => join_destination(&target_dir, &filename),
        };
        let finalize_strategy = match self.collision_strategy {
            CollisionStrategy::Rename => FinalizeStrategy::MoveNoReplace,
            CollisionStrategy::Replace => FinalizeStrategy::ReplaceExisting,
        };
        ResolvedDestination {
            part_path: part_path_for(&destination),
            destination,
            finalize_strategy,
        }
    }

    pub fn resolve_checked(
        &self,
        url: &str,
        preferred_filename: Option<&str>,
    ) -> io::Result<ResolvedDestination> {
        let resolved = self.resolve(url, preferred_filename);
        prepare_resolved_destination(&resolved)?;
        Ok(resolved)
    }

    pub fn finalize_strategy(&self) -> FinalizeStrategy {
        match self.collision_strategy {
            CollisionStrategy::Rename => FinalizeStrategy::MoveNoReplace,
            CollisionStrategy::Replace => FinalizeStrategy::ReplaceExisting,
        }
    }

    fn target_dir_for(&self, filename: &str) -> PathBuf {
        if let Some(explicit_directory) = &self.overrides.explicit_directory {
            return explicit_directory.clone();
        }

        if self.rules_enabled {
            if let Some(dir) = self
                .rules
                .iter()
                .find(|rule| rule.enabled && rule_matches_extension(rule, filename))
                .map(|rule| rule.target_dir.clone())
            {
                return dir;
            }
        }

        self.default_download_dir.clone()
    }
}

pub fn preview_auto_destination(
    url: &str,
    suggested_filename: Option<&str>,
    settings: &Settings,
) -> PathBuf {
    DestinationPolicy::automatic(settings)
        .resolve(url, suggested_filename)
        .destination
}

pub fn fallback_filename_from_url(url: &str) -> String {
    url.rsplit('/')
        .next()
        .and_then(|segment| segment.split('?').next())
        .filter(|segment| !segment.is_empty())
        .unwrap_or("download")
        .to_string()
}

pub fn part_path_for(destination: &Path) -> PathBuf {
    let mut p = destination.to_path_buf();
    let name = p
        .file_name()
        .map(|n| format!("{}.ophelia_part", n.to_string_lossy()))
        .unwrap_or_else(|| "download.ophelia_part".into());
    p.set_file_name(name);
    p
}

pub fn prepare_resolved_destination(resolved: &ResolvedDestination) -> io::Result<()> {
    if let Some(parent) = resolved.destination.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    if resolved.part_path.exists() {
        return Err(io::Error::new(
            ErrorKind::AlreadyExists,
            format!(
                "active download staging file already exists at {}",
                resolved.part_path.display()
            ),
        ));
    }
    Ok(())
}

pub fn finalize_part_file(
    part_path: &Path,
    destination: &Path,
    strategy: FinalizeStrategy,
) -> io::Result<()> {
    match strategy {
        FinalizeStrategy::MoveNoReplace => {
            if destination.exists() {
                return Err(io::Error::new(
                    ErrorKind::AlreadyExists,
                    format!("destination already exists at {}", destination.display()),
                ));
            }
            std::fs::rename(part_path, destination)
        }
        FinalizeStrategy::ReplaceExisting => replace_existing_file(part_path, destination),
    }
}

fn normalize_filename(filename: &str) -> Option<String> {
    let trimmed = filename.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn rule_matches_extension(rule: &DestinationRule, filename: &str) -> bool {
    let Some(extension) = normalized_extension(filename) else {
        return false;
    };
    rule.extensions
        .iter()
        .filter_map(|ext| normalize_rule_extension(ext))
        .any(|ext| ext == extension)
}

fn normalized_extension(filename: &str) -> Option<String> {
    Path::new(filename)
        .extension()
        .and_then(OsStr::to_str)
        .map(|ext| format!(".{}", ext.to_ascii_lowercase()))
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

fn usable_filename(path: &Path) -> Option<&str> {
    path.file_name()
        .and_then(OsStr::to_str)
        .filter(|name| !name.is_empty())
}

fn meaningful_parent(path: &Path) -> Option<PathBuf> {
    let parent = raw_parent(path);
    (!parent.as_os_str().is_empty() && parent != Path::new(".")).then_some(parent)
}

fn raw_parent(path: &Path) -> PathBuf {
    path.parent().map(Path::to_path_buf).unwrap_or_default()
}

fn join_destination(directory: &Path, filename: &str) -> PathBuf {
    if directory.as_os_str().is_empty() {
        PathBuf::from(filename)
    } else {
        directory.join(filename)
    }
}

fn unique_destination(base: PathBuf) -> PathBuf {
    if !base.exists() {
        return base;
    }

    let parent = base.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = base
        .file_stem()
        .and_then(OsStr::to_str)
        .filter(|stem| !stem.is_empty())
        .unwrap_or("download");
    let ext = base.extension().and_then(OsStr::to_str);

    for attempt in 1.. {
        let candidate_name = match ext {
            Some(ext) if !ext.is_empty() => format!("{stem} ({attempt}).{ext}"),
            _ => format!("{stem} ({attempt})"),
        };
        let candidate = parent.join(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("the filesystem ran out of candidate names")
}

fn replace_existing_file(part_path: &Path, destination: &Path) -> io::Result<()> {
    match std::fs::rename(part_path, destination) {
        Ok(()) => Ok(()),
        Err(error) if destination.exists() => {
            let backup_path = unique_backup_path(destination);
            std::fs::rename(destination, &backup_path)?;
            match std::fs::rename(part_path, destination) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&backup_path);
                    Ok(())
                }
                Err(rename_error) => {
                    let _ = std::fs::rename(&backup_path, destination);
                    Err(rename_error)
                }
            }
        }
        Err(error) => Err(error),
    }
}

fn unique_backup_path(destination: &Path) -> PathBuf {
    let parent = destination
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let file_name = destination
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("download");

    for attempt in 0.. {
        let candidate = if attempt == 0 {
            parent.join(format!("{file_name}.ophelia_replace_backup"))
        } else {
            parent.join(format!("{file_name}.ophelia_replace_backup.{attempt}"))
        };
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("the filesystem ran out of backup names")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn settings_for(dir: &Path) -> Settings {
        Settings {
            default_download_dir: Some(dir.to_path_buf()),
            ..Settings::default()
        }
    }

    #[test]
    fn first_enabled_matching_rule_wins_case_insensitively() {
        let root = temp_dir();
        let settings = Settings {
            default_download_dir: Some(root.path().join("Downloads")),
            destination_rules_enabled: true,
            destination_rules: vec![
                DestinationRule {
                    id: "movies".into(),
                    label: "Movies".into(),
                    enabled: true,
                    target_dir: root.path().join("Movies"),
                    extensions: vec![".mkv".into(), ".mp4".into()],
                    icon_name: None,
                },
                DestinationRule {
                    id: "videos".into(),
                    label: "Videos".into(),
                    enabled: true,
                    target_dir: root.path().join("Videos"),
                    extensions: vec!["MP4".into()],
                    icon_name: None,
                },
            ],
            ..Settings::default()
        };

        let resolved = DestinationPolicy::automatic(&settings)
            .resolve("https://example.com/trailer.mp4", Some("Trailer.MP4"));

        assert_eq!(
            resolved.destination,
            root.path().join("Movies").join("Trailer.MP4")
        );
    }

    #[test]
    fn no_matching_rule_falls_back_to_default_download_dir() {
        let root = temp_dir();
        let settings = Settings {
            default_download_dir: Some(root.path().join("Downloads")),
            destination_rules_enabled: true,
            destination_rules: vec![DestinationRule {
                id: "music".into(),
                label: "Music".into(),
                enabled: true,
                target_dir: root.path().join("Music"),
                extensions: vec![".flac".into()],
                icon_name: None,
            }],
            ..Settings::default()
        };

        let resolved = DestinationPolicy::automatic(&settings)
            .resolve("https://example.com/archive.zip", None);

        assert_eq!(
            resolved.destination,
            root.path().join("Downloads").join("archive.zip")
        );
    }

    #[test]
    fn user_overrides_only_filename_and_keeps_directory_automatic() {
        let root = temp_dir();
        let settings = Settings {
            default_download_dir: Some(root.path().join("Downloads")),
            destination_rules_enabled: true,
            destination_rules: vec![
                DestinationRule {
                    id: "music".into(),
                    label: "Music".into(),
                    enabled: true,
                    target_dir: root.path().join("Music"),
                    extensions: vec![".mp3".into()],
                    icon_name: None,
                },
                DestinationRule {
                    id: "videos".into(),
                    label: "Videos".into(),
                    enabled: true,
                    target_dir: root.path().join("Videos"),
                    extensions: vec![".mkv".into()],
                    icon_name: None,
                },
            ],
            ..Settings::default()
        };
        let auto_destination =
            preview_auto_destination("https://example.com/song.mp3", None, &settings);

        let overrides = DestinationOverrides::from_user_destination(
            &root.path().join("Music").join("movie.mkv"),
            &auto_destination,
        )
        .unwrap();
        let resolved = DestinationPolicy::with_overrides(&settings, overrides)
            .resolve("https://example.com/song.mp3", None);

        assert_eq!(
            resolved.destination,
            root.path().join("Videos").join("movie.mkv")
        );
        assert_eq!(resolved.finalize_strategy, FinalizeStrategy::MoveNoReplace);
    }

    #[test]
    fn user_overrides_only_directory_and_keeps_filename_backend_driven() {
        let root = temp_dir();
        let settings = settings_for(&root.path().join("Downloads"));
        let auto_destination =
            preview_auto_destination("https://example.com/song.mp3", None, &settings);

        let overrides = DestinationOverrides::from_user_destination(
            &root.path().join("Custom").join("song.mp3"),
            &auto_destination,
        )
        .unwrap();
        let resolved = DestinationPolicy::with_overrides(&settings, overrides)
            .resolve("https://example.com/song.mp3", Some("renamed.flac"));

        assert_eq!(
            resolved.destination,
            root.path().join("Custom").join("renamed.flac")
        );
    }

    #[test]
    fn user_destination_without_filename_is_rejected() {
        let auto_destination = PathBuf::from("/tmp/Downloads/song.mp3");
        let error = DestinationOverrides::from_user_destination(Path::new(""), &auto_destination)
            .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn preview_does_not_create_target_directories() {
        let root = temp_dir();
        let downloads = root.path().join("Downloads");
        let preview = preview_auto_destination(
            "https://example.com/file.bin",
            None,
            &settings_for(&downloads),
        );

        assert_eq!(preview, downloads.join("file.bin"));
        assert!(!downloads.exists());
    }

    #[test]
    fn rename_collision_uses_numbered_suffix() {
        let root = temp_dir();
        let downloads = root.path().join("Downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(downloads.join("movie.mkv"), b"old").unwrap();

        let resolved = DestinationPolicy::automatic(&settings_for(&downloads))
            .resolve("https://example.com/movie.mkv", None);

        assert_eq!(resolved.destination, downloads.join("movie (1).mkv"));
        assert_eq!(resolved.finalize_strategy, FinalizeStrategy::MoveNoReplace);
    }

    #[test]
    fn active_part_file_duplicate_fails_preparation() {
        let root = temp_dir();
        let downloads = root.path().join("Downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        let resolved = DestinationPolicy::automatic(&settings_for(&downloads))
            .resolve("https://example.com/file.bin", None);
        std::fs::write(&resolved.part_path, b"partial").unwrap();

        let error = prepare_resolved_destination(&resolved).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
    }

    #[test]
    fn replace_strategy_preserves_existing_file_until_commit() {
        let root = temp_dir();
        let downloads = root.path().join("Downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        let destination = downloads.join("movie.mkv");
        let part_path = part_path_for(&destination);
        std::fs::write(&destination, b"old").unwrap();
        std::fs::write(&part_path, b"new").unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"old");
        finalize_part_file(&part_path, &destination, FinalizeStrategy::ReplaceExisting).unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
        assert!(!part_path.exists());
    }

    #[test]
    fn resolve_checked_creates_missing_rule_directories() {
        let root = temp_dir();
        let settings = Settings {
            default_download_dir: Some(root.path().join("Downloads")),
            destination_rules_enabled: true,
            destination_rules: vec![DestinationRule {
                id: "videos".into(),
                label: "Videos".into(),
                enabled: true,
                target_dir: root.path().join("Videos"),
                extensions: vec![".mp4".into()],
                icon_name: None,
            }],
            ..Settings::default()
        };

        let resolved = DestinationPolicy::automatic(&settings)
            .resolve_checked("https://example.com/movie.mp4", None)
            .unwrap();

        assert_eq!(
            resolved.destination,
            root.path().join("Videos").join("movie.mp4")
        );
        assert!(root.path().join("Videos").exists());
    }
}
