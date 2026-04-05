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

pub const FILE_TYPE_ICON_NAMES: &[&str] = &[
    "archive",
    "audio",
    "book",
    "code",
    "default",
    "document",
    "executable",
    "font",
    "image",
    "key",
    "mail",
    "presentation",
    "spreadsheet",
    "vector",
    "video",
    "web",
];

pub fn file_type_icon(name: &str, size: Pixels, color: impl Into<Hsla>) -> Svg {
    svg()
        .path(file_type_icon_path(name))
        .size(size)
        .flex_shrink_0()
        .text_color(color)
}

pub fn file_type_icon_sm(name: &str, color: impl Into<Hsla>) -> Svg {
    file_type_icon(name, px(16.0), color)
}

pub fn normalize_file_type_icon_name(name: &str) -> &'static str {
    FILE_TYPE_ICON_NAMES
        .iter()
        .copied()
        .find(|candidate| *candidate == name)
        .unwrap_or("default")
}

fn file_type_icon_path(name: &str) -> SharedString {
    let name = normalize_file_type_icon_name(name);
    SharedString::from(format!("icons/file_types/{name}.svg"))
}
