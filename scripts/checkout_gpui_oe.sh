#!/usr/bin/env bash
set -euo pipefail

repo="${GPUI_REPO_URL:-https://github.com/ophelianz/gpui-oe.git}"
dest="${GPUI_CHECKOUT_DEST:-../gpui-oe}"
gpui_ref="${GPUI_REF:-}"
ref_file="${GPUI_REF_FILE:-}"

if [[ -z "${gpui_ref}" && -n "${ref_file}" ]]; then
    if [[ ! -f "${ref_file}" ]]; then
        echo "missing gpui-oe ref file: ${ref_file}" >&2
        exit 1
    fi
    gpui_ref="$(tr -d '[:space:]' < "${ref_file}")"
fi

if [[ -e "${dest}" ]]; then
    if [[ -d "${dest}/.git" && -n "$(git -C "${dest}" status --porcelain)" && "${GPUI_FORCE_CLEAN:-}" != "1" ]]; then
        echo "refusing to remove dirty gpui-oe checkout at ${dest}; set GPUI_FORCE_CLEAN=1 to replace it" >&2
        exit 1
    fi
    rm -rf "${dest}"
fi

if [[ -n "${gpui_ref}" ]]; then
    git clone --filter=blob:none --no-checkout "${repo}" "${dest}"
    git -C "${dest}" fetch --depth 1 origin "${gpui_ref}"
    git -C "${dest}" checkout --detach FETCH_HEAD
else
    git clone --filter=blob:none --depth 1 "${repo}" "${dest}"
fi

resolved_ref="$(git -C "${dest}" rev-parse HEAD)"
if [[ -n "${gpui_ref}" ]]; then
    echo "Checked out ${repo} ref ${gpui_ref} at ${resolved_ref} into ${dest} (Cargo packages: gpui, gpui_platform)"
else
    echo "Checked out latest ${repo} at ${resolved_ref} into ${dest} (Cargo packages: gpui, gpui_platform)"
fi
