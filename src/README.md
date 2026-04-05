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
- `settings/`: persistent application settings, including backend runtime knobs such as the IPC port
  - also stores destination-policy settings such as collision strategy and extension-based routing rules

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
- `artifact state`: whether a transfer's bytes are still present on disk, tracked separately from transfer outcome/history
- `live transfer metadata`: provider kind, source label, and control support cached next to live rows so workflow-shaped views can mirror backend semantics cheaply
- `live transfer removal action`: whether a live row left the active surface because the transfer was cancelled or because the artifact was deleted
- `destination policy`: backend-owned resolution of automatic destination folders, collision behavior, and final-file commit semantics

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
  - `sidebar.rs`: top-level `Transfers` / `History` navigation
  - `download_list.rs`: live transfers surface with internal status filters
  - `download_row.rs`: individual download row pieces
  - `history.rs`: global history view and filter chips
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
  - current remove/delete behavior is backend-owned: the app bridge asks the engine to delete artifacts, removes the live row on engine notification, and keeps history intact
  - also caches provider kind, source label, and control support for each live row
  - backend notifications now distinguish cancel-transfer from delete-artifact even though the current UI still handles both as “remove the live row and refresh history”
  - backend state now supports a frontend model of one `Transfers` surface with internal status filters plus a separate global `History` surface
- `ipc.rs`: local Axum server plus app-owned IPC ingress handle
- `settings/`
  - `mod.rs`: persisted settings model and atomic load/save
- `engine/`
  - `engine.rs`: `DownloadEngine` handle and `EngineActor`
  - `destination.rs`: shared destination resolution, collision handling, and final-file commit helpers
  - `provider.rs`: internal provider dispatch, provider lifecycle capabilities, and scheduler-key mapping between the generic scheduler and concrete provider modules
  - `spec.rs`: provider-neutral add/restore request shapes, ingress normalization, and settings-driven provider/config plus destination-policy mapping
  - `types.rs`: shared engine-facing types, persisted source/resume data, provider-aware history read models, progress updates, and engine notifications
  - `state/`: SQLite persistence, provider-kind-aware storage/bootstrap, provider-specific resume-state helpers, DB worker, and history reader
  - `http/`: HTTP-specific executor pipeline

## Placement rules

When adding a new file:

- Put it in `ui/` if it should be reusable outside one screen or window.
- Put it in `views/` if it exists to assemble app-specific state and layout.
- Prefer extending an existing subfolder before creating a new top-level category.
- If a view grows, split presentational pieces first before introducing more folders.

For backend code:

- Put provider-neutral orchestration and shared engine types in `engine/`.
- Put shared destination/path policy in `engine/destination.rs`, not in `app.rs`, IPC handlers, or provider-specific task files.
- Put protocol/tool-specific download logic in a dedicated provider submodule such as `engine/http/`.
- Put persistence and history access in `engine/state/`, not in provider modules.
- Put transport-specific ingress in modules like `ipc.rs`, not inside provider implementations or the engine actor.
- Keep `app.rs` focused on bridging GPUI state to backend services rather than accumulating provider-specific logic.

For deeper backend notes:

- See `docs/architecture.md` for the as-built backend architecture, current gaps, and incremental direction.
- See `tests/` plus local `engine/destination.rs`, `engine/provider.rs`, `ipc.rs`, `engine/state/db.rs`, and `engine/state/mod.rs` tests for backend coverage of the current HTTP executor path, destination-policy behavior, provider glue, engine notifications, provider-kind persistence migration, history queries, IPC ingress normalization, and DB worker event flow.
- Backend history now keeps transfer outcome and artifact presence separate, which is the basis for "delete file but keep history" behavior.
