# Contributing to Ophelia

> [!NOTE]
> This doc is a work in progress and subject to change

First off, **all PRs are welcome**!

If you have an idea that makes Ophelia better, clearer, nicer to use, or easier to extend, go ahead and open a PR :p, just a few things:

## AI Disclaimer

Please disclaim any use of generative AI in your code, we will not accept PRs with AI-generated images, icons, SVG files, or other critically artistic works are included in the content of the pull request.

## Internationalize new UI text

If you add user-visible strings, make sure you wire them thru `rust-i18n`.

## Make sure you to run

- `scripts/checkout_gpui_oe.sh`
- `cargo fmt --check --all`
- `cargo check --locked --workspace --all-targets`
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`
- `cargo test --locked --workspace`

If you touch macOS packaging, updater assets, or bundle metadata, also run an unsigned local bundle
smoke test on macOS:

- `scripts/bundle_macos.sh --channel stable --arch host --output-dir /tmp/ophelia-stable-smoke --no-sign --no-notarize --no-minisign`
- `scripts/bundle_macos.sh --channel nightly --arch host --output-dir /tmp/ophelia-nightly-smoke --no-sign --no-notarize --no-minisign`

If `cargo check --locked` says `Cargo.lock` needs to change after a GPUI fork update, regenerate it
with the latest `../gpui-oe` checkout in place:

- `cargo update -p gpui -p gpui_platform`
- `cargo check --locked --workspace --all-targets`

hey psst, use the kitty script.
