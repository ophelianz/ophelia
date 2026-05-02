use super::*;
use objc2_foundation::{NSError, NSString};
use objc2_service_management::{
    SMAppService, SMAppServiceStatus, kSMErrorAlreadyRegistered, kSMErrorJobNotFound,
    kSMErrorLaunchDeniedByUser,
};
use std::process::Command;
use std::thread;
use std::time::Instant;

const SERVICE_BINARY_ENV: &str = "OPHELIA_SERVICE_BINARY";
const SERVICE_START_MODE_ENV: &str = "OPHELIA_SERVICE_START_MODE";
const DEV_LAUNCHCTL_START_MODE: &str = "dev-launchctl";
const LAUNCH_AGENT_PLIST_NAME: &str = "nz.ophelia.service.plist";
const BUNDLED_SERVICE_PROGRAM: &str = "Contents/MacOS/ophelia-service";

pub(super) fn connect_or_start_local(
    options: LocalServiceOptions,
) -> Result<LocalServiceConnection, OpheliaError> {
    let startup = StartupContext::resolve(options.service_binary)?;
    let manager = MacosServiceManager::new(ProfilePaths::default_profile(), startup);

    if let Some(connection) =
        manager.try_existing_service(options.repair_policy, options.startup_timeout)?
    {
        return Ok(connection);
    }

    manager.start_service(manager.should_refresh_registration_before_start())?;
    manager.wait_until_reachable(options.startup_timeout)
}

struct MacosServiceManager {
    paths: ProfilePaths,
    startup: StartupContext,
}

impl MacosServiceManager {
    fn new(paths: ProfilePaths, startup: StartupContext) -> Self {
        Self { paths, startup }
    }

    fn try_existing_service(
        &self,
        repair_policy: LocalServiceRepairPolicy,
        startup_timeout: Duration,
    ) -> Result<Option<LocalServiceConnection>, OpheliaError> {
        let Ok((client, info)) = connect_client_and_info() else {
            return Ok(None);
        };
        let identity = service_identity(&info, &self.startup);
        if matches!(identity, ServiceIdentity::Matches) {
            tracing::debug!(
                service = info.service_name,
                version = info.version,
                helper = ?info.helper.executable,
                "using matching OpheliaService"
            );
            return Ok(Some(LocalServiceConnection {
                client,
                warning: None,
            }));
        }

        let can_replace_development_mismatch = self.startup.can_replace_development_mismatch(&info);
        let can_repair_service = self.startup.can_repair()
            && repair_policy == LocalServiceRepairPolicy::RepairIfSafe
            && (can_replace_development_mismatch || !service_has_running_transfers(&client));
        if can_repair_service {
            tracing::info!(
                identity = ?identity,
                expected_helper = ?self.startup.service_binary(),
                running_helper = ?info.helper.executable,
                force_development_refresh = can_replace_development_mismatch,
                "refreshing OpheliaService registration"
            );
            self.start_service(true)?;
            return Ok(Some(self.wait_until_reachable(startup_timeout)?));
        }

        tracing::warn!(
            identity = ?identity,
            expected_helper = ?self.startup.service_binary(),
            running_helper = ?info.helper.executable,
            running_version = info.version,
            "keeping active OpheliaService from another install"
        );
        Ok(Some(LocalServiceConnection {
            client,
            warning: Some(active_mismatch_warning(
                &info,
                self.startup.service_binary(),
            )),
        }))
    }

    fn start_service(&self, force_reregister: bool) -> Result<(), OpheliaError> {
        match &self.startup {
            StartupContext::AppBundle {
                app_bundle,
                service_binary,
            } => register_bundled_launch_agent(app_bundle, service_binary, force_reregister),
            StartupContext::DevLaunchctl { service_binary } => {
                self.start_dev_launchctl_service(service_binary)
            }
            StartupContext::NonBundled { .. } => Err(OpheliaError::Transport {
                message: format!(
                    "OpheliaService auto-start requires Ophelia.app on macOS 13+ or a Cargo target binary. For custom local builds, set {SERVICE_START_MODE_ENV}={DEV_LAUNCHCTL_START_MODE} and {SERVICE_BINARY_ENV}"
                ),
            }),
        }
    }

    fn should_refresh_registration_before_start(&self) -> bool {
        self.startup.refreshes_registration_on_cold_start()
    }

