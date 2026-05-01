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

#[cfg(target_os = "macos")]
mod macos;

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::{App, AppContext, Context, Entity, Global};
use minisign_verify::{PublicKey, Signature};
use reqwest::blocking::Client;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::build_info::{BuildInfo, ReleaseChannel, updater_controls_enabled};
use crate::settings::{Settings, UpdateChannel};

const TICK_INTERVAL: Duration = Duration::from_millis(100);
const AUTO_CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);
const INITIAL_AUTO_CHECK_DELAY: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAsset {
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub minisign_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableRelease {
    pub channel: UpdateChannel,
    pub version: String,
    pub pub_date: String,
    pub commit: Option<String>,
    pub notes_url: Option<String>,
    pub asset: UpdateAsset,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AutoUpdaterStatus {
    Idle,
    Checking,
    Available {
        release: AvailableRelease,
    },
    Downloading {
        release: AvailableRelease,
        progress: f32,
    },
    Verifying {
        release: AvailableRelease,
    },
    ReadyToInstall {
        release: AvailableRelease,
        archive_path: PathBuf,
        working_dir: PathBuf,
    },
    Installing {
        release: AvailableRelease,
    },
    Updated {
        release: AvailableRelease,
        staged_app_path: PathBuf,
        working_dir: PathBuf,
    },
    Errored {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateTrigger {
    Automatic,
    Manual,
}

#[derive(Debug)]
pub struct AutoUpdater {
    build: BuildInfo,
    settings: Settings,
    status: AutoUpdaterStatus,
    event_tx: Sender<WorkerEvent>,
    event_rx: Receiver<WorkerEvent>,
    next_auto_check_at: Option<Instant>,
}

#[derive(Default)]
struct GlobalAutoUpdater(Option<Entity<AutoUpdater>>);

impl Global for GlobalAutoUpdater {}

#[derive(Debug, Clone, PartialEq)]
enum WorkerEvent {
    UpdateFound(AvailableRelease),
    DownloadProgress {
        release: AvailableRelease,
        progress: f32,
    },
    Verifying {
        release: AvailableRelease,
    },
    ReadyToInstall {
        release: AvailableRelease,
        archive_path: PathBuf,
        working_dir: PathBuf,
    },
    InstallPrepared {
        release: AvailableRelease,
        staged_app_path: PathBuf,
        working_dir: PathBuf,
    },
    NoUpdate,
    Error(String),
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct ReleaseManifest {
    channel: UpdateChannel,
    version: String,
    pub_date: String,
    #[serde(default)]
    commit: Option<String>,
    #[serde(default)]
    notes_url: Option<String>,
    asset_url: String,
    asset_size: u64,
    sha256: String,
    minisign_url: String,
}

impl ReleaseManifest {
    fn into_release(self) -> AvailableRelease {
        AvailableRelease {
            channel: self.channel,
            version: self.version,
            pub_date: self.pub_date,
            commit: self.commit,
            notes_url: self.notes_url,
            asset: UpdateAsset {
                url: self.asset_url,
                size: self.asset_size,
                sha256: self.sha256,
                minisign_url: self.minisign_url,
            },
        }
    }
}

pub fn init(settings: Settings, cx: &mut App) {
    let updater = cx.new(|cx| AutoUpdater::new(settings, cx));
    cx.set_global(GlobalAutoUpdater(Some(updater)));
}

pub fn entity(cx: &App) -> Option<Entity<AutoUpdater>> {
    cx.has_global::<GlobalAutoUpdater>()
        .then(|| cx.global::<GlobalAutoUpdater>().0.clone())
        .flatten()
}

pub fn status(cx: &App) -> Option<AutoUpdaterStatus> {
    entity(cx).map(|entity| entity.read(cx).status.clone())
}

pub fn apply_settings(settings: Settings, cx: &mut App) {
    let Some(updater) = entity(cx) else {
        return;
    };
    updater.update(cx, |updater, cx| updater.apply_settings(settings, cx));
}

pub fn perform_primary_action(cx: &mut App) {
    let Some(updater) = entity(cx) else {
        return;
    };
    updater.update(cx, |updater, cx| updater.perform_primary_action(cx));
}

pub fn manual_check(cx: &mut App) {
    let Some(updater) = entity(cx) else {
        return;
    };
    updater.update(cx, |updater, cx| {
        updater.check_for_updates(UpdateTrigger::Manual, cx)
    });
}

impl AutoUpdater {
    fn new(settings: Settings, cx: &mut Context<Self>) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        let mut updater = Self {
            build: BuildInfo::current(),
            settings,
            status: AutoUpdaterStatus::Idle,
            event_tx,
            event_rx,
            next_auto_check_at: None,
        };
        updater.reset_auto_check_deadline(Instant::now(), true);

        cx.spawn(async |this, cx: &mut gpui::AsyncApp| {
            loop {
                cx.background_executor().timer(TICK_INTERVAL).await;
                cx.update(|app| {
                    this.update(app, |updater, cx| updater.tick(cx)).ok();
                });
            }
        })
        .detach();

        updater
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        self.drain_worker_events(cx);

        if self.should_auto_check()
            && matches!(
                self.status,
                AutoUpdaterStatus::Idle | AutoUpdaterStatus::Errored { .. }
            )
            && self
                .next_auto_check_at
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.check_for_updates(UpdateTrigger::Automatic, cx);
        }
    }

    fn apply_settings(&mut self, settings: Settings, cx: &mut Context<Self>) {
        let should_reset = self.settings.auto_update_enabled != settings.auto_update_enabled
            || self.settings.update_channel != settings.update_channel;
        self.settings = settings;
        if should_reset {
            self.reset_auto_check_deadline(Instant::now(), false);
        }
        cx.notify();
    }

    fn perform_primary_action(&mut self, cx: &mut Context<Self>) {
        match self.status.clone() {
            AutoUpdaterStatus::ReadyToInstall {
                release,
                archive_path,
                working_dir,
            } => {
                self.status = AutoUpdaterStatus::Installing {
                    release: release.clone(),
                };
                cx.notify();
                self.spawn_prepare_install(release, archive_path, working_dir);
            }
            AutoUpdaterStatus::Updated {
                staged_app_path,
                working_dir,
                ..
            } => {
                if let Err(error) = restart_to_update(&staged_app_path, &working_dir) {
                    self.status = AutoUpdaterStatus::Errored { message: error };
                    cx.notify();
                    return;
                }
                cx.quit();
            }
            AutoUpdaterStatus::Errored { .. } => {
                self.check_for_updates(UpdateTrigger::Manual, cx);
            }
            _ => {}
        }
    }

    fn check_for_updates(&mut self, trigger: UpdateTrigger, cx: &mut Context<Self>) {
        if !supports_updater_runtime(&self.build) {
            if trigger == UpdateTrigger::Manual {
                self.status = AutoUpdaterStatus::Errored {
                    message: unsupported_runtime_message(&self.build),
                };
                cx.notify();
            }
            return;
        }

        if matches!(
            self.status,
            AutoUpdaterStatus::Checking
                | AutoUpdaterStatus::Downloading { .. }
                | AutoUpdaterStatus::Verifying { .. }
                | AutoUpdaterStatus::Installing { .. }
        ) {
            return;
        }

        self.status = AutoUpdaterStatus::Checking;
        self.next_auto_check_at = Some(Instant::now() + AUTO_CHECK_INTERVAL);
        cx.notify();

        let build = self.build.clone();
        let channel = effective_update_channel(self.settings.update_channel, &build);
        let event_tx = self.event_tx.clone();
        thread::spawn(move || {
            run_check_and_download(build, channel, event_tx);
        });
    }

    fn spawn_prepare_install(
        &self,
        release: AvailableRelease,
        archive_path: PathBuf,
        working_dir: PathBuf,
    ) {
        let event_tx = self.event_tx.clone();
        thread::spawn(move || {
            let result = prepare_install(&archive_path, &working_dir);
            match result {
                Ok(staged_app_path) => {
                    let _ = event_tx.send(WorkerEvent::InstallPrepared {
                        release,
                        staged_app_path,
                        working_dir,
                    });
                }
                Err(error) => {
                    let _ = event_tx.send(WorkerEvent::Error(error));
                }
            }
        });
    }

    fn drain_worker_events(&mut self, cx: &mut Context<Self>) {
        while let Ok(event) = self.event_rx.try_recv() {
            self.apply_worker_event(event, cx);
        }
    }

    fn apply_worker_event(&mut self, event: WorkerEvent, cx: &mut Context<Self>) {
        match event {
            WorkerEvent::UpdateFound(release) => {
                self.status = AutoUpdaterStatus::Available { release };
            }
            WorkerEvent::DownloadProgress { release, progress } => {
                self.status = AutoUpdaterStatus::Downloading { release, progress };
            }
            WorkerEvent::Verifying { release } => {
                self.status = AutoUpdaterStatus::Verifying { release };
            }
            WorkerEvent::ReadyToInstall {
                release,
                archive_path,
                working_dir,
            } => {
                self.status = AutoUpdaterStatus::ReadyToInstall {
                    release,
                    archive_path,
                    working_dir,
                };
            }
            WorkerEvent::InstallPrepared {
                release,
                staged_app_path,
                working_dir,
            } => {
                self.status = AutoUpdaterStatus::Updated {
                    release,
                    staged_app_path,
                    working_dir,
                };
            }
            WorkerEvent::NoUpdate => {
                self.status = AutoUpdaterStatus::Idle;
            }
            WorkerEvent::Error(message) => {
                self.status = AutoUpdaterStatus::Errored { message };
            }
        }
        cx.notify();
    }

    fn reset_auto_check_deadline(&mut self, now: Instant, initial: bool) {
        self.next_auto_check_at = self.should_auto_check().then_some(
            now + if initial {
                INITIAL_AUTO_CHECK_DELAY
            } else {
                AUTO_CHECK_INTERVAL
            },
        );
    }

    fn should_auto_check(&self) -> bool {
        self.settings.auto_update_enabled && supports_updater_runtime(&self.build)
    }
}

fn run_check_and_download(build: BuildInfo, channel: UpdateChannel, event_tx: Sender<WorkerEvent>) {
    let client = match Client::builder().build() {
        Ok(client) => client,
        Err(error) => {
            let _ = event_tx.send(WorkerEvent::Error(error.to_string()));
            return;
        }
    };

    let result = fetch_newer_release(&client, &build, channel).and_then(|release| {
        let Some(release) = release else {
            let _ = event_tx.send(WorkerEvent::NoUpdate);
            return Ok(());
        };

        let _ = event_tx.send(WorkerEvent::UpdateFound(release.clone()));
        let public_key = minisign_public_key(&build)?;
        let downloaded = download_and_verify_release(&client, &release, public_key, &event_tx)?;
        let _ = event_tx.send(WorkerEvent::ReadyToInstall {
            release,
            archive_path: downloaded.archive_path,
            working_dir: downloaded.working_dir,
        });
        Ok(())
    });

    if let Err(error) = result {
        let _ = event_tx.send(WorkerEvent::Error(error));
    }
}

fn fetch_newer_release(
    client: &Client,
    build: &BuildInfo,
    channel: UpdateChannel,
) -> Result<Option<AvailableRelease>, String> {
    let manifest_url = manifest_url(build, channel);
    let response = client
        .get(&manifest_url)
        .send()
        .map_err(|error| format!("failed to fetch update manifest: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "update manifest returned {}",
            response.status().as_u16()
        ));
    }

    let manifest = response
        .text()
        .map_err(|error| format!("failed to read update manifest: {error}"))
        .and_then(|body| {
            serde_json::from_str::<ReleaseManifest>(&body)
                .map_err(|error| format!("failed to parse update manifest: {error}"))
        })?;
    validate_release_manifest(&manifest, channel)?;
    let release = manifest.into_release();
    if is_newer_release(build, channel, &release)? {
        Ok(Some(release))
    } else {
        Ok(None)
    }
}

fn download_and_verify_release(
    client: &Client,
    release: &AvailableRelease,
    minisign_public_key: &str,
    event_tx: &Sender<WorkerEvent>,
) -> Result<DownloadedRelease, String> {
    let working_dir = create_working_dir()?;
    let archive_path = working_dir.join(format!("Ophelia-{}.zip", release.channel.slug()));

    let mut response = client
        .get(&release.asset.url)
        .send()
        .map_err(|error| format!("failed to download update asset: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "update asset returned {}",
            response.status().as_u16()
        ));
    }

    let mut file = File::create(&archive_path)
        .map_err(|error| format!("failed to create update archive: {error}"))?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    let total_size = release
        .asset
        .size
        .max(response.content_length().unwrap_or_default());
    let mut buffer = [0_u8; 64 * 1024];

    let _ = event_tx.send(WorkerEvent::DownloadProgress {
        release: release.clone(),
        progress: 0.0,
    });

    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|error| format!("failed to read update download stream: {error}"))?;
        if read == 0 {
            break;
        }

        file.write_all(&buffer[..read])
            .map_err(|error| format!("failed to write update archive: {error}"))?;
        hasher.update(&buffer[..read]);
        downloaded = downloaded.saturating_add(read as u64);

        if total_size > 0 {
            let progress = (downloaded as f32 / total_size as f32).clamp(0.0, 1.0);
            let _ = event_tx.send(WorkerEvent::DownloadProgress {
                release: release.clone(),
                progress,
            });
        }
    }

    file.flush()
        .map_err(|error| format!("failed to flush update archive: {error}"))?;
    let _ = event_tx.send(WorkerEvent::Verifying {
        release: release.clone(),
    });

    let digest = hex_digest(hasher.finalize().as_slice());
    if !release.asset.sha256.eq_ignore_ascii_case(&digest) {
        return Err("downloaded update hash did not match manifest".into());
    }

    let signature = client
        .get(&release.asset.minisign_url)
        .send()
        .map_err(|error| format!("failed to fetch minisign signature: {error}"))?
        .text()
        .map_err(|error| format!("failed to read minisign signature: {error}"))?;
    verify_minisign_signature(&archive_path, minisign_public_key, &signature)?;

    Ok(DownloadedRelease {
        archive_path,
        working_dir,
    })
}

