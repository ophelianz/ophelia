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

use gpui::{Bounds, Pixels, TitlebarOptions, WindowBounds, WindowOptions, point, px};

use crate::platform::WindowChrome;

const TRAFFIC_LIGHT_PADDING: f32 = 72.0;

pub fn window_chrome() -> WindowChrome {
    WindowChrome {
        height: 44.0,
        leading_padding: TRAFFIC_LIGHT_PADDING,
        horizontal_padding: 24.0,
    }
}

pub fn window_options(bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            appears_transparent: true,
            traffic_light_position: Some(point(px(16.0), px(14.0))),
            ..Default::default()
        }),
        ..Default::default()
    }
}
