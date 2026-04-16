# Contributing to Ophelia

> [!NOTE]
> This doc is a work in progress and subject to change

First off, **all PRs are welcome**!

If you have an idea that makes Ophelia better, clearer, nicer to use, or easier to extend, go ahead and open a PR :p

That said, unfortunately for you, I'm a bit of a perfectionist (or add/autistic, diagnosis pending).

Ophelia is being built with a pretty deliberate architecture. The goal is not just to make features work today. The goal is to make future features, providers, and extensions fit into the codebase cleanly without the project turning into a pile of special cases. I won't bash you with insults Linus-style if you submit a bad PR but I WILL look at you like this

![very mad guy](https://100r.co/media/interface/travel.png)

This guide is **frontend-focused for now**. I'll add more backend-specific stuff later

# AI Disclaimer

Please disclaim any use of generative AI in your code, we will not accept PRs with AI-generated images, icons, SVG files, or other critically artistic works are included in the content of the pull request.

## Philosophy

Ophelia tries to be:

- modular without being over-abstracted
- explicit without being verbose
- extensible without feeling framework-y
- testable without turning simple code into ceremony

In practice, that means we prefer code that fits into the existing shape of the project over code that is clever in isolation.

### When abstraction is worth it

Usually yes:

- there are multiple implementations with meaningfully different construction or behavior
- deferred execution or lifecycle ownership matters
- the abstraction removes repeated complexity for the caller
- the abstraction matches a real product or architectural boundary

Usually no:

- it only exists to share a couple of member variables
- it hides simple code behind a new layer with no real payoff
- it makes ownership harder to follow
- it is more generic than any current or near-future use case needs

If in doubt, prefer the simpler shape.

## Understanding Ophelia

If you want the bigger picture, start here:

- [src/README.md](/Users/viktorluna/Documents/ophelia/src/README.md)

## Quick GPUI Overview

GPUI is a hybrid immediate/retained UI framework.

For Ophelia contributors, the short version is:

- entities own state
- `render()` builds a UI tree from that state
- views observe entities they depend on
- controls emit typed events upward
- actions and overlays are routed through app-level state when they matter globally

You do **not** need to think about GPUI as “React in Rust” or as a scene graph you should manually control all the time. The most important habit is: keep ownership clear.

In Ophelia, the usual pattern is:

- reusable building blocks go in `src/ui/`
- app-specific compositions go in `src/views/`
- frontend-facing app semantics live in `src/app.rs`

When possible, we follow patterns from:

- `gpui-component`
- `gpui-ce`

That is intentional. `gpui-component` comes from a major GPUI contributor and is one of the best practical references for idiomatic GPUI usage. If you are unsure how to structure a control or interaction pattern, copying that style is usually better than inventing a brand-new Ophelia-specific one.

Ophelia is currently being migrated to a fresh upstream-based fork, `gpui-oe`, while keeping the crate package name compatible as `gpui-ce` for the first cutover. The expected local layout during this spike is:

- `../ophelia`
- `../gpui-oe`

GitHub CI and release builds now pin the published sibling checkout through:

- `.github/gpui-oe-ref`

The important compatibility split is:

- repository and sibling checkout name: `gpui-oe`
- Cargo package compatibility name: `gpui-ce`

That means Ophelia still depends on `package = "gpui-ce"` while resolving it from the `../gpui-oe` checkout.

If you intentionally need newer GPUI behavior in CI or release builds, update the published `gpui-oe` fork first, then update the pin in the same change and call it out in the PR.

If the local dev setup changes later, we will move back to a git or published dependency.

## Local updater QA

The custom updater is currently macOS-only and easiest to validate through the local Nightly flow.

Use:

- `scripts/local_nightly_update_qa.sh --minisign-pubkey "<pubkey>"`

That helper writes a reusable env file in `/tmp/ophelia-update-lab/qa-env.sh` and prints the exact remaining steps for:

- building an older Nightly QA app
- building a newer Nightly app
- signing, notarizing, and stapling the newer app
- rebuilding the updater ZIP and manifest
- serving the local update site

It intentionally does **not** try to hide Apple notarization latency. If Apple’s queue is slow, the helper should still leave you with a reproducible flow instead of terminal-history archaeology.

## Release Pipeline

The macOS release/update pipeline now has a dedicated runbook:

- `.github/release-pipeline.md`

Read that before changing GitHub Actions, release secrets, website manifest publication, or the pinned `gpui-oe` revision.

That includes Apple signing and notarization auth changes. Keep those details in the runbook instead of scattering them across PR comments or workflow history.

## Frontend Structure

The frontend is organized around a few simple layers:

### `src/ui/`

Reusable UI building blocks.

- `primitives/` for low-level reusable pieces
- `controls/` for interactive widgets with behavior/state
- `chrome/` for app shell pieces like headers, popups, and modal surfaces

If a piece could reasonably be reused across multiple screens, it probably belongs here.

### `src/views/`

App-specific screens, panels, and overlays.

Examples:

- main window surfaces
- settings sections
- modal layers
- product-specific row/list compositions

If it only makes sense inside Ophelia’s product UI, it probably belongs here.

### `src/app.rs`

This is the frontend-facing bridge to backend state.

Views should prefer consuming read models from here instead of re-deriving backend semantics ad hoc inside UI files.

That means:

- keep UI copy and presentation formatting in the views
- move reusable frontend-facing semantics into the app bridge
- do not make views infer everything from raw engine arrays or backend internals

## Frontend Rules

### 1. Builder-tree first

Use normal GPUI builder-tree layout for shells, panels, rows, and settings layouts.

Custom painting is fine when it is actually needed, but it should be the exception, not the default.

Good uses of custom drawing:

- graphs
- logos
- truly custom visualizations

Bad uses:

- things that are really just flex rows and cards in disguise

### 2. Keep render methods thin

Prefer:

- small local view models
- helper methods that derive render-ready state
- presentational subcomponents when a chunk of UI gets dense

Avoid:

- mixing data derivation, app semantics, and visual composition in one long `render()`

### 3. Reuse shared controls before inventing new inline patterns

If the interaction already exists as a shared control, use it.

Examples:

- `Button`
- `FilterChip`
- `SegmentedControl`
- `DropdownSelect`
- `NumberInput`
- `DirectoryInput`
- `PopupSurface`

If you find yourself rebuilding the same clickable `div()` pattern in multiple places, that is usually a sign the pattern should be converged.

### 4. Keep settings ownership centralized

`src/views/settings/mod.rs` is intentionally the owner of settings draft state and routing.

Do not explode settings into a mini framework unless there is a very strong reason. Small leaf rendering helpers are fine. Shattering ownership across many files usually is not.

### 5. Keep overlays app-routed when they are global

Ophelia uses app-level actions plus shared visibility state for global overlays and windows.

That is the right pattern for things like:

- settings
- download modal
- about modal

Do not wire these up through random local closures if they are meant to work from menus, shortcuts, and buttons consistently.

### 6. Respect viewport and resizing behavior

Resizable UI is part of the architecture now, not just polish.

Every significant surface should have a clear resize contract:

- sensible default size
- sensible min/max range
- a clear scroll owner
- a clear text collapse strategy

Text should truncate or wrap before controls disappear off-screen. New multi-pane layouts should use the shared resizable primitive instead of bespoke split logic.

### 7. Internationalize new user-facing text

If you add user-visible strings, wire them through `rust-i18n`.

This does not need to become a giant translation pass every time you touch a file, but new text should not quietly hardcode English if we are already in that area of the UI.

### 8. Prefer established Ophelia patterns over “clean slate” rewrites

If a part of the frontend already has an intentional pattern, extend it.

If you think the pattern is bad, say so in the PR and justify the change clearly. “I personally prefer X” is not enough. Explain the maintenance or product payoff.

### 9. Use GPUI-native tests when the behavior is UI-shaped

If a behavior depends on GPUI entities, focus, clicks, typing, subscriptions, prompts, clipboard, restart, or resize, prefer a GPUI `TestApp` test over “we clicked around manually and it seemed fine.”

Quick rule:

- pure logic -> normal unit test
- real UI behavior -> `TestApp`

`TestAppContext` and the lower-level visual/test contexts are still fine when a test needs a capability the higher-level wrapper does not expose cleanly. They are the escape hatch, not the default.

For now, colocated `#[cfg(test)]` modules are preferred unless there is a strong reason to build a heavier test harness.

## Known Frontend Hotspots

These are not forbidden areas, but you should treat them carefully:

- `src/ui/controls/text_field.rs`
- `src/views/settings/mod.rs`
- `src/views/main/stats_bar.rs`

These files carry more complexity than average. Changes here should usually come with extra care, extra testing, and a strong reason.

## What Good Frontend PRs Usually Look Like

Good PRs usually do one or more of these:

- converge repeated UI patterns into a shared primitive or control
- simplify a view by moving semantics into `src/app.rs`
- improve resize/clipping behavior without adding needless abstraction
- follow a `gpui-component`-style interaction pattern instead of inventing a bespoke one
- make a feature more provider-aware without making the UI mirror backend internals

## What Usually Needs Extra Justification

- new abstraction layers
- large file-structure reorganizations
- custom-painted controls where builder-tree UI would work
- adding framework-like indirection around settings, actions, or overlays
- backend-shaped UI abstractions that leak engine internals into the frontend

## Before Opening a PR

Please make a reasonable effort to:

- read [src/README.md](/Users/viktorluna/Documents/ophelia/src/README.md)
- match existing naming and file placement conventions
- run formatting and tests relevant to your change
- if your feature is testable, add tests for it

For frontend changes, that usually means:

- `cargo fmt`
- `cargo check`
- `cargo test`

If you changed a tricky control or resizing behavior, mention what you tested manually too.

## Final Note

Ophelia absolutely wants contributions.

The bar here is not “don’t touch anything unless you are already deeply familiar with the codebase.” The bar is: understand the local pattern first, then contribute in a way that makes the project easier to grow rather than harder.

If you do that, your PR is very likely moving Ophelia in the right direction.