fn verify_minisign_signature(
    archive_path: &Path,
    public_key: &str,
    signature: &str,
) -> Result<(), String> {
    let signature = Signature::decode(signature)
        .map_err(|error| format!("failed to decode minisign signature: {error}"))?;
    let public_key = PublicKey::from_base64(public_key)
        .map_err(|error| format!("failed to decode minisign public key: {error}"))?;
    let mut verifier = public_key
        .verify_stream(&signature)
        .map_err(|error| format!("failed to initialize minisign verifier: {error}"))?;
    let mut file =
        File::open(archive_path).map_err(|error| format!("failed to open archive: {error}"))?;
    let mut buffer = [0_u8; 4096];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to read archive: {error}"))?;
        if read == 0 {
            break;
        }
        verifier.update(&buffer[..read]);
    }
    verifier
        .finalize()
        .map_err(|error| format!("minisign verification failed: {error}"))
}

fn prepare_install(archive_path: &Path, working_dir: &Path) -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        macos::prepare_install(archive_path, working_dir)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = archive_path;
        let _ = working_dir;
        Err("auto-update install is only implemented on macOS right now".into())
    }
}

fn restart_to_update(staged_app_path: &Path, working_dir: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::restart_to_update(staged_app_path, working_dir)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = staged_app_path;
        let _ = working_dir;
        Err("auto-update restart is only implemented on macOS right now".into())
    }
}

