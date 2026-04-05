/***************************************************
** This file is part of Ophelia, distributed under the
** terms of the GPL License, version 3 or later.
**
**   ⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜⏜
** ( bugs, do no evil and behave plz )
**   ⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝⏝
**   ○
**     ○
**       ／l、
**     （ﾟ､ ｡ ７
**       l  ~ヽ
**       じしf_,)ノ
**************************************************/

use gpui::{Hsla, Pixels, SharedString, Styled, Svg, px, svg};

/// Icon names mapping to Lucide SVG files in assets/icons/
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
    Network,
    PanelLeftClose,
    PanelLeftOpen,
    Plus,
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
            Self::Network => "network",
            Self::Plus => "plus",
            Self::PanelLeftClose => "panel-left-close",
            Self::PanelLeftOpen => "panel-left-open",
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

/// Default 16px icon
pub fn icon_sm(name: IconName, color: impl Into<Hsla>) -> Svg {
    icon(name, px(16.0), color)
}

pub fn icon_m(name: IconName, color: impl Into<Hsla>) -> Svg {
    icon(name, px(20.0), color)
}
