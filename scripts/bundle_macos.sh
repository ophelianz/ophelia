#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage:
  scripts/bundle_macos.sh --channel stable|nightly [options]

Options:
  --arch arm64|x86_64|host       Target architecture. Defaults to host.
  --output-dir DIR               Artifact output directory. Defaults to dist/macos/<channel>/<arch>.
  --sign | --no-sign             Import Developer ID cert and sign app/DMG. Defaults to --no-sign.
  --notarize | --no-notarize     Notarize and staple signed app/DMG. Defaults to --no-notarize.
  --minisign | --no-minisign     Sign ZIP and DMG with minisign. Defaults to --no-minisign.
  -h, --help                     Show this help.

Signing environment:
  APPLE_CERTIFICATE_P12          Base64-encoded Developer ID .p12 certificate.
  APPLE_CERTIFICATE_P12_PATH     Path to Developer ID .p12 certificate. Used if APPLE_CERTIFICATE_P12 is empty.
  APPLE_CERTIFICATE_PASSWORD     Password for the .p12 certificate.
  APPLE_SIGNING_IDENTITY         Developer ID Application identity name or hash.

Notarization environment:
  APPLE_NOTARY_API_KEY_P8_B64    Base64-encoded App Store Connect API key.
  APPLE_NOTARY_API_KEY_PATH      Path to App Store Connect API key. Used if APPLE_NOTARY_API_KEY_P8_B64 is empty.
  APPLE_NOTARY_API_KEY_ID        App Store Connect API key id.
  APPLE_NOTARY_API_ISSUER_ID     App Store Connect issuer id.

Minisign environment:
  MINISIGN_PRIVATE_KEY           Minisign private key text.
  MINISIGN_KEY_PATH              Path to minisign private key. Used if MINISIGN_PRIVATE_KEY is empty.
  OPHELIA_MINISIGN_PUBKEY        Optional public key used to verify generated signatures.
EOF
}

die() {
    echo "error: $*" >&2
    exit 1
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

base64_decode_to_file() {
    local encoded="$1"
    local output="$2"
    if printf '' | base64 --decode >/dev/null 2>&1; then
        printf '%s' "${encoded}" | base64 --decode > "${output}"
    else
        printf '%s' "${encoded}" | base64 -D > "${output}"
    fi
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
gui_dir="${repo_root}/crates/ophelia-gui"
gui_manifest="${gui_dir}/Cargo.toml"
entitlements="${gui_dir}/macos/entitlements.plist"
cargo_bundle_version="${CARGO_BUNDLE_VERSION:-0.10.0}"

channel=""
arch="host"
output_dir=""
sign=false
notarize=false
minisign=false

while (($# > 0)); do
    case "$1" in
        --channel)
            shift
            channel="${1:-}"
            ;;
        --arch)
            shift
            arch="${1:-}"
            ;;
        --output-dir)
            shift
            output_dir="${1:-}"
            ;;
        --sign)
            sign=true
            ;;
        --no-sign)
            sign=false
            ;;
        --notarize)
            notarize=true
            ;;
        --no-notarize)
            notarize=false
            ;;
        --minisign)
            minisign=true
            ;;
        --no-minisign)
            minisign=false
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown argument: $1"
            ;;
    esac
    shift
done

[[ "$(uname -s)" == "Darwin" ]] || die "macOS bundling must run on macOS."
[[ "${channel}" == "stable" || "${channel}" == "nightly" ]] || die "--channel must be stable or nightly."
[[ -f "${entitlements}" ]] || die "missing entitlements file: ${entitlements}"

host_arch="$(uname -m)"
case "${host_arch}" in
    arm64|aarch64)
        host_arch="arm64"
        ;;
    x86_64|amd64)
        host_arch="x86_64"
        ;;
    *)
        die "unsupported host architecture: ${host_arch}"
        ;;
esac

if [[ "${arch}" == "host" ]]; then
    arch="${host_arch}"
fi
[[ "${arch}" == "arm64" || "${arch}" == "x86_64" ]] || die "--arch must be arm64, x86_64, or host."
[[ "${arch}" == "${host_arch}" ]] || die "cross-architecture macOS bundling is not supported here; requested ${arch} on ${host_arch}."