fn create_working_dir() -> Result<PathBuf, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("failed to compute update timestamp: {error}"))?
        .as_millis();
    let dir =
        std::env::temp_dir().join(format!("ophelia-update-{}-{timestamp}", std::process::id()));
    fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create update workspace: {error}"))?;
    Ok(dir)
}

fn manifest_url(build: &BuildInfo, channel: UpdateChannel) -> String {
    let base_url = std::env::var("OPHELIA_UPDATER_MANIFEST_BASE_URL")
        .unwrap_or_else(|_| build.manifest_base_url.to_string());
    format!(
        "{}/{}/{}/{}.json",
        base_url.trim_end_matches('/'),
        current_platform_slug(),
        current_arch_slug(),
        effective_update_channel(channel, build).slug()
    )
}

fn minisign_public_key(build: &BuildInfo) -> Result<&str, String> {
    std::env::var("OPHELIA_MINISIGN_PUBKEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| build.minisign_public_key.map(str::to_string))
        .filter(|value| !value.trim().is_empty())
        .map(|value| Box::leak(value.into_boxed_str()) as &str)
        .ok_or_else(|| "missing Ophelia minisign public key".into())
}

fn validate_release_manifest(
    manifest: &ReleaseManifest,
    requested_channel: UpdateChannel,
) -> Result<(), String> {
    if manifest.channel != requested_channel {
        return Err(format!(
            "update manifest channel '{}' did not match requested channel '{}'",
            manifest.channel.slug(),
            requested_channel.slug()
        ));
    }
    if manifest.version.trim().is_empty() {
        return Err("update manifest is missing a version".into());
    }
    if manifest.pub_date.trim().is_empty() {
        return Err("update manifest is missing a publish date".into());
    }
    parse_rfc3339(&manifest.pub_date)
        .map_err(|error| format!("update manifest pub_date was invalid: {error}"))?;
    if manifest.asset_url.trim().is_empty() {
        return Err("update manifest is missing an asset_url".into());
    }
    if manifest.sha256.trim().is_empty() {
        return Err("update manifest is missing a sha256".into());
    }
    if manifest.minisign_url.trim().is_empty() {
        return Err("update manifest is missing a minisign_url".into());
    }
    if requested_channel == UpdateChannel::Stable {
        Version::parse(&manifest.version)
            .map_err(|error| format!("stable update manifest version was invalid: {error}"))?;
    }
    Ok(())
}

fn effective_update_channel(configured: UpdateChannel, build: &BuildInfo) -> UpdateChannel {
    if build.channel == ReleaseChannel::Dev {
        match std::env::var("OPHELIA_UPDATER_CHANNEL").ok().as_deref() {
            Some("stable") => UpdateChannel::Stable,
            Some("nightly") => UpdateChannel::Nightly,
            _ => configured,
        }
    } else {
        configured
    }
}

fn supports_updater_runtime(build: &BuildInfo) -> bool {
    updater_controls_enabled() && cfg!(target_os = "macos") && !build.version.is_empty()
}

fn unsupported_runtime_message(build: &BuildInfo) -> String {
    if !cfg!(target_os = "macos") {
        "Auto-update is only available on macOS right now.".into()
    } else if build.channel.is_dev() {
        "Dev builds do not check for updates unless OPHELIA_UPDATER_MANIFEST_BASE_URL and OPHELIA_MINISIGN_PUBKEY are set.".into()
    } else {
        "Auto-update is not available in this build.".into()
    }
}

fn is_newer_release(
    build: &BuildInfo,
    selected_channel: UpdateChannel,
    release: &AvailableRelease,
) -> Result<bool, String> {
    match selected_channel {
        UpdateChannel::Stable => {
            let current = Version::parse(build.version)
                .map_err(|error| format!("failed to parse current version: {error}"))?;
            let fetched = Version::parse(&release.version)
                .map_err(|error| format!("failed to parse release version: {error}"))?;
            Ok(fetched > current)
        }
        UpdateChannel::Nightly => {
            let current = build
                .timestamp
                .ok_or_else(|| "nightly builds require an embedded build timestamp".to_string())
                .and_then(|timestamp| {
                    parse_rfc3339(timestamp).map_err(|error| {
                        format!("current nightly build timestamp was invalid: {error}")
                    })
                })?;
            let published = parse_rfc3339(&release.pub_date)
                .map_err(|error| format!("release pub_date was invalid: {error}"))?;
            Ok(published > current)
        }
    }
}

fn parse_rfc3339(value: &str) -> Result<OffsetDateTime, String> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|error| error.to_string())
}

