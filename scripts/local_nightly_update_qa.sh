#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage:
  scripts/local_nightly_update_qa.sh [--minisign-pubkey KEY] [--manifest-base-url URL] [--lab-dir DIR] [--qa-app PATH]

Behavior:
  - Writes a reusable env file for local Nightly updater QA.
  - Precomputes old/new Nightly timestamps so local manifests are guaranteed to compare correctly.
  - Prints the remaining manual build, notarization, manifest, and server steps.

Defaults:
  --manifest-base-url  http://127.0.0.1:8000/updates
  --lab-dir            /tmp/ophelia-update-lab
  --qa-app             $HOME/Applications/Ophelia-QA.app

The minisign public key can be provided with --minisign-pubkey or OPHELIA_MINISIGN_PUBKEY.
EOF
}

manifest_base_url="${OPHELIA_UPDATE_MANIFEST_BASE_URL:-http://127.0.0.1:8000/updates}"
lab_dir="/tmp/ophelia-update-lab"
qa_app_path="${HOME}/Applications/Ophelia-QA.app"
minisign_pubkey="${OPHELIA_MINISIGN_PUBKEY:-}"

while (($# > 0)); do
    case "$1" in
        --minisign-pubkey)
            shift
            minisign_pubkey="${1:-}"
            ;;
        --manifest-base-url)
            shift
            manifest_base_url="${1:-}"
            ;;
        --lab-dir)
            shift
            lab_dir="${1:-}"
            ;;
        --qa-app)
            shift
            qa_app_path="${1:-}"
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
    shift
done

if [[ -z "${minisign_pubkey}" ]]; then
    echo "missing minisign public key; pass --minisign-pubkey or set OPHELIA_MINISIGN_PUBKEY" >&2
    exit 1
fi

old_ts="$(python3 - <<'PY'
from datetime import datetime, timedelta, timezone
print((datetime.now(timezone.utc) - timedelta(hours=1)).replace(microsecond=0).isoformat().replace("+00:00", "Z"))
PY
)"
new_ts="$(python3 - <<'PY'
from datetime import datetime, timedelta, timezone
print((datetime.now(timezone.utc) + timedelta(hours=1)).replace(microsecond=0).isoformat().replace("+00:00", "Z"))
PY
)"

qa_arch="$(uname -m)"
case "${qa_arch}" in
    arm64|aarch64)
        qa_arch="arm64"
        ;;
    x86_64|amd64)
        qa_arch="x86_64"
        ;;
    *)
        echo "unsupported macOS architecture for updater QA: ${qa_arch}" >&2
        exit 1
        ;;
esac

site_dir="${lab_dir}/site"
env_file="${lab_dir}/qa-env.sh"
mkdir -p "${site_dir}/updates/macos/${qa_arch}"
mkdir -p "$(dirname "${qa_app_path}")"

cat > "${env_file}" <<EOF
export LAB_DIR='${lab_dir}'
export SITE_DIR='${site_dir}'
export BASE_URL='${manifest_base_url}'
export QA_APP_PATH='${qa_app_path}'
export QA_ARCH='${qa_arch}'
export OLD_TS='${old_ts}'
export NEW_TS='${new_ts}'
export OLD_COMMIT='qa-old'
export NEW_COMMIT='qa-new'
export OPHELIA_RELEASE_CHANNEL='nightly'
export OPHELIA_UPDATE_MANIFEST_BASE_URL='${manifest_base_url}'
export OPHELIA_MINISIGN_PUBKEY='${minisign_pubkey}'
EOF

cat <<EOF
Wrote local Nightly updater QA env file:
  ${env_file}

Start each new shell with:
  source "${env_file}"

If you still need a local minisign keypair:
  minisign -G -p "${lab_dir}/ophelia.pub" -s "${lab_dir}/ophelia.key"

1. Build and install the older Nightly bundle:
  OPHELIA_BUILD_COMMIT="\$OLD_COMMIT" OPHELIA_BUILD_TIMESTAMP="\$OLD_TS" \\
    scripts/bundle_macos.sh --channel nightly --arch "\$QA_ARCH" --output-dir "\$LAB_DIR/old" --no-sign --no-notarize --no-minisign
  rm -rf "\$QA_APP_PATH"
  cp -R "\$LAB_DIR/old/Ophelia.app" "\$QA_APP_PATH"

2. Build the newer Nightly bundle:
  OPHELIA_BUILD_COMMIT="\$NEW_COMMIT" OPHELIA_BUILD_TIMESTAMP="\$NEW_TS" MINISIGN_KEY_PATH="\$LAB_DIR/ophelia.key" \\
    scripts/bundle_macos.sh --channel nightly --arch "\$QA_ARCH" --output-dir "\$SITE_DIR" --sign --notarize --minisign

3. The bundle script signs, notarizes, staples, builds the updater ZIP and DMG, and minisigns both files.

4. Regenerate the local manifest:
  python3 scripts/update_manifest.py \\
    --channel nightly \\
    --version "nightly-local" \\
    --pub-date "\$NEW_TS" \\
    --commit "\$NEW_COMMIT" \\
    --notes-url "http://127.0.0.1:8000/notes" \\
    --asset-url "http://127.0.0.1:8000/Ophelia-macos-\$QA_ARCH.zip" \\
    --asset-size "\$(stat -f%z "\$SITE_DIR/Ophelia-macos-\$QA_ARCH.zip")" \\
    --sha256 "\$(shasum -a 256 "\$SITE_DIR/Ophelia-macos-\$QA_ARCH.zip" | awk '{print \$1}')" \\
    --minisign-url "http://127.0.0.1:8000/Ophelia-macos-\$QA_ARCH.zip.minisig" \\
    --output "\$SITE_DIR/updates/macos/\$QA_ARCH/nightly.json"

5. Serve the update site and launch the installed QA app:
  python3 -m http.server 8000 --directory "\$SITE_DIR"
  open "\$QA_APP_PATH"

6. In Ophelia:
  - set Update Channel to Nightly
  - use Check for Updates…
  - click Install Update when it appears

7. Verify the relaunched app is the new build:
  strings "\$QA_APP_PATH/Contents/MacOS/ophelia" | rg 'qa-old|qa-new'

Note:
  Notarization latency can still take a long time. This helper prepares the local QA metadata,
  but it intentionally does not try to hide Apple's queue or approval delays.
EOF