if [[ "${notarize}" == true && "${sign}" != true ]]; then
    die "--notarize requires --sign."
fi

app_name="Ophelia"
bundle_identifier="nz.ophelia.app"
artifact_base="Ophelia-macos-${arch}"
volume_name="Ophelia"

if [[ -z "${output_dir}" ]]; then
    output_dir="${repo_root}/dist/macos/${channel}/${arch}"
fi
mkdir -p "${output_dir}"
output_dir="$(cd "${output_dir}" && pwd)"

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/ophelia-macos-bundle.XXXXXX")"
manifest_backup=""
keychain_path=""
certificate_path=""
notary_key_path=""
minisign_private_key_path=""
minisign_public_key_path=""
notary_profile=""
previous_keychain=""

cleanup() {
    local status=$?
    if [[ -n "${manifest_backup}" && -f "${manifest_backup}" ]]; then
        cp "${manifest_backup}" "${gui_manifest}"
    fi
    if [[ -n "${previous_keychain}" ]]; then
        security default-keychain -s "${previous_keychain}" >/dev/null 2>&1 || true
    fi
    if [[ -n "${keychain_path}" ]]; then
        security delete-keychain "${keychain_path}" >/dev/null 2>&1 || true
    fi
    rm -rf "${work_dir}"
    exit "${status}"
}
trap cleanup EXIT

install_cargo_bundle() {
    need_cmd cargo
    local expected_version
    expected_version="cargo-bundle v${cargo_bundle_version}"
    local actual_version=""
    if cargo bundle --version >/dev/null 2>&1; then
        actual_version="$(cargo bundle --version)"
    fi
    if [[ "${actual_version}" != "${expected_version}" ]]; then
        cargo install cargo-bundle --version "${cargo_bundle_version}" --locked
    fi
}

select_bundle_metadata() {
    manifest_backup="${work_dir}/Cargo.toml.orig"
    cp "${gui_manifest}" "${manifest_backup}"

    CHANNEL="${channel}" MANIFEST="${gui_manifest}" python3 - <<'PY'
import os
from pathlib import Path

channel = os.environ["CHANNEL"]
manifest = Path(os.environ["MANIFEST"])
text = manifest.read_text(encoding="utf-8")
lines = text.splitlines(keepends=True)


def table_bounds(table_name: str) -> tuple[int, int]:
    header = f"[package.metadata.{table_name}]\n"
    start = None
    for index, line in enumerate(lines):
        if line == header:
            start = index
            break
    if start is None:
        raise SystemExit(f"missing {header.strip()} in {manifest}")
    end = len(lines)
    for index in range(start + 1, len(lines)):
        if lines[index].startswith("["):
            end = index
            break
    return start, end


selected_start, selected_end = table_bounds(f"bundle-{channel}")
bundle_start, bundle_end = table_bounds("bundle")

selected = lines[selected_start:selected_end]
selected[0] = "[package.metadata.bundle]\n"
replacement = lines[:bundle_start] + selected + lines[bundle_end:]
manifest.write_text("".join(replacement), encoding="utf-8")
PY
}

restore_bundle_metadata() {
    if [[ -n "${manifest_backup}" && -f "${manifest_backup}" ]]; then
        cp "${manifest_backup}" "${gui_manifest}"
        manifest_backup=""
    fi
}

find_app_bundle() {
    local candidate
    for candidate in \
        "${repo_root}/target/release/bundle/osx/${app_name}.app" \
        "${gui_dir}/target/release/bundle/osx/${app_name}.app"; do
        if [[ -d "${candidate}" ]]; then
            printf '%s\n' "${candidate}"
            return 0
        fi
    done
    find "${repo_root}/target/release/bundle" "${gui_dir}/target/release/bundle" \
        -name "${app_name}.app" -type d -print -quit 2>/dev/null
}

plist_value() {
    local plist="$1"
    local key="$2"
    /usr/libexec/PlistBuddy -c "Print :${key}" "${plist}" 2>/dev/null || true
}