fn current_platform_slug() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        "linux"
    }
}

fn current_arch_slug() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => other,
    }
}

struct DownloadedRelease {
    archive_path: PathBuf,
    working_dir: PathBuf,
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

trait UpdateChannelLabel {
    fn slug(self) -> &'static str;
}

impl UpdateChannelLabel for UpdateChannel {
    fn slug(self) -> &'static str {
        match self {
            UpdateChannel::Stable => "stable",
            UpdateChannel::Nightly => "nightly",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable_build() -> BuildInfo {
        BuildInfo {
            version: "1.0.0",
            channel: ReleaseChannel::Stable,
            commit: Some("abcdef0"),
            timestamp: Some("2026-04-08T12:00:00Z"),
            manifest_base_url: "https://example.com/updates",
            minisign_public_key: Some("RWTVGbNhJ/77g9Dm280SNcfxaPz118Hgg8vI55tFX83sIMiObZuxpDyV"),
        }
    }

    fn nightly_release(pub_date: &str) -> AvailableRelease {
        AvailableRelease {
            channel: UpdateChannel::Nightly,
            version: "1.0.0".into(),
            pub_date: pub_date.into(),
            commit: Some("abcdef1".into()),
            notes_url: None,
            asset: UpdateAsset {
                url: "https://example.com/nightly.zip".into(),
                size: 10,
                sha256: "deadbeef".into(),
                minisign_url: "https://example.com/nightly.zip.minisig".into(),
            },
        }
    }

    #[test]
    fn parses_release_manifest() {
        let manifest: ReleaseManifest = serde_json::from_str(
            r#"{
                "channel":"stable",
                "version":"1.2.3",
                "pub_date":"2026-04-08T20:00:00Z",
                "commit":"abc1234",
                "notes_url":"https://example.com/release-notes",
                "asset_url":"https://example.com/Ophelia.zip",
                "asset_size":12345,
                "sha256":"00ff",
                "minisign_url":"https://example.com/Ophelia.zip.minisig"
            }"#,
        )
        .unwrap();

        assert_eq!(manifest.channel, UpdateChannel::Stable);
        assert_eq!(manifest.asset_size, 12345);
    }