    fn start_dev_launchctl_service(&self, service_binary: &Path) -> Result<(), OpheliaError> {
        ensure_service_binary_exists(service_binary)?;
        tracing::info!(
            helper = ?service_binary,
            "starting OpheliaService through dev launchctl fallback"
        );
        let plist = render_dev_launch_agent_plist(service_binary, &self.paths.logs_dir);
        let plist_path = dev_launch_agent_path()?;
        fs::create_dir_all(&self.paths.logs_dir)?;
        write_dev_launch_agent_plist(&plist_path, &plist)?;
        bootout_dev_service();
        bootstrap_dev_service(&plist_path)?;
        kickstart_dev_service()
    }

    fn wait_until_reachable(
        &self,
        timeout: Duration,
    ) -> Result<LocalServiceConnection, OpheliaError> {
        let deadline = Instant::now() + timeout;
        let mut last_error = OpheliaError::Closed;

        while Instant::now() < deadline {
            match connect_client_and_info() {
                Ok((client, info))
                    if service_base_matches_expected(&info, self.startup.service_binary()) =>
                {
                    tracing::debug!(
                        service = info.service_name,
                        version = info.version,
                        helper = ?info.helper.executable,
                        "OpheliaService became reachable"
                    );
                    return Ok(LocalServiceConnection {
                        client,
                        warning: None,
                    });
                }
                Ok((_, info)) => {
                    last_error = OpheliaError::Transport {
                        message: format!(
                            "started service did not match this install: {:?}",
                            active_mismatch_warning(&info, self.startup.service_binary())
                        ),
                    };
                }
                Err(error) => last_error = error,
            }
            thread::sleep(Duration::from_millis(100));
        }

        Err(last_error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StartupContext {
    AppBundle {
        app_bundle: PathBuf,
        service_binary: PathBuf,
    },
    DevLaunchctl {
        service_binary: PathBuf,
    },
    NonBundled {
        service_binary: PathBuf,
    },
}

impl StartupContext {
    fn resolve(explicit_service_binary: Option<PathBuf>) -> Result<Self, OpheliaError> {
        Self::resolve_from(
            explicit_service_binary,
            std::env::var_os(SERVICE_BINARY_ENV).map(PathBuf::from),
            std::env::current_exe().ok(),
            std::env::var(SERVICE_START_MODE_ENV).ok(),
        )
    }

    fn resolve_from(
        explicit_service_binary: Option<PathBuf>,
        env_service_binary: Option<PathBuf>,
        current_exe: Option<PathBuf>,
        start_mode: Option<String>,
    ) -> Result<Self, OpheliaError> {
        let current_exe = current_exe.ok_or_else(|| OpheliaError::Transport {
            message: "could not resolve current executable path".into(),
        })?;
        let configured_service_binary = explicit_service_binary.or(env_service_binary);

        if matches!(start_mode.as_deref(), Some(DEV_LAUNCHCTL_START_MODE)) {
            let service_binary = configured_service_binary
                .unwrap_or_else(|| default_service_binary_for_exe(&current_exe));
            return Ok(Self::DevLaunchctl { service_binary });
        }
        if let Some(mode) = start_mode {
            return Err(OpheliaError::Transport {
                message: format!(
                    "unknown {SERVICE_START_MODE_ENV} value {mode:?}; expected {DEV_LAUNCHCTL_START_MODE:?}"
                ),
            });
        }
        if let Some(app_bundle) = app_bundle_for_executable(&current_exe) {
            return Ok(Self::AppBundle {
                service_binary: app_bundle.join(BUNDLED_SERVICE_PROGRAM),
                app_bundle,
            });
        }

        if infer_install_kind(Some(&current_exe)) == OpheliaInstallKind::Development {
            let service_binary = configured_service_binary.unwrap_or(current_exe);
            return Ok(Self::DevLaunchctl { service_binary });
        }

        let service_binary = configured_service_binary
            .unwrap_or_else(|| default_service_binary_for_exe(&current_exe));
        Ok(Self::NonBundled { service_binary })
    }

    fn service_binary(&self) -> &Path {
        match self {
            Self::AppBundle { service_binary, .. }
            | Self::DevLaunchctl { service_binary }
            | Self::NonBundled { service_binary } => service_binary,
        }
    }

    fn can_repair(&self) -> bool {
        matches!(self, Self::AppBundle { .. } | Self::DevLaunchctl { .. })
    }

    fn refreshes_registration_on_cold_start(&self) -> bool {
        matches!(self, Self::AppBundle { .. })
    }

    fn can_replace_development_mismatch(&self, info: &OpheliaServiceInfo) -> bool {
        matches!(self, Self::DevLaunchctl { .. })
            && info.helper.install_kind == OpheliaInstallKind::Development
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceRegistrationStatus {
    NotRegistered,
    Enabled,
    RequiresApproval,
    NotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceRegistrationAction {
    Noop,
    Register,
    Reregister,
    RequiresApproval,
}

impl From<SMAppServiceStatus> for ServiceRegistrationStatus {
    fn from(status: SMAppServiceStatus) -> Self {
        match status {
            SMAppServiceStatus::Enabled => Self::Enabled,
            SMAppServiceStatus::RequiresApproval => Self::RequiresApproval,
            SMAppServiceStatus::NotFound => Self::NotFound,
            _ => Self::NotRegistered,
        }
    }
}

fn register_bundled_launch_agent(
    app_bundle: &Path,
    service_binary: &Path,
    force_reregister: bool,
) -> Result<(), OpheliaError> {
    validate_bundled_service_files(app_bundle, service_binary)?;
    let plist_name = NSString::from_str(LAUNCH_AGENT_PLIST_NAME);
    let service = unsafe { SMAppService::agentServiceWithPlistName(&plist_name) };

    match registration_action(service_registration_status(&service), force_reregister) {
        ServiceRegistrationAction::Reregister => {
            tracing::info!(
                helper = ?service_binary,
                plist = LAUNCH_AGENT_PLIST_NAME,
                "re-registering bundled OpheliaService"
            );
            unregister_smapp_service(&service)?;
            register_smapp_service(&service)
        }
        ServiceRegistrationAction::Register => {
            tracing::info!(
                helper = ?service_binary,
                plist = LAUNCH_AGENT_PLIST_NAME,
                "registering bundled OpheliaService"
            );
            register_smapp_service(&service)
        }
        ServiceRegistrationAction::Noop => {
            tracing::debug!(
                helper = ?service_binary,
                "bundled OpheliaService is already registered"
            );
            Ok(())
        }
        ServiceRegistrationAction::RequiresApproval => {
            tracing::warn!("OpheliaService requires approval in System Settings");
            Err(approval_required_error())
        }
    }
}

fn registration_action(
    status: ServiceRegistrationStatus,
    force_reregister: bool,
) -> ServiceRegistrationAction {
    match status {
        ServiceRegistrationStatus::Enabled if force_reregister => {
            ServiceRegistrationAction::Reregister
        }
        ServiceRegistrationStatus::Enabled => ServiceRegistrationAction::Noop,
        ServiceRegistrationStatus::NotRegistered => ServiceRegistrationAction::Register,
        ServiceRegistrationStatus::RequiresApproval => ServiceRegistrationAction::RequiresApproval,
        ServiceRegistrationStatus::NotFound => ServiceRegistrationAction::Register,
    }
}

fn service_registration_status(service: &SMAppService) -> ServiceRegistrationStatus {
    unsafe { service.status() }.into()
}

fn register_smapp_service(service: &SMAppService) -> Result<(), OpheliaError> {
    match unsafe { service.registerAndReturnError() } {
        Ok(()) => Ok(()),
        Err(error) if service_management_error_code(&error) == kSMErrorAlreadyRegistered => Ok(()),
        Err(error) if service_management_error_code(&error) == kSMErrorLaunchDeniedByUser => {
            Err(approval_required_error())
        }
        Err(error) => Err(service_management_error("register", &error)),
    }
}

fn unregister_smapp_service(service: &SMAppService) -> Result<(), OpheliaError> {
    match unsafe { service.unregisterAndReturnError() } {
        Ok(()) => Ok(()),
        Err(error) if service_management_error_code(&error) == kSMErrorJobNotFound => Ok(()),
        Err(error) if service_management_error_code(&error) == kSMErrorLaunchDeniedByUser => {
            Err(approval_required_error())
        }
        Err(error) => Err(service_management_error("unregister", &error)),
    }
}

fn validate_bundled_service_files(
    app_bundle: &Path,
    service_binary: &Path,
) -> Result<(), OpheliaError> {
    let plist_path = app_bundle
        .join("Contents")
        .join("Library")
        .join("LaunchAgents")
        .join(LAUNCH_AGENT_PLIST_NAME);
    if !plist_path.is_file() {
        return Err(OpheliaError::Transport {
            message: format!(
                "OpheliaService LaunchAgent plist was not found at {}",
                plist_path.display()
            ),
        });
    }
    ensure_service_binary_exists(service_binary)
}

fn approval_required_error() -> OpheliaError {
    OpheliaError::ServiceApprovalRequired {
        service_name: OPHELIA_MACH_SERVICE_NAME.to_string(),
    }
}

fn service_management_error(action: &str, error: &NSError) -> OpheliaError {
    OpheliaError::Transport {
        message: format!(
            "ServiceManagement failed to {action} OpheliaService: code {}",
            service_management_error_code(error)
        ),
    }
}

fn service_management_error_code(error: &NSError) -> u32 {
    error.code() as u32
}

fn ensure_service_binary_exists(path: &Path) -> Result<(), OpheliaError> {
    if path.is_file() {
        return Ok(());
    }
    Err(OpheliaError::Transport {
        message: format!("service binary was not found at {}", path.display()),
    })
}

fn connect_client_and_info() -> Result<(OpheliaClient, OpheliaServiceInfo), OpheliaError> {
    let client = OpheliaClient::connect_local()?;
    let info = futures::executor::block_on(client.service_info())?;
    Ok((client, info))
}

fn service_has_running_transfers(client: &OpheliaClient) -> bool {
    let Ok(snapshot) = futures::executor::block_on(client.snapshot()) else {
        return true;
    };
    snapshot.transfers.summaries().iter().any(|transfer| {
        matches!(
            transfer.status,
            TransferStatus::Pending | TransferStatus::Downloading
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceIdentity {
    Matches,
    NeedsRefresh,
    Mismatch,
}

fn service_identity(info: &OpheliaServiceInfo, startup: &StartupContext) -> ServiceIdentity {
    if !service_base_matches_expected(info, startup.service_binary()) {
        return ServiceIdentity::Mismatch;
    }
    if !startup.refreshes_registration_on_cold_start() {
        return ServiceIdentity::Matches;
    }
    let expected_hash = executable_sha256(startup.service_binary()).ok();
    helper_hash_identity(
        info.helper.executable_sha256.as_deref(),
        expected_hash.as_deref(),
    )
}

fn helper_hash_identity(
    running_hash: Option<&str>,
    expected_hash: Option<&str>,
) -> ServiceIdentity {
    match (running_hash, expected_hash) {
        (Some(running), Some(expected)) if running == expected => ServiceIdentity::Matches,
        _ => ServiceIdentity::NeedsRefresh,
    }
}

fn service_base_matches_expected(info: &OpheliaServiceInfo, expected_binary: &Path) -> bool {
    info.service_name == OPHELIA_MACH_SERVICE_NAME
        && info.version == env!("CARGO_PKG_VERSION")
        && info.helper.install_kind == infer_install_kind(Some(expected_binary))
        && info
            .helper
            .executable
            .as_deref()
            .is_some_and(|actual| paths_match(actual, expected_binary))
}

fn active_mismatch_warning(
    info: &OpheliaServiceInfo,
    expected_binary: &Path,
) -> LocalServiceWarning {
    LocalServiceWarning::ActiveServiceMismatch {
        expected_binary: expected_binary.to_path_buf(),
        actual_binary: info.helper.executable.clone(),
        expected_version: env!("CARGO_PKG_VERSION").to_string(),
        actual_version: info.version.clone(),
    }
}

fn default_service_binary_for_exe(executable: &Path) -> PathBuf {
    executable
        .parent()
        .map(|parent| parent.join("ophelia-service"))
        .unwrap_or_else(|| PathBuf::from("ophelia-service"))
}

fn app_bundle_for_executable(executable: &Path) -> Option<PathBuf> {
    executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .map(Path::to_path_buf)
}

fn dev_launch_agent_path() -> Result<PathBuf, OpheliaError> {
    let home = std::env::var_os("HOME").ok_or_else(|| OpheliaError::Transport {
        message: "HOME is not set".into(),
    })?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{OPHELIA_MACH_SERVICE_NAME}.plist")))
}

fn write_dev_launch_agent_plist(path: &Path, plist: &str) -> Result<(), OpheliaError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("plist.tmp");
    fs::write(&tmp, plist)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn render_dev_launch_agent_plist(service_binary: &Path, logs_dir: &Path) -> String {
    let binary = xml_escape(&service_binary.to_string_lossy());
    let logs = xml_escape(&logs_dir.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{OPHELIA_MACH_SERVICE_NAME}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>{OPHELIA_RUN_SERVICE_ENV}</key>
        <string>1</string>
    </dict>
    <key>MachServices</key>
    <dict>
        <key>{OPHELIA_MACH_SERVICE_NAME}</key>
        <true/>
    </dict>
    <key>KeepAlive</key>
    <false/>
    <key>ProcessType</key>
    <string>Interactive</string>
    <key>StandardOutPath</key>
    <string>{logs}/ophelia-service.out.log</string>
    <key>StandardErrorPath</key>
    <string>{logs}/ophelia-service.err.log</string>
</dict>
</plist>
"#
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn bootout_dev_service() {
    let _ = run_dev_launchctl(["bootout".into(), dev_launchd_service_target()]);
}

fn bootstrap_dev_service(plist_path: &Path) -> Result<(), OpheliaError> {
    run_dev_launchctl([
        "bootstrap".into(),
        dev_launchd_gui_domain(),
        plist_path.display().to_string(),
    ])
}

fn kickstart_dev_service() -> Result<(), OpheliaError> {
    run_dev_launchctl([
        "kickstart".into(),
        "-k".into(),
        dev_launchd_service_target(),
    ])
}

fn run_dev_launchctl<const N: usize>(args: [String; N]) -> Result<(), OpheliaError> {
    let output = Command::new("launchctl")
        .args(args.iter().map(String::as_str))
        .output()
        .map_err(|error| OpheliaError::Transport {
            message: format!("failed to run launchctl: {error}"),
        })?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(OpheliaError::Transport {
        message: format!("launchctl failed: {}", stderr.trim()),
    })
}

fn dev_launchd_service_target() -> String {
    format!("{}/{}", dev_launchd_gui_domain(), OPHELIA_MACH_SERVICE_NAME)
}

fn dev_launchd_gui_domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

fn paths_match(actual: &Path, expected: &Path) -> bool {
    let actual = actual
        .canonicalize()
        .unwrap_or_else(|_| actual.to_path_buf());
    let expected = expected
        .canonicalize()
        .unwrap_or_else(|_| expected.to_path_buf());
    actual == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_context_uses_app_bundle_for_production() {
        let context = StartupContext::resolve_from(
            None,
            None,
            Some(PathBuf::from(
                "/Applications/Ophelia.app/Contents/MacOS/ophelia",
            )),
            None,
        )
        .unwrap();

        assert_eq!(
            context,
            StartupContext::AppBundle {
                app_bundle: PathBuf::from("/Applications/Ophelia.app"),
                service_binary: PathBuf::from(
                    "/Applications/Ophelia.app/Contents/MacOS/ophelia-service"
                ),
            }
        );
    }

    #[test]
    fn startup_context_uses_dev_launchctl_for_cargo_run_binary() {
        let context = StartupContext::resolve_from(
            None,
            None,
            Some(PathBuf::from("/repo/target/debug/ophelia")),
            None,
        )
        .unwrap();

        assert_eq!(
            context,
            StartupContext::DevLaunchctl {
                service_binary: PathBuf::from("/repo/target/debug/ophelia"),
            }
        );
    }

    #[test]
    fn startup_context_allows_dev_launchctl_only_with_explicit_mode() {
        let context = StartupContext::resolve_from(
            None,
            Some(PathBuf::from("/repo/target/debug/ophelia-service")),
            Some(PathBuf::from("/repo/target/debug/ophelia")),
            Some(DEV_LAUNCHCTL_START_MODE.into()),
        )
        .unwrap();

        assert_eq!(
            context,
            StartupContext::DevLaunchctl {
                service_binary: PathBuf::from("/repo/target/debug/ophelia-service"),
            }
        );
    }

    #[test]
    fn startup_context_keeps_other_non_bundled_binaries_unsupported_without_mode() {
        let context = StartupContext::resolve_from(
            None,
            Some(PathBuf::from("/usr/local/bin/ophelia-service")),
            Some(PathBuf::from("/usr/local/bin/ophelia")),
            None,
        )
        .unwrap();

        assert_eq!(
            context,
            StartupContext::NonBundled {
                service_binary: PathBuf::from("/usr/local/bin/ophelia-service"),
            }
        );
    }

    #[test]
    fn dev_launchctl_can_replace_development_service_mismatch() {
        let startup = StartupContext::DevLaunchctl {
            service_binary: PathBuf::from("/repo/target/debug/ophelia"),
        };
        let info = service_info_for_binary(
            Path::new("/repo/target/debug/ophelia-service"),
            Some("hash".into()),
        );

        assert!(startup.can_replace_development_mismatch(&info));
    }

    #[test]
    fn app_bundle_refreshes_registration_on_cold_start() {
        let context = StartupContext::resolve_from(
            None,
            None,
            Some(PathBuf::from(
                "/Applications/Ophelia.app/Contents/MacOS/ophelia",
            )),
            None,
        )
        .unwrap();

        assert!(context.refreshes_registration_on_cold_start());
    }

    #[test]
    fn dev_launchctl_does_not_reregister_on_cold_start() {
        let context = StartupContext::resolve_from(
            None,
            Some(PathBuf::from("/repo/target/debug/ophelia-service")),
            Some(PathBuf::from("/repo/target/debug/ophelia")),
            Some(DEV_LAUNCHCTL_START_MODE.into()),
        )
        .unwrap();

        assert!(!context.refreshes_registration_on_cold_start());
    }

    #[test]
    fn startup_context_rejects_unknown_start_mode() {
        let error = StartupContext::resolve_from(
            None,
            None,
            Some(PathBuf::from("/repo/target/debug/ophelia")),
            Some("launchctl".into()),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unknown OPHELIA_SERVICE_START_MODE")
        );
    }

    #[test]
    fn service_registration_status_maps_smapp_constants() {
        assert_eq!(
            ServiceRegistrationStatus::from(SMAppServiceStatus::Enabled),
            ServiceRegistrationStatus::Enabled
        );
        assert_eq!(
            ServiceRegistrationStatus::from(SMAppServiceStatus::RequiresApproval),
            ServiceRegistrationStatus::RequiresApproval
        );
        assert_eq!(
            ServiceRegistrationStatus::from(SMAppServiceStatus::NotFound),
            ServiceRegistrationStatus::NotFound
        );
        assert_eq!(
            ServiceRegistrationStatus::from(SMAppServiceStatus::NotRegistered),
            ServiceRegistrationStatus::NotRegistered
        );
    }

    #[test]
    fn enabled_registration_is_refreshed_only_when_requested() {
        assert_eq!(
            registration_action(ServiceRegistrationStatus::Enabled, false),
            ServiceRegistrationAction::Noop
        );
        assert_eq!(
            registration_action(ServiceRegistrationStatus::Enabled, true),
            ServiceRegistrationAction::Reregister
        );
    }

    #[test]
    fn registration_action_handles_status_branches() {
        assert_eq!(
            registration_action(ServiceRegistrationStatus::NotRegistered, false),
            ServiceRegistrationAction::Register
        );
        assert_eq!(
            registration_action(ServiceRegistrationStatus::RequiresApproval, true),
            ServiceRegistrationAction::RequiresApproval
        );
        assert_eq!(
            registration_action(ServiceRegistrationStatus::NotFound, true),
            ServiceRegistrationAction::Register
        );
    }

    #[test]
    fn approval_required_error_is_typed() {
        assert!(matches!(
            approval_required_error(),
            OpheliaError::ServiceApprovalRequired { service_name }
                if service_name == OPHELIA_MACH_SERVICE_NAME
        ));
    }

    #[test]
    fn sha256_reader_hashes_known_bytes() {
        let mut bytes = &b"hello"[..];

        assert_eq!(
            sha256_reader(&mut bytes).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn helper_hash_match_keeps_registered_service() {
        let dir = tempfile::tempdir().unwrap();
        let service_binary = dir
            .path()
            .join("Ophelia.app/Contents/MacOS/ophelia-service");
        fs::create_dir_all(service_binary.parent().unwrap()).unwrap();
        fs::write(&service_binary, b"helper-v1").unwrap();
        let hash = executable_sha256(&service_binary).unwrap();
        let info = service_info_for_binary(&service_binary, Some(hash));

        assert_eq!(
            service_identity(&info, &startup_for_binary(service_binary)),
            ServiceIdentity::Matches
        );
    }

    #[test]
    fn changed_helper_hash_requests_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let service_binary = dir
            .path()
            .join("Ophelia.app/Contents/MacOS/ophelia-service");
        fs::create_dir_all(service_binary.parent().unwrap()).unwrap();
        fs::write(&service_binary, b"helper-v2").unwrap();
        let info = service_info_for_binary(&service_binary, Some("old-hash".into()));

        assert_eq!(
            service_identity(&info, &startup_for_binary(service_binary)),
            ServiceIdentity::NeedsRefresh
        );
    }

    #[test]
    fn unreadable_helper_hash_uses_safe_refresh_path() {
        let dir = tempfile::tempdir().unwrap();
        let service_binary = dir
            .path()
            .join("Ophelia.app/Contents/MacOS/ophelia-service");
        let info = service_info_for_binary(&service_binary, None);

        assert_eq!(
            service_identity(&info, &startup_for_binary(service_binary)),
            ServiceIdentity::NeedsRefresh
        );
    }

    #[test]
    fn bundled_launch_agent_plist_uses_bundle_program() {
        let plist = include_str!("../../../service/macos/nz.ophelia.service.plist");

        assert!(plist.contains("<string>nz.ophelia.service</string>"));
        assert!(plist.contains("<key>nz.ophelia.service</key>"));
        assert!(plist.contains("<key>BundleProgram</key>"));
        assert!(plist.contains("<string>Contents/MacOS/ophelia-service</string>"));
        assert!(!plist.contains("ProgramArguments"));
    }

    #[test]
    fn dev_launch_agent_plist_uses_absolute_program_and_logs() {
        let plist = render_dev_launch_agent_plist(
            Path::new("/tmp/Ophelia & Co/ophelia-service"),
            Path::new("/tmp/Logs"),
        );

        assert!(plist.contains("<string>nz.ophelia.service</string>"));
        assert!(plist.contains("<key>nz.ophelia.service</key>"));
        assert!(plist.contains("/tmp/Ophelia &amp; Co/ophelia-service"));
        assert!(plist.contains("<key>OPHELIA_RUN_SERVICE</key>"));
        assert!(plist.contains("/tmp/Logs/ophelia-service.err.log"));
    }

    #[test]
    fn active_mismatch_warning_keeps_expected_and_actual_owner() {
        let mut info =
            OpheliaServiceInfo::current(&ProfilePaths::new("/tmp/downloads.db", "/tmp/downloads"));
        info.version = "9.9.9".into();
        info.helper.executable = Some(PathBuf::from("/old/ophelia-service"));

        let warning = active_mismatch_warning(&info, Path::new("/new/ophelia-service"));

        assert_eq!(
            warning,
            LocalServiceWarning::ActiveServiceMismatch {
                expected_binary: PathBuf::from("/new/ophelia-service"),
                actual_binary: Some(PathBuf::from("/old/ophelia-service")),
                expected_version: env!("CARGO_PKG_VERSION").to_string(),
                actual_version: "9.9.9".into(),
            }
        );
    }

    fn startup_for_binary(service_binary: PathBuf) -> StartupContext {
        StartupContext::AppBundle {
            app_bundle: PathBuf::from("/Applications/Ophelia.app"),
            service_binary,
        }
    }

    fn service_info_for_binary(
        service_binary: &Path,
        executable_sha256: Option<String>,
    ) -> OpheliaServiceInfo {
        let mut info =
            OpheliaServiceInfo::current(&ProfilePaths::new("/tmp/downloads.db", "/tmp/downloads"));
        let install_kind = infer_install_kind(Some(service_binary));
        info.owner.install_kind = install_kind;
        info.owner.executable = Some(service_binary.to_path_buf());
        info.helper.install_kind = install_kind;
        info.helper.executable = Some(service_binary.to_path_buf());
        info.helper.executable_sha256 = executable_sha256;
        info
    }
}
