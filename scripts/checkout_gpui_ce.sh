#!/usr/bin/env bash
set -euo pipefail

repo="${GPUI_REPO_URL:-https://github.com/ophelianz/gpui-ce.git}"
dest="${GPUI_CHECKOUT_DEST:-../gpui-ce}"
ref_file="${GPUI_REF_FILE:-.github/gpui-ce-ref}"

if [[ ! -f "${ref_file}" ]]; then
    echo "missing gpui-ce ref file: ${ref_file}" >&2
    exit 1
fi

gpui_ref="$(tr -d '[:space:]' < "${ref_file}")"

if [[ ! "${gpui_ref}" =~ ^[0-9a-f]{40}$ ]]; then
    echo "gpui-ce ref must be a full 40-character commit SHA: ${gpui_ref}" >&2
    exit 1
fi

rm -rf "${dest}"
git clone --filter=blob:none --no-checkout "${repo}" "${dest}"
git -C "${dest}" fetch --depth 1 origin "${gpui_ref}"
git -C "${dest}" checkout --detach "${gpui_ref}"

resolved_ref="$(git -C "${dest}" rev-parse HEAD)"
if [[ "${resolved_ref}" != "${gpui_ref}" ]]; then
    echo "gpui-ce resolved to ${resolved_ref}, expected ${gpui_ref}" >&2
    exit 1
fi

echo "Checked out ${repo} at ${resolved_ref} into ${dest}"
