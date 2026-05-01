use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=OPHELIA_RELEASE_CHANNEL");
    println!("cargo:rerun-if-env-changed=OPHELIA_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=OPHELIA_BUILD_TIMESTAMP");
    println!("cargo:rerun-if-env-changed=OPHELIA_UPDATE_MANIFEST_BASE_URL");
    println!("cargo:rerun-if-env-changed=OPHELIA_MINISIGN_PUBKEY");

    let release_channel = env::var("OPHELIA_RELEASE_CHANNEL").unwrap_or_else(|_| "dev".into());
    let build_commit = env::var("OPHELIA_BUILD_COMMIT").unwrap_or_default();
    let build_timestamp = env::var("OPHELIA_BUILD_TIMESTAMP").unwrap_or_default();
    let manifest_base_url = env::var("OPHELIA_UPDATE_MANIFEST_BASE_URL")
        .unwrap_or_else(|_| "https://ophelia.nz/updates".into());
    let minisign_pubkey = env::var("OPHELIA_MINISIGN_PUBKEY").unwrap_or_default();

    println!("cargo:rustc-env=OPHELIA_RELEASE_CHANNEL={release_channel}");
    println!("cargo:rustc-env=OPHELIA_BUILD_COMMIT={build_commit}");
    println!("cargo:rustc-env=OPHELIA_BUILD_TIMESTAMP={build_timestamp}");
    println!("cargo:rustc-env=OPHELIA_UPDATE_MANIFEST_BASE_URL={manifest_base_url}");
    println!("cargo:rustc-env=OPHELIA_MINISIGN_PUBKEY={minisign_pubkey}");
}