validate_app_bundle() {
    local app_bundle="$1"
    local info_plist="${app_bundle}/Contents/Info.plist"
    [[ -f "${info_plist}" ]] || die "missing Info.plist in ${app_bundle}"

    local actual_identifier
    actual_identifier="$(plist_value "${info_plist}" "CFBundleIdentifier")"
    [[ "${actual_identifier}" == "${bundle_identifier}" ]] || die "expected bundle id ${bundle_identifier}, found ${actual_identifier}"

    local actual_name
    actual_name="$(plist_value "${info_plist}" "CFBundleName")"
    [[ -z "${actual_name}" || "${actual_name}" == "${app_name}" ]] || die "expected app name ${app_name}, found ${actual_name}"

    local required_resources=(
        "Contents/Resources/AppIcon.icns"
        "Contents/Resources/assets/logo.svg"
        "Contents/Resources/assets/fonts/Inter-VariableFont_opsz,wght.ttf"
        "Contents/Resources/locales/en.yml"
    )
    local resource
    for resource in "${required_resources[@]}"; do
        [[ -e "${app_bundle}/${resource}" ]] || die "missing bundled resource: ${resource}"
    done
}

prepare_signing_keychain() {
    need_cmd security
    need_cmd codesign
    [[ -n "${APPLE_CERTIFICATE_PASSWORD:-}" ]] || die "APPLE_CERTIFICATE_PASSWORD is required with --sign."
    [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]] || die "APPLE_SIGNING_IDENTITY is required with --sign."

    certificate_path="${work_dir}/ophelia-certificate.p12"
    if [[ -n "${APPLE_CERTIFICATE_P12:-}" ]]; then
        base64_decode_to_file "${APPLE_CERTIFICATE_P12}" "${certificate_path}"
    elif [[ -n "${APPLE_CERTIFICATE_P12_PATH:-}" ]]; then
        cp "${APPLE_CERTIFICATE_P12_PATH}" "${certificate_path}"
    else
        die "APPLE_CERTIFICATE_P12 or APPLE_CERTIFICATE_P12_PATH is required with --sign."
    fi

    previous_keychain="$(security default-keychain | tr -d '\"' || true)"
    keychain_path="${work_dir}/ophelia-build.keychain-db"
    local keychain_password
    keychain_password="$(uuidgen)"

    security create-keychain -p "${keychain_password}" "${keychain_path}"
    security default-keychain -s "${keychain_path}"
    security unlock-keychain -p "${keychain_password}" "${keychain_path}"
    security set-keychain-settings "${keychain_path}"
    security import "${certificate_path}" -k "${keychain_path}" -P "${APPLE_CERTIFICATE_PASSWORD}" -T /usr/bin/codesign
    security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "${keychain_password}" "${keychain_path}"

    local matching_identities
    matching_identities="$(security find-identity -v -p codesigning "${keychain_path}" | grep -F "${APPLE_SIGNING_IDENTITY}" || true)"
    if [[ -z "${matching_identities}" ]]; then
        echo "::error::Expected signing identity '${APPLE_SIGNING_IDENTITY}' was not imported."
        security find-identity -v -p codesigning "${keychain_path}"
        exit 1
    fi

    local match_count
    match_count="$(printf '%s\n' "${matching_identities}" | sed '/^$/d' | wc -l | tr -d ' ')"
    [[ "${match_count}" == "1" ]] || die "expected one matching signing identity, found ${match_count}: ${matching_identities}"
}

prepare_notary_profile() {
    need_cmd xcrun
    [[ -n "${APPLE_NOTARY_API_KEY_ID:-}" ]] || die "APPLE_NOTARY_API_KEY_ID is required with --notarize."
    [[ -n "${APPLE_NOTARY_API_ISSUER_ID:-}" ]] || die "APPLE_NOTARY_API_ISSUER_ID is required with --notarize."

    notary_key_path="${work_dir}/ophelia-notary-authkey.p8"
    if [[ -n "${APPLE_NOTARY_API_KEY_P8_B64:-}" ]]; then
        base64_decode_to_file "${APPLE_NOTARY_API_KEY_P8_B64}" "${notary_key_path}"
    elif [[ -n "${APPLE_NOTARY_API_KEY_PATH:-}" ]]; then
        cp "${APPLE_NOTARY_API_KEY_PATH}" "${notary_key_path}"
    else
        die "APPLE_NOTARY_API_KEY_P8_B64 or APPLE_NOTARY_API_KEY_PATH is required with --notarize."
    fi

    notary_profile="ophelia-notary-$$"
    xcrun notarytool store-credentials "${notary_profile}" \
        --key "${notary_key_path}" \
        --key-id "${APPLE_NOTARY_API_KEY_ID}" \
        --issuer "${APPLE_NOTARY_API_ISSUER_ID}" \
        --validate \
        --keychain "${keychain_path}"
}

