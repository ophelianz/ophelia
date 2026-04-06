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

use gpui::{Div, Styled, div};

pub use crate::theme::*;
pub use crate::ui::chrome::app_menu_bar::*;
pub use crate::ui::chrome::modal::*;
pub use crate::ui::chrome::popup_surface::*;
pub use crate::ui::chrome::window_header::*;
pub use crate::ui::controls::button::*;
pub use crate::ui::controls::directory_input::*;
pub use crate::ui::controls::dropdown_select::*;
pub use crate::ui::controls::filter_chip::*;
pub use crate::ui::controls::number_input::*;
pub use crate::ui::controls::segmented_control::*;
pub use crate::ui::controls::switch::*;
pub use crate::ui::controls::text_field::*;
pub use crate::ui::primitives::file_type_icon::*;
pub use crate::ui::primitives::icon::*;
pub use crate::ui::primitives::logo::*;
pub use crate::ui::primitives::resizable::*;

pub fn h_flex() -> Div {
    div().flex()
}

pub fn v_flex() -> Div {
    div().flex().flex_col()
}
