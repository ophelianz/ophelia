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

use gpui::{Bounds, Pixels, Size, WindowOptions};

pub(crate) mod paths;

#[derive(Clone, Copy)]
pub struct WindowChrome {
    pub height: f32,
    pub leading_padding: f32,
    pub horizontal_padding: f32,
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(not(target_os = "macos"))]
mod default;
#[cfg(not(target_os = "macos"))]
use default as imp;

pub fn window_chrome() -> WindowChrome {
    imp::window_chrome()
}

pub fn window_options(bounds: Bounds<Pixels>, min_size: Size<Pixels>) -> WindowOptions {
    imp::window_options(bounds, min_size)
}