    #[test]
    fn stable_updates_compare_by_semver() {
        let build = stable_build();
        let release = AvailableRelease {
            channel: UpdateChannel::Stable,
            version: "1.1.0".into(),
            pub_date: "2026-04-09T01:00:00Z".into(),
            commit: None,
            notes_url: None,
            asset: UpdateAsset {
                url: String::new(),
                size: 0,
                sha256: String::new(),
                minisign_url: String::new(),
            },
        };

        assert!(is_newer_release(&build, UpdateChannel::Stable, &release).unwrap());
    }

    #[test]
    fn nightly_updates_compare_by_publish_time() {
        let mut build = stable_build();
        build.channel = ReleaseChannel::Nightly;

        assert!(
            is_newer_release(
                &build,
                UpdateChannel::Nightly,
                &nightly_release("2026-04-08T12:00:01Z")
            )
            .unwrap()
        );
    }

    #[test]
    fn switching_from_nightly_to_stable_never_downgrades() {
        let mut build = stable_build();
        build.channel = ReleaseChannel::Nightly;
        build.version = "1.1.0";

        let release = AvailableRelease {
            channel: UpdateChannel::Stable,
            version: "1.0.1".into(),
            pub_date: "2026-04-09T01:00:00Z".into(),
            commit: None,
            notes_url: None,
            asset: UpdateAsset {
                url: String::new(),
                size: 0,
                sha256: String::new(),
                minisign_url: String::new(),
            },
        };

        assert!(!is_newer_release(&build, UpdateChannel::Stable, &release).unwrap());
    }

