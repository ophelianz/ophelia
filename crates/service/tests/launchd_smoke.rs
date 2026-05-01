#[cfg(target_os = "macos")]
#[test]
#[ignore = "bootstraps a user LaunchAgent and talks to the real Mach service"]
fn launchd_starts_service_and_answers_snapshot() {
    use ophelia::service::{OPHELIA_MACH_SERVICE_NAME, OpheliaClient};
    use std::time::Duration;

    let test_dir =
        std::env::temp_dir().join(format!("ophelia-launchd-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&test_dir).unwrap();
    let plist_path = test_dir.join("nz.ophelia.service.plist");
    let binary = env!("CARGO_BIN_EXE_ophelia-service");
    let logs = test_dir.join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let plist = include_str!("../macos/nz.ophelia.service.plist")
        .replace("__OPHELIA_SERVICE_BINARY__", binary)
        .replace("__OPHELIA_LOG_DIR__", &logs.to_string_lossy());
    std::fs::write(&plist_path, plist).unwrap();

    let domain = format!("gui/{}", current_uid());
    launchctl(["bootstrap", &domain, &plist_path.to_string_lossy()]);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let result = runtime.block_on(async {
        let client = OpheliaClient::connect_local()?;
        for _ in 0..30 {
            match client.snapshot().await {
                Ok(snapshot) => return Ok(snapshot),
                Err(error) => {
                    std::thread::sleep(Duration::from_millis(100));
                    if matches!(error, ophelia::service::OpheliaError::BadRequest { .. }) {
                        return Err(error);
                    }
                }
            }
        }
        client.snapshot().await
    });

    launchctl(["bootout", &format!("{domain}/{OPHELIA_MACH_SERVICE_NAME}")]);
    std::fs::remove_dir_all(&test_dir).ok();

    assert!(result.unwrap().transfers.is_empty());
}

#[cfg(target_os = "macos")]
fn current_uid() -> String {
    let output = std::process::Command::new("id").arg("-u").output().unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[cfg(target_os = "macos")]
fn launchctl<const N: usize>(args: [&str; N]) {
    let status = std::process::Command::new("launchctl")
        .args(args)
        .status()
        .unwrap();
    assert!(status.success());
}
