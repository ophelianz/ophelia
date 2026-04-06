/***************************************************
** This file is part of Ophelia.
** Copyright © 2026 Viktor Luna <viktor@hystericca.dev>
** Released under the GPL License, version 3 or later.
**
** If you found a weird little bug in here, tell the cat:
** viktor@hystericca.dev
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs behave plz, we're all trying our best )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

use gpui::{Rgba, rgb, rgba};

pub const APP_FONT_FAMILY: &str = "Inter";

// ---------------------------------------------------------------------------
// Color palette
// ---------------------------------------------------------------------------

pub struct Colors;

impl Colors {
    // Backgrounds
    pub fn background() -> Rgba {
        rgb(0x010202)
    }
    pub fn sidebar() -> Rgba {
        rgb(0x010202)
    }
    pub fn card() -> Rgba {
        rgb(0x0f0f0f)
    }
    pub fn card_hover() -> Rgba {
        rgb(0x141414)
    }
    pub fn muted() -> Rgba {
        rgb(0x1a1a1a)
    }

    // Text
    pub fn foreground() -> Rgba {
        rgb(0xf5f5f5)
    }
    pub fn muted_foreground() -> Rgba {
        rgb(0x737373)
    }

    // State colors
    pub fn active() -> Rgba {
        rgb(0x7ED37F)
    }
    pub fn active_dim() -> Rgba {
        rgba(0x7ED37F73)
    }
    pub fn finished() -> Rgba {
        rgb(0x4A90D9)
    }
    pub fn queued() -> Rgba {
        rgb(0xA78BFA)
    }
    pub fn error() -> Rgba {
        rgb(0xE5634D)
    }

    // Borders
    pub fn border() -> Rgba {
        rgba(0xffffff12)
    }
    pub fn input_border() -> Rgba {
        rgba(0xffffff1a)
    }
    pub fn ring() -> Rgba {
        rgba(0x7ED37F66)
    }
}

// ---------------------------------------------------------------------------
// Spacing
// ---------------------------------------------------------------------------

pub struct Spacing;

impl Spacing {
    pub const SIDEBAR_WIDTH: f32 = 232.0;
    pub const SIDEBAR_COLLAPSED_WIDTH: f32 = 56.0;
    pub const SIDEBAR_SECTION_PADDING: f32 = 16.0;
    pub const SIDEBAR_NAV_PADDING_X: f32 = 10.0;
    pub const CONTENT_PADDING_X: f32 = 32.0;
    pub const CONTENT_PADDING_Y: f32 = 24.0;
    pub const SECTION_GAP: f32 = 16.0;
    pub const LIST_GAP: f32 = 8.0;
    pub const CONTROL_GAP: f32 = 10.0;
    pub const ROW_GAP: f32 = 12.0;
    pub const ROW_PADDING_X: f32 = 20.0;
    pub const ROW_PADDING_Y: f32 = 14.0;
    pub const SECTION_LABEL_BOTTOM_MARGIN: f32 = 14.0;

    pub const SETTINGS_SECTION_GAP: f32 = 24.0;
    pub const SETTINGS_GROUP_GAP: f32 = 20.0;
    pub const SETTINGS_PANEL_GAP: f32 = 12.0;
    pub const SETTINGS_ROW_GAP: f32 = 24.0;
    pub const SETTINGS_LABEL_GAP: f32 = 3.0;
    pub const SETTINGS_INLINE_GAP: f32 = 8.0;
    pub const SETTINGS_CONTENT_PADDING: f32 = 32.0;
    pub const SETTINGS_CONTROL_WIDTH: f32 = 220.0;
    pub const SETTINGS_SIDEBAR_WIDTH: f32 = 160.0;
}

// ---------------------------------------------------------------------------
// Shared chrome metrics
// ---------------------------------------------------------------------------

pub struct Chrome;

impl Chrome {
    pub const RESIZE_HANDLE_SIZE: f32 = 1.0;
    pub const RESIZE_HANDLE_PADDING: f32 = 4.0;

    pub const CONTROL_RADIUS: f32 = 6.0;
    pub const BUTTON_RADIUS: f32 = 8.0;
    pub const BUTTON_HEIGHT: f32 = 32.0;
    pub const BUTTON_PADDING_X: f32 = 14.0;
    pub const BUTTON_COMPACT_PADDING_X: f32 = 10.0;
    pub const CARD_RADIUS: f32 = 10.0;
    pub const PANEL_RADIUS: f32 = 12.0;
    pub const MODAL_RADIUS: f32 = 14.0;

    pub const SIDEBAR_HEADER_TOP: f32 = 14.0;
    pub const SIDEBAR_HEADER_BOTTOM_MARGIN: f32 = 22.0;
    pub const SIDEBAR_BUTTON_SIZE: f32 = 40.0;
    pub const SIDEBAR_NAV_ITEM_PADDING_X: f32 = 14.0;
    pub const SIDEBAR_NAV_ITEM_PADDING_Y: f32 = 10.0;
    pub const STORAGE_BAR_HEIGHT: f32 = 4.0;

    pub const HEADER_GAP: f32 = 12.0;
    pub const WINDOW_CONTROL_WIDTH: f32 = 38.0;
    pub const WINDOW_CONTROL_HEIGHT: f32 = 28.0;

    pub const MENU_BAR_GAP: f32 = 4.0;
    pub const MENU_TRIGGER_PADDING_X: f32 = 10.0;
    pub const MENU_TRIGGER_PADDING_Y: f32 = 6.0;
    pub const MENU_POPUP_GAP: f32 = 2.0;
    pub const MENU_POPUP_MIN_WIDTH: f32 = 210.0;
    pub const MENU_POPUP_PADDING: f32 = 6.0;
    pub const MENU_ITEM_PADDING_X: f32 = 10.0;
    pub const MENU_ITEM_PADDING_Y: f32 = 8.0;
    pub const MENU_ITEM_CHECK_WIDTH: f32 = 12.0;

    pub const SETTINGS_RULE_CARD_PADDING: f32 = 14.0;
    pub const SETTINGS_SECTION_PANEL_PADDING: f32 = 16.0;
    pub const SETTINGS_ICON_TRIGGER_SIZE: f32 = 36.0;
    pub const SETTINGS_ICON_PICKER_WIDTH: f32 = 252.0;
    pub const SETTINGS_ICON_TILE_SIZE: f32 = 40.0;

    pub const MODAL_PADDING: f32 = 28.0;
    pub const MODAL_STACK_GAP: f32 = 18.0;
    pub const ABOUT_MODAL_WIDTH: f32 = 460.0;
    pub const DOWNLOAD_MODAL_WIDTH: f32 = 520.0;

    pub const PROGRESS_BAR_HEIGHT: f32 = 4.0;
    pub const STATS_CARD_PADDING: f32 = 24.0;
    pub const STATS_GRAPH_HEIGHT: f32 = 120.0;
}
