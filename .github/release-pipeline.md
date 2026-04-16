# macOS Release Pipeline

This document is the maintainer runbook for Ophelia's macOS CI/CD and updater pipeline.

## Current shape

Ophelia currently ships macOS-only updater builds.

- GitHub Releases host signed binaries.
- `https://ophelia.nz/updates/...` hosts manifest JSON only.
- The updater is custom and Ophelia-owned.
- The published GPUI fork used by CI/release is `ophelianz/gpui-oe`.
- The local and CI sibling checkout path is `../gpui-oe`.
- Ophelia still resolves the dependency as Cargo package `gpui-ce` for compatibility during this phase.

## Workflows

### CI

File: `.github/workflows/ci.yml`

- Runs on every `push` and `pull_request`
- macOS only
- Checks out the pinned `gpui-oe` sibling revision
- Runs:
    - `cargo fmt --all -- --check`
    - `cargo check --locked`
    - `cargo test --locked`

The branch-protection-facing job name is intentionally `ci / macos`.

### Nightly

File: `.github/workflows/nightly-macos.yml`

- Triggers on pushes to `main` and `workflow_dispatch`
- Runs a preflight job first
- Builds `arm64` and `x86_64` in parallel
- Signs, notarizes, staples, and minisigns both architectures
- Uploads build outputs as workflow artifacts
- Validates website token access in the final publish job before uploading release assets
- Publishes the rolling `nightly` GitHub release from one serialized job
- Generates both Nightly manifests together
- Pushes both Nightly manifests in one website repo commit

### Stable

File: `.github/workflows/release-macos.yml`

- Triggers on GitHub Release `published`
- Verifies the release tag matches `Cargo.toml`
- Builds `arm64` and `x86_64` in parallel from the release tag
- Signs, notarizes, staples, and minisigns both architectures
- Uploads build outputs as workflow artifacts
- Validates website token access in the final publish job before uploading release assets
- Uploads assets to the published GitHub release from one serialized job
- Generates both Stable manifests together
- Pushes both Stable manifests in one website repo commit

## Why the publish job is serialized

The release/build matrix should be parallel.

The manifest/release publication should be serialized.

That split avoids the old failure mode where one matrix leg could publish assets or a manifest before the other architecture finished, leaving Nightly or Stable in a half-published state.

## GPUI fork pinning

The authoritative CI/release revision lives in:

- `.github/gpui-oe-ref`

The fork/repo name and the Cargo package name are intentionally different right now:

- published repo and sibling checkout: `gpui-oe`
- Cargo package compatibility name: `gpui-ce`

When you intentionally update the published GPUI fork in CI:

1. Update your local sibling checkout.
2. Validate Ophelia against it locally.
3. Update `.github/gpui-oe-ref` to the exact tested commit SHA.
4. Mention the pin bump in the PR or release notes.

## Website manifest destination

The workflows publish manifests into the separate website repo.

Default repo:

- `ophelianz/website`

Optional override:

- repository variable `WEBSITE_REPO`

Important:

- `WEBSITE_REPO` must be `owner/repo`
- do not use a full Git URL
- do not include a `.git` suffix

The workflow preflight intentionally fails before the expensive build if the website repo configuration is wrong.

Because the website deploy token is expected to live in environment secrets, token authentication is validated at the start of the final publish job rather than in the global preflight job. That still prevents partial release publication if the token is missing or invalid.

## Required secrets

Repository secrets currently used by the workflows:

- `APPLE_CERTIFICATE_P12`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_SIGNING_IDENTITY`
- `APPLE_NOTARY_API_KEY_P8_B64`
- `APPLE_NOTARY_API_KEY_ID`
- `APPLE_NOTARY_API_ISSUER_ID`
- `MINISIGN_PRIVATE_KEY`
- `OPHELIA_MINISIGN_PUBKEY`

The notarization key should be a Team App Store Connect API key. Store the `.p8` contents base64-encoded in `APPLE_NOTARY_API_KEY_P8_B64`.

Environment secrets currently used by the workflows:

- `WEBSITE_DEPLOY_TOKEN` in `nightly-release`
- `WEBSITE_DEPLOY_TOKEN` in `stable-release`

Recommended future cleanup:

- move `OPHELIA_MINISIGN_PUBKEY` out of secrets because it is public material
- move `APPLE_SIGNING_IDENTITY` out of secrets because it is not secret
- keep the old Apple ID notary secrets only until the first successful Stable release after the API-key migration, then remove them:
    - `APPLE_NOTARY_APPLE_ID`
    - `APPLE_NOTARY_PASSWORD`
    - `APPLE_NOTARY_TEAM_ID`
- delete the old `WEBSITE_REPO` secret if it still exists; the workflows now use a repo variable or the default repo path

GitHub does not allow repository secret values to be read back out. That means moving `WEBSITE_DEPLOY_TOKEN` from a repo secret into environment secrets is a manual copy step unless the token value is already available outside GitHub.

## Required repo settings to confirm

### Environments

Current workflow environment names:

- `nightly-release`
- `stable-release`

Recommended settings:

- `stable-release` should require reviewer approval
- `stable-release` should hold any stable-only deployment credentials if those diverge later
- `nightly-release` can remain automatic

### Branch protection

`main` should be protected.

At minimum:

- require the `ci / macos` status check
- require pull requests
- disable force-pushes
- restrict who can push directly to `main`

## Website token expectations

`WEBSITE_DEPLOY_TOKEN` should be a fine-grained PAT or GitHub App token with:

- repository access limited to the website repo
- `Contents: Read and write`

The website repo is currently public, but the token still needs write access for manifest commits.

## Notarization notes

The workflows already include retries for:

- `codesign --timestamp`
- app stapling
- DMG stapling

They also print Apple notarization logs on non-accepted app notarization.

Still recommended:

- use a Team App Store Connect API key for `notarytool`
- keep the `.p12` export limited to the real `Developer ID Application` identity only
- keep the minisign private key passwordless in CI

The workflows store the notary credentials in the temporary build keychain with `xcrun notarytool store-credentials`, using a temporary `.p8` file decoded from `APPLE_NOTARY_API_KEY_P8_B64`. The temp key file is deleted in cleanup.

## Observability and debugging

When a release job fails:

1. Check the preflight job first.
2. Check whether both architecture build jobs completed.
3. Check whether the serialized publish job ran.
4. Inspect the GitHub Release asset list.
5. Inspect the website repo commit history for `public/updates/...`.
6. Verify the deployed manifest URLs under `https://ophelia.nz/updates/...`.

Useful symptoms:

- `Checkout website repo` failing with `Not Found` usually means `WEBSITE_REPO` is wrong or malformed.
- A release containing only one architecture means publication happened too early or one build failed before the serialized publish job.
- A successful GitHub Release with stale manifests means website repo push or site deployment failed after release upload.

## Docs that should stay in sync

If the pipeline changes, update:

- this file
- `.github/CONTRIBUTING.md`
- `scripts/local_nightly_update_qa.sh` if local QA assumptions changed
- any repo/environment secret inventory used by maintainers
