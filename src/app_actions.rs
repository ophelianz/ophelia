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

use gpui::{App, AppContext, Bounds, Entity, Global, WindowHandle, px, size};

use crate::app_menu;
use crate::platform;
use crate::views::main::main_window::MainWindow;
use crate::views::settings::{SettingsClosed, SettingsWindow};

pub struct AppState {
    main_window: Option<WindowHandle<MainWindow>>,
    settings_window: Option<WindowHandle<SettingsWindow>>,
    pub(crate) show_about: Entity<bool>,
    pub(crate) show_download_modal: Entity<bool>,
}

impl Global for AppState {}

const SETTINGS_WINDOW_MIN_WIDTH: f32 = 760.0;
const SETTINGS_WINDOW_MIN_HEIGHT: f32 = 520.0;

pub fn init(cx: &mut App) {
    let show_about = cx.new(|_| false);
    let show_download_modal = cx.new(|_| false);

    cx.set_global(AppState {
        main_window: None,
        settings_window: None,
        show_about,
        show_download_modal,
    });

    cx.on_action(open_settings);
    cx.on_action(open_about);
    cx.on_action(open_download_modal);
    cx.on_action(quit);
}

pub fn set_main_window(main_window: WindowHandle<MainWindow>, cx: &mut App) {
    if cx.has_global::<AppState>() {
        cx.global_mut::<AppState>().main_window = Some(main_window);
    }
}

fn open_settings(_: &app_menu::OpenSettings, cx: &mut App) {
    if !cx.has_global::<AppState>() {
        return;
    }

    if let Some(settings_window) = cx.global::<AppState>().settings_window {
        if settings_window
            .update(cx, |_, window, _| {
                window.activate_window();
            })
            .is_ok()
        {
            return;
        }

        cx.global_mut::<AppState>().settings_window = None;
    }

    let Some(main_window) = main_window(cx) else {
        return;
    };

    let bounds = Bounds::centered(None, size(px(1280.), px(600.)), cx);
    let Ok(settings_window) = cx.open_window(
        platform::window_options(
            bounds,
            size(
                px(SETTINGS_WINDOW_MIN_WIDTH),
                px(SETTINGS_WINDOW_MIN_HEIGHT),
            ),
        ),
        |_, cx| cx.new(|cx| SettingsWindow::new(cx)),
    ) else {
        return;
    };

    if let Ok(entity) = settings_window.entity(cx) {
        let subscription = cx.subscribe(&entity, move |_, event: &SettingsClosed, cx| {
            rust_i18n::set_locale(event.settings.resolved_language());
            app_menu::refresh(cx);
            let _ = main_window.update(cx, |this, _, cx| {
                this.apply_settings(event.settings.clone(), cx);
            });

            if cx.has_global::<AppState>() {
                cx.global_mut::<AppState>().settings_window = None;
            }
        });
        subscription.detach();
    }

    cx.global_mut::<AppState>().settings_window = Some(settings_window);
}

fn open_about(_: &app_menu::About, cx: &mut App) {
    set_transient_visibility(cx, |state| state.show_about.clone(), true);
}

fn open_download_modal(_: &app_menu::OpenDownloadModal, cx: &mut App) {
    set_transient_visibility(cx, |state| state.show_download_modal.clone(), true);
}

fn quit(_: &app_menu::Quit, cx: &mut App) {
    cx.quit();
}

fn main_window(cx: &App) -> Option<WindowHandle<MainWindow>> {
    cx.has_global::<AppState>()
        .then(|| cx.global::<AppState>().main_window)
        .flatten()
}

fn set_transient_visibility(
    cx: &mut App,
    entity: impl FnOnce(&AppState) -> Entity<bool>,
    visible: bool,
) {
    if !cx.has_global::<AppState>() {
        return;
    }

    let show = entity(cx.global::<AppState>());
    show.update(cx, |show, cx| {
        *show = visible;
        cx.notify();
    });
}
