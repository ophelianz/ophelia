use gpui::{Rgba, rgb, rgba};

// ---------------------------------------------------------------------------
// Color palette
// ---------------------------------------------------------------------------

pub struct Colors;

impl Colors {
    // Backgrounds
    pub fn background() -> Rgba { rgb(0x010202) }
    pub fn sidebar() -> Rgba    { rgb(0x010202) }
    pub fn card() -> Rgba       { rgb(0x0f0f0f) }
    pub fn card_hover() -> Rgba { rgb(0x141414) }
    pub fn muted() -> Rgba      { rgb(0x1a1a1a) }

    // Text
    pub fn foreground() -> Rgba       { rgb(0xf5f5f5) }
    pub fn muted_foreground() -> Rgba { rgb(0x737373) }

    // State colors
    pub fn active() -> Rgba    { rgb(0x7ED37F) }
    pub fn active_dim() -> Rgba { rgba(0x7ED37F73) }
    pub fn finished() -> Rgba  { rgb(0x4A90D9) }
    pub fn queued() -> Rgba    { rgb(0xA78BFA) }
    pub fn error() -> Rgba     { rgb(0xE5634D) }

    // Borders
    pub fn border() -> Rgba      { rgba(0xffffff12) }
    pub fn input_border() -> Rgba { rgba(0xffffff1a) }
    pub fn ring() -> Rgba        { rgba(0x7ED37F66) }
}

// ---------------------------------------------------------------------------
// Spacing
// ---------------------------------------------------------------------------

pub struct Spacing;

impl Spacing {
    pub const SIDEBAR_WIDTH: f32 = 232.0;
    pub const CONTENT_PADDING_X: f32 = 32.0;
    pub const CONTENT_PADDING_Y: f32 = 24.0;
    pub const CARD_GAP: f32 = 14.0;
    pub const LIST_GAP: f32 = 8.0;
    pub const ROW_PADDING_X: f32 = 20.0;
    pub const ROW_PADDING_Y: f32 = 14.0;
}
