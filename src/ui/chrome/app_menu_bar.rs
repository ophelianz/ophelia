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

use gpui::{
    Context, Entity, FontWeight, IntoElement, MouseButton, MouseMoveEvent, OwnedMenu,
    ParentElement, Render, Window, div, prelude::*, px,
};

use crate::app_menu::{self, OwnedMenuItemLike};
use crate::ui::prelude::*;

pub struct AppMenuBar {
    menus: Vec<OwnedMenu>,
    open_menu: Option<usize>,
}

impl AppMenuBar {
    pub fn new(menus: Vec<OwnedMenu>, cx: &mut Context<Self>) -> Self {
        let _ = cx;
        Self {
            menus,
            open_menu: None,
        }
    }

    pub fn set_menus(&mut self, menus: Vec<OwnedMenu>, cx: &mut Context<Self>) {
        self.menus = menus;
        if self
            .open_menu
            .is_some_and(|index| index >= self.menus.len())
        {
            self.open_menu = None;
        }
        cx.notify();
    }

    fn toggle_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        self.open_menu = if self.open_menu == Some(index) {
            None
        } else {
            Some(index)
        };
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.open_menu.take().is_some() {
            cx.notify();
        }
    }
}

impl Render for AppMenuBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();

        h_flex()
            .id("app-menu-bar")
            .items_center()
            .gap(px(Chrome::MENU_BAR_GAP))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                cx.stop_propagation();
                window.prevent_default();
            })
            .children(self.menus.iter().enumerate().map(|(index, menu)| {
                let is_open = self.open_menu == Some(index);
                let button = div()
                    .id(("menu-trigger", index))
                    .rounded(px(Chrome::CONTROL_RADIUS))
                    .px(px(Chrome::MENU_TRIGGER_PADDING_X))
                    .py(px(Chrome::MENU_TRIGGER_PADDING_Y))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(Colors::foreground())
                    .hover(|style| style.bg(Colors::muted()))
                    .when(is_open, |this| this.bg(Colors::muted()))
                    .child(app_menu::menu_label(menu))
                    .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                        if this.open_menu.is_some() && this.open_menu != Some(index) {
                            this.open_menu = Some(index);
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            window.prevent_default();
                            this.toggle_menu(index, cx);
                        }),
                    );

                let popup = if is_open {
                    Some(render_menu_popup(index, menu, entity.clone(), cx).into_any_element())
                } else {
                    None
                };

                div().relative().h_full().child(button).children(popup)
            }))
    }
}

fn render_menu_popup(
    index: usize,
    menu: &OwnedMenu,
    entity: Entity<AppMenuBar>,
    cx: &mut Context<AppMenuBar>,
) -> impl IntoElement {
    popup_surface(("menu-popup", index))
        .min_width(px(Chrome::MENU_POPUP_MIN_WIDTH))
        .on_close(move |_, app| {
            let _ = entity.update(app, |this, cx| {
                this.close_menu(cx);
            });
        })
        .children(
            app_menu::owned_menu_items(menu)
                .enumerate()
                .map(|(item_index, item)| match item {
                    OwnedMenuItemLike::Separator => div()
                        .id(("menu-separator", index * 1000 + item_index))
                        .my(px(4.0))
                        .h(px(1.0))
                        .bg(Colors::border())
                        .into_any_element(),
                    OwnedMenuItemLike::Action {
                        name,
                        action,
                        checked,
                        disabled,
                    } => {
                        let action = action.boxed_clone();
                        div()
                            .id(("menu-item", index * 1000 + item_index))
                            .flex()
                            .items_center()
                            .gap(px(Spacing::CONTROL_GAP))
                            .px(px(Chrome::MENU_ITEM_PADDING_X))
                            .py(px(Chrome::MENU_ITEM_PADDING_Y))
                            .rounded(px(Chrome::BUTTON_RADIUS))
                            .text_sm()
                            .text_color(if disabled {
                                Colors::muted_foreground()
                            } else {
                                Colors::foreground()
                            })
                            .when(!disabled, |this| {
                                this.cursor_pointer()
                                    .hover(|style| style.bg(Colors::muted()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_menu = None;
                                        cx.notify();
                                        let action = action.boxed_clone();
                                        cx.defer(move |cx| {
                                            cx.dispatch_action(action.as_ref());
                                        });
                                    }))
                            })
                            .child(
                                div()
                                    .w(px(Chrome::MENU_ITEM_CHECK_WIDTH))
                                    .text_xs()
                                    .text_color(Colors::active())
                                    .child(if checked { "✓" } else { "" }),
                            )
                            .child(div().flex_1().child(name.to_string()))
                            .into_any_element()
                    }
                }),
        )
}
