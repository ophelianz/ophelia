use gpui::{Div, Styled, div};

pub use crate::theme::*;
pub use crate::ui::app_menu_bar::*;
pub use crate::ui::directory_input::*;
pub use crate::ui::icon::*;
pub use crate::ui::logo::*;
pub use crate::ui::modal::*;
pub use crate::ui::number_input::*;
pub use crate::ui::text_field::*;
pub use crate::ui::window_header::*;

pub fn h_flex() -> Div {
    div().flex()
}

pub fn v_flex() -> Div {
    div().flex().flex_col()
}