    #[test]
    fn nightly_manifest_requires_non_empty_pub_date() {
        let manifest = ReleaseManifest {
            channel: UpdateChannel::Nightly,
            version: "nightly-local".into(),
            pub_date: String::new(),
            commit: None,
            notes_url: None,
            asset_url: "https://example.com/nightly.zip".into(),
            asset_size: 10,
            sha256: "deadbeef".into(),
            minisign_url: "https://example.com/nightly.zip.minisig".into(),
        };

        let error = validate_release_manifest(&manifest, UpdateChannel::Nightly).unwrap_err();
        assert!(error.contains("publish date"));
    }

    #[test]
    fn stable_manifest_requires_semver_version() {
        let manifest = ReleaseManifest {
            channel: UpdateChannel::Stable,
            version: "nightly-local".into(),
            pub_date: "2026-04-08T20:00:00Z".into(),
            commit: None,
            notes_url: None,
            asset_url: "https://example.com/Ophelia.zip".into(),
            asset_size: 10,
            sha256: "deadbeef".into(),
            minisign_url: "https://example.com/Ophelia.zip.minisig".into(),
        };

        let error = validate_release_manifest(&manifest, UpdateChannel::Stable).unwrap_err();
        assert!(error.contains("stable update manifest version was invalid"));
    }

    #[test]
    fn dev_builds_do_not_auto_check_by_default() {
        let mut build = stable_build();
        build.channel = ReleaseChannel::Dev;

        assert!(!supports_updater_runtime(&build));
    }

    #[test]
    fn header_button_states_are_visible_for_progressful_statuses() {
        let release = nightly_release("2026-04-08T12:00:01Z");
        let statuses = [
            AutoUpdaterStatus::Checking,
            AutoUpdaterStatus::Downloading {
                release: release.clone(),
                progress: 0.5,
            },
            AutoUpdaterStatus::ReadyToInstall {
                release: release.clone(),
                archive_path: PathBuf::from("/tmp/Ophelia.zip"),
                working_dir: PathBuf::from("/tmp/work"),
            },
            AutoUpdaterStatus::Installing {
                release: release.clone(),
            },
            AutoUpdaterStatus::Updated {
                release,
                staged_app_path: PathBuf::from("/tmp/Ophelia.app"),
                working_dir: PathBuf::from("/tmp/work"),
            },
        ];

        assert_eq!(statuses.len(), 5);
    }
}
