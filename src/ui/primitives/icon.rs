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

use gpui::{
    App, Hsla, IntoElement, ParentElement, Pixels, RenderOnce, SharedString, Styled, Svg, Window,
    div, px, svg,
};

const ICON_FRAME_SM: f32 = 20.0;
const ICON_GLYPH_SM: f32 = 16.0;
const ICON_FRAME_MD: f32 = 24.0;
const ICON_GLYPH_MD: f32 = 20.0;
const ICON_ACTION_FRAME: f32 = 32.0;

/// Icon names map to SVG files in assets/icons/
#[derive(Debug, Clone, Copy)]
pub enum IconName {
    Inbox,
    ArrowDownToLine,
    CircleCheck,
    CirclePause,
    CirclePlay,
    CircleX,
    Database,
    Folder,
    GeneralSettings,
    History,
    Network,
    PanelLeftClose,
    PanelLeftOpen,
    Plus,
    Storage,
    Trash2,
}

impl IconName {
    pub fn path(self) -> SharedString {
        let name = match self {
            Self::Inbox => "inbox",
            Self::ArrowDownToLine => "arrow-down-to-line",
            Self::CircleCheck => "circle-check",
            Self::CirclePause => "circle-pause",
            Self::CirclePlay => "circle-play",
            Self::CircleX => "circle-x",
            Self::Database => "database",
            Self::Folder => "folder",
            Self::GeneralSettings => "general-settings",
            Self::History => "history",
            Self::Network => "network",
            Self::Plus => "plus",
            Self::PanelLeftClose => "panel-left-close",
            Self::PanelLeftOpen => "panel-left-open",
            Self::Storage => "storage",
            Self::Trash2 => "trash",
        };
        SharedString::from(format!("icons/{name}.svg"))
    }
}

pub fn icon(name: IconName, size: Pixels, color: impl Into<Hsla>) -> Svg {
    svg()
        .path(name.path())
        .size(size)
        .flex_shrink_0()
        .text_color(color)
}

/// A fixed geometry slot for icons.
///
/// The frame owns alignment and row participation; the glyph only owns its SVG
/// path and color.
#[derive(IntoElement, Clone)]
pub struct IconBox {
    name: IconName,
    frame_size: Pixels,
    glyph_size: Pixels,
    color: Hsla,
}

impl IconBox {
    pub fn new(name: IconName, color: impl Into<Hsla>) -> Self {
        Self::custom(name, px(ICON_FRAME_SM), px(ICON_GLYPH_SM), color)
    }

    pub fn medium(name: IconName, color: impl Into<Hsla>) -> Self {
        Self::custom(name, px(ICON_FRAME_MD), px(ICON_GLYPH_MD), color)
    }

    pub fn action(name: IconName, color: impl Into<Hsla>) -> Self {
        Self::custom(name, px(ICON_ACTION_FRAME), px(ICON_GLYPH_SM), color)
    }

    pub fn custom(
        name: IconName,
        frame_size: Pixels,
        glyph_size: Pixels,
        color: impl Into<Hsla>,
    ) -> Self {
        Self {
            name,
            frame_size,
            glyph_size,
            color: color.into(),
        }
    }
}

impl RenderOnce for IconBox {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .size(self.frame_size)
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .child(icon(self.name, self.glyph_size, self.color))
    }
}