sign_code_path() {
    local code_path="$1"
    for attempt in 1 2 3; do
        if codesign --force --options runtime --timestamp --entitlements "${entitlements}" --verbose=4 --sign "${APPLE_SIGNING_IDENTITY}" "${code_path}"; then
            return 0
        fi
        [[ "${attempt}" -lt 3 ]] || return 1
        echo "codesign failed; retrying in 20s (attempt ${attempt}/3)"
        sleep 20
    done
}

sign_app_bundle() {
    local app_bundle="$1"
    local code_path
    while IFS= read -r -d '' code_path; do
        sign_code_path "${code_path}"
    done < <(find "${app_bundle}/Contents" -type f \( -perm -111 -o -name "*.dylib" -o -name "*.so" \) -print0)

    sign_code_path "${app_bundle}"
    codesign --verify --strict --verbose=4 "${app_bundle}"
}

submit_for_notarization() {
    local artifact="$1"
    local label="$2"
    local result
    result="$(xcrun notarytool submit "${artifact}" \
        -p "${notary_profile}" \
        --keychain "${keychain_path}" \
        --wait \
        --output-format json)"
    echo "${result}"

    local status
    status="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])' <<<"${result}")"
    if [[ "${status}" != "Accepted" ]]; then
        local submission_id
        submission_id="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' <<<"${result}")"
        echo "${label} notarization failed with status: ${status}" >&2
        xcrun notarytool log "${submission_id}" \
            -p "${notary_profile}" \
            --keychain "${keychain_path}" || true
        exit 1
    fi
}

staple_with_retries() {
    local artifact="$1"
    local label="$2"
    for attempt in 1 2 3 4 5; do
        if xcrun stapler staple "${artifact}"; then
            return 0
        fi
        [[ "${attempt}" -lt 5 ]] || return 1
        echo "Stapler ticket not ready yet for ${label}; retrying in 30s (attempt ${attempt}/5)"
        sleep 30
    done
}

notarize_app_bundle() {
    local app_bundle="$1"
    local notary_zip="${work_dir}/${artifact_base}-notary.zip"
    ditto -c -k --keepParent --rsrc --sequesterRsrc "${app_bundle}" "${notary_zip}"
    submit_for_notarization "${notary_zip}" "App bundle"
    staple_with_retries "${app_bundle}" "app bundle"
    xcrun stapler validate "${app_bundle}"
    spctl --assess --type execute -vv "${app_bundle}"
}

create_updater_zip() {
    local app_bundle="$1"
    local zip_path="$2"
    rm -f "${zip_path}"
    ditto -c -k --keepParent --rsrc --sequesterRsrc "${app_bundle}" "${zip_path}"
}

create_dmg() {
    local app_bundle="$1"
    local dmg_path="$2"
    local dmg_root="${work_dir}/dmg-root"
    rm -rf "${dmg_root}" "${dmg_path}"
    mkdir -p "${dmg_root}"
    ditto "${app_bundle}" "${dmg_root}/${app_name}.app"
    ln -s /Applications "${dmg_root}/Applications"
    hdiutil create -volname "${volume_name}" -srcfolder "${dmg_root}" -ov -format UDZO "${dmg_path}"
    hdiutil verify "${dmg_path}"
}

sign_dmg() {
    local dmg_path="$1"
    codesign --force --timestamp --verbose=4 --sign "${APPLE_SIGNING_IDENTITY}" "${dmg_path}"
    codesign --verify --strict --verbose=4 "${dmg_path}"
}

notarize_dmg() {
    local dmg_path="$1"
    submit_for_notarization "${dmg_path}" "DMG"
    staple_with_retries "${dmg_path}" "DMG"
    xcrun stapler validate "${dmg_path}"
    spctl --assess --type open --context context:primary-signature -vv "${dmg_path}"
}

