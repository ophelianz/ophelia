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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReleaseChannel {
    #[default]
    Dev,
    Stable,
    Nightly,
}

impl ReleaseChannel {
    pub fn is_dev(self) -> bool {
        matches!(self, Self::Dev)
    }
}

impl From<&str> for ReleaseChannel {
    fn from(value: &str) -> Self {
        match value {
            "stable" => Self::Stable,
            "nightly" => Self::Nightly,
            _ => Self::Dev,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    pub version: &'static str,
    pub channel: ReleaseChannel,
    pub commit: Option<&'static str>,
    pub timestamp: Option<&'static str>,
    pub manifest_base_url: &'static str,
    pub minisign_public_key: Option<&'static str>,
}

impl BuildInfo {
    pub fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            channel: ReleaseChannel::from(env!("OPHELIA_RELEASE_CHANNEL")),
            commit: option_env_str("OPHELIA_BUILD_COMMIT"),
            timestamp: option_env_str("OPHELIA_BUILD_TIMESTAMP"),
            manifest_base_url: env!("OPHELIA_UPDATE_MANIFEST_BASE_URL"),
            minisign_public_key: option_env_str("OPHELIA_MINISIGN_PUBKEY"),
        }
    }

    pub fn updater_available_by_default(&self) -> bool {
        cfg!(target_os = "macos") && !self.channel.is_dev()
    }
}

pub fn dev_updater_overrides_enabled() -> bool {
    std::env::var_os("OPHELIA_UPDATER_MANIFEST_BASE_URL").is_some()
        && std::env::var_os("OPHELIA_MINISIGN_PUBKEY").is_some()
}

pub fn updater_controls_enabled() -> bool {
    let build = BuildInfo::current();
    build.updater_available_by_default()
        || (cfg!(target_os = "macos") && dev_updater_overrides_enabled())
}

fn option_env_str(name: &'static str) -> Option<&'static str> {
    let value = match name {
        "OPHELIA_BUILD_COMMIT" => env!("OPHELIA_BUILD_COMMIT"),
        "OPHELIA_BUILD_TIMESTAMP" => env!("OPHELIA_BUILD_TIMESTAMP"),
        "OPHELIA_MINISIGN_PUBKEY" => env!("OPHELIA_MINISIGN_PUBKEY"),
        _ => "",
    };
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_release_channel_defaults_to_dev() {
        assert_eq!(ReleaseChannel::from("weird"), ReleaseChannel::Dev);
    }
}
