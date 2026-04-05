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

use gpui::{Action, App, KeyBinding, Menu, MenuItem, OsAction, OwnedMenu, SharedString, actions};
use rust_i18n::t;

use crate::ui::controls::text_field;

actions!(ophelia_menu, [OpenDownloadModal, OpenSettings, About, Quit]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-q", Quit, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("alt-f4", Quit, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-n", OpenDownloadModal, None),
    ]);
}

pub fn build_menus() -> Vec<Menu> {
    if cfg!(target_os = "macos") {
        vec![
            Menu {
                name: t!("app.name").to_string().into(),
                items: vec![
                    MenuItem::action(t!("menu.about").to_string(), About),
                    MenuItem::separator(),
                    MenuItem::action(t!("menu.settings").to_string(), OpenSettings),
                    MenuItem::separator(),
                    MenuItem::action(t!("menu.quit").to_string(), Quit),
                ],
            },
            Menu {
                name: t!("menu.file").to_string().into(),
                items: vec![MenuItem::action(
                    t!("menu.new_download").to_string(),
                    OpenDownloadModal,
                )],
            },
            edit_menu(),
            Menu {
                name: t!("menu.window").to_string().into(),
                items: vec![],
            },
            Menu {
                name: t!("menu.help").to_string().into(),
                items: vec![MenuItem::action(t!("menu.about").to_string(), About)],
            },
        ]
    } else {
        vec![
            Menu {
                name: t!("menu.file").to_string().into(),
                items: vec![
                    MenuItem::action(t!("menu.new_download").to_string(), OpenDownloadModal),
                    MenuItem::action(t!("menu.settings").to_string(), OpenSettings),
                    MenuItem::separator(),
                    MenuItem::action(t!("menu.quit").to_string(), Quit),
                ],
            },
            edit_menu(),
            Menu {
                name: t!("menu.window").to_string().into(),
                items: vec![MenuItem::action(
                    t!("menu.new_download").to_string(),
                    OpenDownloadModal,
                )],
            },
            Menu {
                name: t!("menu.help").to_string().into(),
                items: vec![MenuItem::action(t!("menu.about").to_string(), About)],
            },
        ]
    }
}

pub fn build_owned_menus() -> Vec<OwnedMenu> {
    build_menus().into_iter().map(Menu::owned).collect()
}

pub fn refresh(cx: &mut App) {
    cx.set_menus(build_menus());
}

pub enum OwnedMenuItemLike<'a> {
    Separator,
    Action {
        name: &'a str,
        action: &'a dyn Action,
        checked: bool,
        disabled: bool,
    },
}

pub fn owned_menu_items(menu: &OwnedMenu) -> impl Iterator<Item = OwnedMenuItemLike<'_>> {
    menu.items.iter().filter_map(|item| match item {
        gpui::OwnedMenuItem::Separator => Some(OwnedMenuItemLike::Separator),
        gpui::OwnedMenuItem::Action {
            name,
            action,
            checked,
            ..
        } => Some(OwnedMenuItemLike::Action {
            name,
            action: action.as_ref(),
            checked: *checked,
            disabled: false,
        }),
        _ => None,
    })
}

pub fn menu_label(menu: &OwnedMenu) -> SharedString {
    menu.name.clone()
}

fn edit_menu() -> Menu {
    Menu {
        name: t!("menu.edit").to_string().into(),
        items: vec![
            MenuItem::os_action(t!("menu.cut").to_string(), text_field::Cut, OsAction::Cut),
            MenuItem::os_action(
                t!("menu.copy").to_string(),
                text_field::Copy,
                OsAction::Copy,
            ),
            MenuItem::os_action(
                t!("menu.paste").to_string(),
                text_field::Paste,
                OsAction::Paste,
            ),
            MenuItem::separator(),
            MenuItem::os_action(
                t!("menu.select_all").to_string(),
                text_field::SelectAll,
                OsAction::SelectAll,
            ),
        ],
    }
}