mount_smoke_dmg() {
    local dmg_path="$1"
    local mount_dir="${work_dir}/dmg-mount"
    rm -rf "${mount_dir}"
    mkdir -p "${mount_dir}"
    hdiutil attach -readonly -nobrowse -mountpoint "${mount_dir}" "${dmg_path}" >/dev/null
    [[ -d "${mount_dir}/${app_name}.app" ]] || die "DMG is missing ${app_name}.app"
    [[ -L "${mount_dir}/Applications" ]] || die "DMG is missing /Applications symlink"
    hdiutil detach "${mount_dir}" >/dev/null
}

prepare_minisign_key() {
    need_cmd minisign
    if [[ -n "${MINISIGN_PRIVATE_KEY:-}" ]]; then
        minisign_private_key_path="${work_dir}/ophelia-minisign.key"
        umask 077
        printf '%s' "${MINISIGN_PRIVATE_KEY}" > "${minisign_private_key_path}"
    elif [[ -n "${MINISIGN_KEY_PATH:-}" ]]; then
        minisign_private_key_path="${MINISIGN_KEY_PATH}"
    else
        die "MINISIGN_PRIVATE_KEY or MINISIGN_KEY_PATH is required with --minisign."
    fi

    if [[ -n "${OPHELIA_MINISIGN_PUBKEY:-}" ]]; then
        minisign_public_key_path="${work_dir}/ophelia-minisign.pub"
        printf '%s\n' "${OPHELIA_MINISIGN_PUBKEY}" > "${minisign_public_key_path}"
    fi
}

minisign_artifact() {
    local artifact="$1"
    minisign -S -s "${minisign_private_key_path}" -m "${artifact}"
    if [[ -n "${minisign_public_key_path}" ]]; then
        minisign -Vm "${artifact}" -p "${minisign_public_key_path}"
    fi
}

need_cmd ditto
need_cmd hdiutil
need_cmd /usr/libexec/PlistBuddy

if [[ "${sign}" == true ]]; then
    prepare_signing_keychain
fi
if [[ "${notarize}" == true ]]; then
    prepare_notary_profile
fi
if [[ "${minisign}" == true ]]; then
    prepare_minisign_key
fi

install_cargo_bundle
select_bundle_metadata

rm -rf \
    "${repo_root}/target/release/bundle/osx/${app_name}.app" \
    "${gui_dir}/target/release/bundle/osx/${app_name}.app"

(
    cd "${gui_dir}"
    OPHELIA_RELEASE_CHANNEL="${channel}" cargo build -p ophelia-gui --release
    OPHELIA_RELEASE_CHANNEL="${channel}" cargo bundle --release --package ophelia-gui
)

restore_bundle_metadata

built_app="$(find_app_bundle)"
[[ -n "${built_app}" ]] || die "cargo-bundle did not produce ${app_name}.app"

staged_app="${work_dir}/${app_name}.app"
ditto "${built_app}" "${staged_app}"
validate_app_bundle "${staged_app}"

if [[ "${sign}" == true ]]; then
    sign_app_bundle "${staged_app}"
fi
if [[ "${notarize}" == true ]]; then
    notarize_app_bundle "${staged_app}"
fi

final_app="${output_dir}/${app_name}.app"
zip_path="${output_dir}/${artifact_base}.zip"
dmg_path="${output_dir}/${artifact_base}.dmg"

rm -rf "${final_app}"
ditto "${staged_app}" "${final_app}"
create_updater_zip "${staged_app}" "${zip_path}"
create_dmg "${staged_app}" "${dmg_path}"

if [[ "${sign}" == true ]]; then
    sign_dmg "${dmg_path}"
fi
if [[ "${notarize}" == true ]]; then
    notarize_dmg "${dmg_path}"
fi
mount_smoke_dmg "${dmg_path}"

if [[ "${minisign}" == true ]]; then
    minisign_artifact "${zip_path}"
    minisign_artifact "${dmg_path}"
fi

printf 'Built macOS %s artifacts in %s\n' "${channel}" "${output_dir}"
