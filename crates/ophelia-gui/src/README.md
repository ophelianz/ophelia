# `src/` layout

- `ui/`: reusable UI building blocks that are not tied to a specific product screen
- `views/`: app-specific compositions such as windows, panels, lists, and overlays
- `theme.rs`: shared design tokens and visual constants
- `app.rs`: GPUI-facing download rows, history rows, and stats view state
- `engine_bridge.rs`: Tokio-owned backend engine bridge and lightweight UI command client
- `app_menu.rs` / `app_actions.rs` / `tray.rs`: app-level actions, shortcuts, tray integration, and shell wiring
- `platform/`: shared OS integration such as window chrome and app path policy
- `runtime.rs`: GPUI-owned Tokio runtime for app services
- `engine.rs`: thin facade over the backend crate named `ophelia`
- `ipc.rs`: local ingress for browser-extension download handoff
- `settings/`: persistent application settings, including backend runtime knobs such as the IPC port
    - also stores destination-policy settings such as collision strategy and extension-based routing rules
    - also stores HTTP download-ordering settings such as `Balanced`, `FileSpecific`, `Sequential`, and the file-specific extension list
    - persists through the shared platform path policy

## Directory map

### `ui/`

- `primitives/`
    - `icon.rs`: icon rendering helpers and icon names
    - `logo.rs`: Ophelia logo made in GPUI
    - `resizable/`: local resizable panel primitive modeled after `gpui-component`
- `controls/`
    - `dropdown_select.rs`: reusable anchored dropdown/select control
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
    - `transfers_list.rs`: live transfers surface with internal status filters
    - `transfer_row.rs`: individual transfer row pieces
    - `history.rs`: global history view and filter chips
    - `stats_bar.rs`: throughput and status summary card
    - `chunk_map.rs`: selected-transfer chunk-map summary card
- `settings/`
    - `mod.rs`: settings window entity
    - `general.rs`: general app settings section
    - `destinations.rs`: fallback folder, collision policy, and destination-rule settings
    - `destination_rule_icon_picker.rs`: destination-rule-specific icon picker composition
    - `network.rs`: network settings section
- `overlays/`
    - `download_modal.rs`: add-download overlay
    - `about_modal.rs`: about overlay
    - `notification.rs`: transient notification popup

### Backend-adjacent root

- `app.rs`: GPUI download model type, live row arrays, history reader, and stats sampler
    - does not own `DownloadEngine`
    - consumes `EngineEvent` from `engine_bridge.rs` and upserts/removes live rows by `DownloadId`
    - caches provider kind, source label, control support, HTTP chunk-map state, and an `id -> index` side map for each live row
- `engine_bridge.rs`: single owner of backend `DownloadEngine`
    - sends async engine commands from `EngineClient`
    - continuously drains `next_event()` and forwards events into GPUI state
- `runtime.rs`: Zed-shaped GPUI global that owns a Tokio runtime/handle for app services
- `app_actions.rs`: app-shell owner for the global `Downloads` entity handle, main/settings window reuse, overlay visibility state, and macOS dock/tray mode transitions
- `tray.rs`: macOS tray/menu-bar bridge that refreshes aggregate speed and routes queued tray intents into `app_actions`
- `ipc.rs`: local Axum server plus app-owned IPC ingress handle, spawned on the shared app runtime
- `platform/`
    - `mod.rs`: platform module root and window-chrome entry points
    - `paths.rs`: shared app config/data/log/download directory policy plus legacy path helpers
- `settings/`
    - `mod.rs`: persisted settings model and atomic load/save, including destination-policy, HTTP ordering-mode settings, and conversion into backend `CoreConfig` / `CorePaths`
