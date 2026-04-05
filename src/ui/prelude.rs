use gpui::{Div, Styled, div};

pub use crate::theme::*;
pub use crate::ui::chrome::app_menu_bar::*;
pub use crate::ui::chrome::modal::*;
pub use crate::ui::chrome::window_header::*;
pub use crate::ui::controls::directory_input::*;
pub use crate::ui::controls::number_input::*;
pub use crate::ui::controls::switch::*;
pub use crate::ui::controls::text_field::*;
pub use crate::ui::primitives::icon::*;
pub use crate::ui::primitives::logo::*;

pub fn h_flex() -> Div {
    div().flex()
}

pub fn v_flex() -> Div {
    div().flex().flex_col()
}
