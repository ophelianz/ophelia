# `src/` layout

- `ui/`: reusable UI building blocks that are not tied to a specific product screen
- `views/`: app-specific compositions such as windows, panels, lists, and overlays
- `theme.rs`: shared design tokens and visual constants
- `app.rs`: app-layer bridge between GPUI and the backend engine
- `app_menu.rs` / `app_actions.rs` / `tray.rs`: app-level actions, shortcuts, tray integration, and shell wiring
- `platform/`: shared OS integration such as window chrome and app path policy
- `engine/`: download engine, persistence, provider-specific backend logic
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

- `app.rs`: GPUI-facing download model type, backend service owner, progress polling, and history bridge
    - current remove/delete behavior is backend-owned: the app bridge asks the engine to delete artifacts, removes the live row on engine notification, and keeps history intact
    - also caches provider kind, source label, control support, HTTP chunk-map state, and an `id -> index` side map for each live row
    - backend notifications now distinguish cancel-transfer from delete-artifact even though the current UI still handles both as “remove the live row and refresh history”
    - backend state now supports a frontend model of one `Transfers` surface with internal status filters plus a separate global `History` surface
- `app_actions.rs`: app-shell owner for the global `Downloads` entity handle, main/settings window reuse, overlay visibility state, and macOS dock/tray mode transitions
- `tray.rs`: macOS tray/menu-bar bridge that refreshes aggregate speed and routes queued tray intents into `app_actions`
- `ipc.rs`: local Axum server plus app-owned IPC ingress handle
- `platform/`
    - `mod.rs`: platform module root and window-chrome entry points
    - `paths.rs`: shared app config/data/log/download directory policy plus legacy path helpers
- `settings/`
    - `mod.rs`: persisted settings model and atomic load/save, including destination-policy and HTTP ordering-mode settings
- `engine/`
    - `engine.rs`: `DownloadEngine` handle and `EngineActor`
    - `destination.rs`: shared destination resolution, collision handling, and final-file commit helpers
    - `provider.rs`: internal provider dispatch, provider lifecycle capabilities, and scheduler-key mapping between the generic scheduler and concrete provider modules
    - `spec.rs`: provider-neutral add/restore request shapes, ingress normalization, and settings-driven provider/config plus destination-policy mapping
    - `types.rs`: shared engine-facing types, persisted source/resume data, provider-aware history read models, progress updates, and engine notifications
    - `state/`: SQLite persistence, provider-kind-aware storage/bootstrap, provider-specific resume-state helpers, DB worker, and history reader
    - `http/`: HTTP-specific executor pipeline, including live chunk-map snapshot reporting and ordering-mode-aware scheduling for active chunked transfers
