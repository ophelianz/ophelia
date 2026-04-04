# `src/` layout

Ophelia keeps the frontend and backend split into a few clear layers:

- `ui/`: reusable UI building blocks that are not tied to a specific product screen
- `views/`: app-specific compositions such as windows, panels, lists, and overlays
- `theme.rs`: shared design tokens and visual constants
- `app.rs`: app-layer bridge between GPUI and the backend engine
- `app_menu.rs` / `app_actions.rs`: app-level actions, shortcuts, and menu wiring
- `platform/`: platform-specific window/chrome integration
- `engine/`: download engine, persistence, and provider-specific backend logic
- `ipc.rs`: local ingress for browser-extension download handoff
- `settings/`: persistent application settings

## Frontend terms

These names are intentional:

- `primitive`: low-level reusable building block
- `control`: interactive widget with behavior/state
- `chrome`: reusable window/menu/modal shell UI
- `view`: app-specific composition of controls and chrome
- `helper`: non-visual utility function or tiny render helper

## Backend terms

These names are intentional too:

- `app bridge`: the app-facing state/entity layer that connects GPUI to backend services
- `engine`: provider-neutral runtime control, shared types, scheduling, and orchestration
- `provider`: protocol/tool-specific download implementation such as HTTP
- `state`: persistence and history access
- `ingress`: transport for getting external requests into the app, such as local IPC

## Directory map

### `ui/`

- `primitives/`
  - `icon.rs`: icon rendering helpers and icon names
  - `logo.rs`: Ophelia logo element
- `controls/`
  - `text_field.rs`: custom text input
  - `number_input.rs`: numeric input control
  - `directory_input.rs`: directory-picker input control
- `chrome/`
  - `window_header.rs`: shared titlebar/header chrome
  - `app_menu_bar.rs`: Linux/Windows client-side app menu bar
  - `modal.rs`: reusable modal shell
- `prelude.rs`: shared UI imports for GPUI-heavy files

### `views/`

- `main/`
  - `main_window.rs`: root application window
  - `sidebar.rs`: main navigation/sidebar
  - `download_list.rs`: active download list composition
  - `download_row.rs`: individual download row pieces
  - `history.rs`: history view and filter chips
  - `stats_bar.rs`: throughput and status summary card
- `settings/`
  - `mod.rs`: settings window entity
  - `general.rs`: general settings section
  - `network.rs`: network settings section
- `overlays/`
  - `download_modal.rs`: add-download overlay
  - `about_modal.rs`: about overlay
  - `notification.rs`: transient notification popup

### Backend-adjacent root

- `app.rs`: GPUI-facing download model, backend service owner, progress polling, and history bridge
- `ipc.rs`: local Axum server plus app-owned IPC ingress handle
- `settings/`
  - `mod.rs`: persisted settings model and atomic load/save
- `engine/`
  - `engine.rs`: `DownloadEngine` handle and `EngineActor`
  - `spec.rs`: provider-neutral add/restore request shapes, ingress normalization, and settings-driven provider/config mapping
  - `types.rs`: shared engine-facing types, persisted source/resume data, progress updates, and engine notifications
  - `state/`: SQLite persistence, provider-kind-aware storage/bootstrap, DB worker, and history reader
  - `http/`: HTTP-specific executor pipeline

## Placement rules

When adding a new file:

- Put it in `ui/` if it should be reusable outside one screen or window.
- Put it in `views/` if it exists to assemble app-specific state and layout.
- Prefer extending an existing subfolder before creating a new top-level category.
- If a view grows, split presentational pieces first before introducing more folders.

For backend code:

- Put provider-neutral orchestration and shared engine types in `engine/`.
- Put protocol/tool-specific download logic in a dedicated provider submodule such as `engine/http/`.
- Put persistence and history access in `engine/state/`, not in provider modules.
- Put transport-specific ingress in modules like `ipc.rs`, not inside provider implementations or the engine actor.
- Keep `app.rs` focused on bridging GPUI state to backend services rather than accumulating provider-specific logic.

For deeper backend notes:

- See `docs/architecture.md` for the as-built backend architecture, current gaps, and incremental direction.
- See `tests/` plus local `engine/state/db.rs` unit tests for backend coverage of the current HTTP executor path, engine notifications, and provider-kind persistence migration.
