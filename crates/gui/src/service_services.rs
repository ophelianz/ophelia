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

//! App-shell ownership for backend-side helpers.

use std::path::Path;

use gpui::{App, BorrowAppContext, Global};
use ophelia::service::{OpheliaClient, OpheliaError, OpheliaInstallKind, OpheliaServiceInfo};
use tokio::runtime::Handle;

use crate::app_actions;
use crate::ipc::IpcServer;
use crate::runtime::Tokio;
use crate::settings::Settings;
use crate::views::overlays::toast::Toast;

const SERVICE_OWNER_WARNING_TITLE: &str = "Ophelia is using another service";

pub struct BackendServices {
    ipc: IpcServer,
}

impl Global for BackendServices {}

pub fn start(settings: &Settings, cx: &mut App) -> Result<OpheliaClient, OpheliaError> {
    let runtime = Tokio::handle(cx);
    let client = OpheliaClient::connect_local()?;
    let ipc = IpcServer::start(settings.ipc_port, &runtime, client.clone());
    install(ipc, cx);
    Ok(client)
}

pub fn install<C: BorrowAppContext>(ipc: IpcServer, cx: &mut C) {
    cx.set_global(BackendServices { ipc });
}

pub fn restart_ipc<C: BorrowAppContext>(
    port: u16,
    runtime: &Handle,
    client: OpheliaClient,
    cx: &mut C,
) {
    cx.update_global::<BackendServices, _>(|services, _| {
        services.ipc = IpcServer::start(port, runtime, client);
    });
}

pub fn warn_if_owner_mismatch(client: OpheliaClient, cx: &mut App) {
    let expected = current_gui_install_kind();
    cx.spawn(
        async move |cx: &mut gpui::AsyncApp| match client.service_info().await {
            Ok(info) => {
                let Some(warning) = service_owner_warning_text(&info, expected) else {
                    return;
                };
                cx.update(|cx| {
                    app_actions::show_toast(
                        Toast::warning(warning.title).detail(warning.detail),
                        cx,
                    );
                });
            }
            Err(error) => {
                tracing::warn!("backend service info query failed: {error}");
            }
        },
    )
    .detach();
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServiceOwnerWarningText {
    title: &'static str,
    detail: String,
}

fn service_owner_warning_text(
    info: &OpheliaServiceInfo,
    expected: OpheliaInstallKind,
) -> Option<ServiceOwnerWarningText> {
    if expected == OpheliaInstallKind::Unknown || info.owner.install_kind == expected {
        return None;
    }

    let owner = install_kind_label(info.owner.install_kind);
    let binary = info
        .owner
        .executable
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "unavailable".to_string());

    Some(ServiceOwnerWarningText {
        title: SERVICE_OWNER_WARNING_TITLE,
        detail: format!("Service owner: {owner}. Binary: {binary}"),
    })
}

fn current_gui_install_kind() -> OpheliaInstallKind {
    install_kind_for_executable(std::env::current_exe().ok().as_deref())
}

fn install_kind_for_executable(executable: Option<&Path>) -> OpheliaInstallKind {
    let Some(executable) = executable else {
        return OpheliaInstallKind::Unknown;
    };
    let text = executable.to_string_lossy();
    if text.contains(".app/Contents/") {
        OpheliaInstallKind::AppBundle
    } else if text.starts_with("/opt/homebrew/")
        || text.starts_with("/usr/local/Homebrew/")
        || text.starts_with("/usr/local/Cellar/")
    {
        OpheliaInstallKind::HomebrewFormula
    } else if text.contains("/target/debug/") || text.contains("/target/release/") {
        OpheliaInstallKind::Development
    } else {
        OpheliaInstallKind::Other
    }
}

fn install_kind_label(kind: OpheliaInstallKind) -> &'static str {
    match kind {
        OpheliaInstallKind::AppBundle => "app bundle",
        OpheliaInstallKind::HomebrewFormula => "homebrew formula",
        OpheliaInstallKind::Development => "development build",
        OpheliaInstallKind::Other => "other",
        OpheliaInstallKind::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ophelia::service::{
        OPHELIA_MACH_SERVICE_NAME, OpheliaEndpointKind, OpheliaProfileInfo, OpheliaServiceEndpoint,
        OpheliaServiceOwner,
    };

    use super::*;

    #[test]
    fn service_owner_mismatch_produces_warning_text() {
        let info = service_info(
            OpheliaInstallKind::HomebrewFormula,
            Some("/opt/homebrew/bin/ophelia-service"),
        );

        let warning = service_owner_warning_text(&info, OpheliaInstallKind::AppBundle).unwrap();

        assert_eq!(warning.title, "Ophelia is using another service");
        assert_eq!(
            warning.detail,
            "Service owner: homebrew formula. Binary: /opt/homebrew/bin/ophelia-service"
        );
    }

    #[test]
    fn matching_service_owner_does_not_warn() {
        let info = service_info(
            OpheliaInstallKind::Development,
            Some("/repo/target/debug/ophelia-service"),
        );

        assert_eq!(
            service_owner_warning_text(&info, OpheliaInstallKind::Development),
            None
        );
    }

    #[test]
    fn unknown_gui_owner_does_not_guess_at_service_mismatch() {
        let info = service_info(
            OpheliaInstallKind::AppBundle,
            Some("/Applications/Ophelia.app/Contents/MacOS/ophelia-service"),
        );

        assert_eq!(
            service_owner_warning_text(&info, OpheliaInstallKind::Unknown),
            None
        );
    }

    #[test]
    fn gui_install_kind_uses_the_same_install_families_as_service_info() {
        assert_eq!(
            install_kind_for_executable(Some(Path::new(
                "/Applications/Ophelia.app/Contents/MacOS/ophelia"
            ))),
            OpheliaInstallKind::AppBundle
        );
        assert_eq!(
            install_kind_for_executable(Some(Path::new("/opt/homebrew/bin/ophelia"))),
            OpheliaInstallKind::HomebrewFormula
        );
        assert_eq!(
            install_kind_for_executable(Some(Path::new("/repo/target/debug/ophelia"))),
            OpheliaInstallKind::Development
        );
    }

    fn service_info(kind: OpheliaInstallKind, executable: Option<&str>) -> OpheliaServiceInfo {
        let root = PathBuf::from("/tmp/ophelia-test");
        OpheliaServiceInfo {
            service_name: OPHELIA_MACH_SERVICE_NAME.to_string(),
            version: "0.1.0".into(),
            owner: OpheliaServiceOwner {
                install_kind: kind,
                executable: executable.map(PathBuf::from),
                pid: 42,
            },
            profile: OpheliaProfileInfo {
                config_dir: root.join("config"),
                data_dir: root.join("data"),
                logs_dir: root.join("logs"),
                database_path: root.join("data/downloads.db"),
                settings_path: root.join("config/settings.json"),
                service_lock_path: root.join("data/service.lock"),
                default_download_dir: root.join("downloads"),
            },
            endpoint: OpheliaServiceEndpoint {
                kind: OpheliaEndpointKind::MachService,
                name: OPHELIA_MACH_SERVICE_NAME.to_string(),
            },
        }
    }
}
