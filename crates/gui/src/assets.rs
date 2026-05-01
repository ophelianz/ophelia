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

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use gpui::{AssetSource, Result, SharedString};

#[derive(Clone)]
pub struct Assets {
    base: PathBuf,
}

impl Assets {
    pub fn new() -> Self {
        Self { base: asset_root() }
    }

    pub fn path(&self, path: impl AsRef<Path>) -> PathBuf {
        self.base.join(path)
    }

    pub fn read(&self, path: impl AsRef<Path>) -> std::io::Result<Vec<u8>> {
        std::fs::read(self.path(path))
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        std::fs::read(self.base.join(path))
            .map(|data| Some(Cow::Owned(data)))
            .map_err(|err| err.into())
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        std::fs::read_dir(self.base.join(path))
            .map(|entries| {
                entries
                    .filter_map(|entry| {
                        entry
                            .ok()
                            .and_then(|entry| entry.file_name().into_string().ok())
                            .map(SharedString::from)
                    })
                    .collect()
            })
            .map_err(|err| err.into())
    }
}

pub(crate) fn asset_root() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|executable| bundled_asset_root(&executable))
        .filter(|path| path.is_dir())
        .unwrap_or_else(source_asset_root)
}

fn source_asset_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
}

fn bundled_asset_root(executable: &Path) -> Option<PathBuf> {
    let macos_dir = executable.parent()?;
    if macos_dir.file_name()? != "MacOS" {
        return None;
    }

    let contents_dir = macos_dir.parent()?;
    if contents_dir.file_name()? != "Contents" {
        return None;
    }

    let app_dir = contents_dir.parent()?;
    if app_dir.extension()? != "app" {
        return None;
    }

    Some(contents_dir.join("Resources").join("assets"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_assets_inside_macos_app_bundle() {
        assert_eq!(
            bundled_asset_root(Path::new(
                "/Applications/Ophelia.app/Contents/MacOS/ophelia"
            )),
            Some(PathBuf::from(
                "/Applications/Ophelia.app/Contents/Resources/assets"
            ))
        );
    }

    #[test]
    fn ignores_non_bundle_executable_paths() {
        assert_eq!(
            bundled_asset_root(Path::new("/tmp/ophelia/target/release/ophelia")),
            None
        );
    }
}
