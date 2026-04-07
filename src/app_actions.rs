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

use gpui::{
    App, AppContext, ApplicationActivationPolicy, Bounds, Entity, Global, TrayIntent, WindowHandle,
    px, size,
};

use crate::app::Downloads;
use crate::app_menu;
use crate::platform;
use crate::tray;
use crate::views::main::main_window::MainWindow;
use crate::views::settings::{SettingsClosed, SettingsWindow};

pub struct AppState {
    pub(crate) downloads: Entity<Downloads>,
    main_window: Option<WindowHandle<MainWindow>>,
    settings_window: Option<WindowHandle<SettingsWindow>>,
    pub(crate) show_about: Entity<bool>,
    pub(crate) show_download_modal: Entity<bool>,
    pub(crate) last_tray_title: Option<String>,
}

impl Global for AppState {}

const SETTINGS_WINDOW_MIN_WIDTH: f32 = 760.0;
const SETTINGS_WINDOW_MIN_HEIGHT: f32 = 520.0;
const MAIN_WINDOW_MIN_WIDTH: f32 = 960.0;
const MAIN_WINDOW_MIN_HEIGHT: f32 = 620.0;

pub fn init(downloads: Entity<Downloads>, cx: &mut App) {
    let show_about = cx.new(|_| false);
    let show_download_modal = cx.new(|_| false);

    cx.set_global(AppState {
        downloads,
        main_window: None,
        settings_window: None,
        show_about,
        show_download_modal,
        last_tray_title: None,
    });

    cx.on_action(open_main_window);
    cx.on_action(open_settings);
    cx.on_action(open_about);
    cx.on_action(open_download_modal);
    cx.on_action(quit);
}

pub(crate) fn downloads(cx: &App) -> Option<Entity<Downloads>> {
    cx.has_global::<AppState>()
        .then(|| cx.global::<AppState>().downloads.clone())
}

pub(crate) fn ensure_main_window(cx: &mut App) -> Option<WindowHandle<MainWindow>> {
    if !cx.has_global::<AppState>() {
        return None;
    }

    set_shell_mode_for_main_window(true, cx);

    if let Some(main_window) = cx.global::<AppState>().main_window {
        if window_is_open(main_window, cx) {
            focus_main_window(main_window, cx);
            cx.activate(true);
            return Some(main_window);
        }

        cx.global_mut::<AppState>().main_window = None;
    }

    if let Some(main_window) = find_open_main_window(cx) {
        cx.global_mut::<AppState>().main_window = Some(main_window);
        focus_main_window(main_window, cx);
        cx.activate(true);
        return Some(main_window);
    }

    let downloads = cx.global::<AppState>().downloads.clone();
    let bounds = Bounds::centered(None, size(px(1280.), px(720.)), cx);
    let Ok(main_window) = cx.open_window(
        platform::window_options(
            bounds,
            size(px(MAIN_WINDOW_MIN_WIDTH), px(MAIN_WINDOW_MIN_HEIGHT)),
        ),
        |_, cx| cx.new(|cx| MainWindow::new(downloads.clone(), cx)),
    ) else {
        return None;
    };

    cx.global_mut::<AppState>().main_window = Some(main_window);
    cx.activate(true);
    Some(main_window)
}

pub(crate) fn handle_main_window_close(cx: &mut App) {
    if !cx.has_global::<AppState>() {
        return;
    }

    let (settings_window, show_about, show_download_modal) = {
        let state = cx.global::<AppState>();
        (
            state.settings_window,
            state.show_about.clone(),
            state.show_download_modal.clone(),
        )
    };

    if let Some(settings_window) = settings_window {
        let _ = settings_window.update(cx, |_, window, _| {
            window.remove_window();
        });
    }

    {
        let state = cx.global_mut::<AppState>();
        state.main_window = None;
        state.settings_window = None;
    }

    show_about.update(cx, |show, cx| {
        *show = false;
        cx.notify();
    });
    show_download_modal.update(cx, |show, cx| {
        *show = false;
        cx.notify();
    });
    let show_about_after_close = show_about.clone();
    let show_download_modal_after_close = show_download_modal.clone();
    cx.defer(move |cx| {
        show_about_after_close.update(cx, |show, cx| {
            *show = false;
            cx.notify();
        });
        show_download_modal_after_close.update(cx, |show, cx| {
            *show = false;
            cx.notify();
        });
    });

    tray::refresh(cx, true);
    set_shell_mode_for_main_window(false, cx);
}

fn open_main_window(_: &app_menu::OpenMainWindow, cx: &mut App) {
    open_main_window_impl(cx);
}

fn open_settings(_: &app_menu::OpenSettings, cx: &mut App) {
    open_settings_impl(cx);
}

fn open_settings_impl(cx: &mut App) {
    if !cx.has_global::<AppState>() {
        return;
    }

    if let Some(settings_window) = cx.global::<AppState>().settings_window {
        if settings_window_is_open(settings_window, cx) {
            focus_settings_window(settings_window, cx);
            return;
        }

        cx.global_mut::<AppState>().settings_window = None;
    }

    if let Some(settings_window) = find_open_settings_window(cx) {
        cx.global_mut::<AppState>().settings_window = Some(settings_window);
        focus_settings_window(settings_window, cx);
        return;
    }

    let Some(main_window) = ensure_main_window(cx) else {
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
            tray::refresh(cx, true);

            if cx.has_global::<AppState>() {
                cx.global_mut::<AppState>().settings_window = None;
            }
        });
        subscription.detach();
    }

    cx.global_mut::<AppState>().settings_window = Some(settings_window);
    cx.defer(move |cx| {
        let _ = settings_window.update(cx, |_, window, _| {
            window.activate_window();
        });
    });
}

fn open_about(_: &app_menu::About, cx: &mut App) {
    open_about_impl(cx);
}

fn open_download_modal(_: &app_menu::OpenDownloadModal, cx: &mut App) {
    open_download_modal_impl(cx);
}

fn quit(_: &app_menu::Quit, cx: &mut App) {
    quit_impl(cx);
}

pub(crate) fn handle_tray_intent(intent: TrayIntent, cx: &mut App) {
    match intent {
        TrayIntent::OpenMainWindow => open_main_window_impl(cx),
        TrayIntent::OpenDownloadModal => open_download_modal_impl(cx),
        TrayIntent::OpenSettings => open_settings_impl(cx),
        TrayIntent::OpenAbout => open_about_impl(cx),
        TrayIntent::Quit => quit_impl(cx),
    }
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

fn find_open_main_window(cx: &App) -> Option<WindowHandle<MainWindow>> {
    cx.windows()
        .into_iter()
        .filter_map(|window| window.downcast::<MainWindow>())
        .find(|window| window_is_open(*window, cx))
}

fn find_open_settings_window(cx: &App) -> Option<WindowHandle<SettingsWindow>> {
    cx.windows()
        .into_iter()
        .filter_map(|window| window.downcast::<SettingsWindow>())
        .find(|window| settings_window_is_open(*window, cx))
}

fn set_shell_mode_for_main_window(has_main_window: bool, cx: &App) {
    let policy = if has_main_window {
        ApplicationActivationPolicy::Regular
    } else {
        ApplicationActivationPolicy::Accessory
    };
    cx.set_activation_policy(policy);
}

fn window_is_open(window: WindowHandle<MainWindow>, cx: &App) -> bool {
    cx.windows()
        .into_iter()
        .any(|candidate| candidate == window.into())
}

fn settings_window_is_open(window: WindowHandle<SettingsWindow>, cx: &App) -> bool {
    cx.windows()
        .into_iter()
        .any(|candidate| candidate == window.into())
}

fn focus_main_window(window: WindowHandle<MainWindow>, cx: &mut App) {
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, _| {
            window.activate_window();
        });
    });
}

fn focus_settings_window(window: WindowHandle<SettingsWindow>, cx: &mut App) {
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, _| {
            window.activate_window();
        });
    });
}

fn open_main_window_impl(cx: &mut App) {
    let _ = ensure_main_window(cx);
}

fn open_about_impl(cx: &mut App) {
    let _ = ensure_main_window(cx);
    cx.defer(|cx| {
        set_transient_visibility(cx, |state| state.show_about.clone(), true);
    });
}

fn open_download_modal_impl(cx: &mut App) {
    let _ = ensure_main_window(cx);
    cx.defer(|cx| {
        set_transient_visibility(cx, |state| state.show_download_modal.clone(), true);
    });
}

fn quit_impl(cx: &mut App) {
    cx.quit();
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AnyWindowHandle, TestApp};

    fn setup_app() -> TestApp {
        let mut app = TestApp::new();
        app.update(|cx| {
            let downloads = cx.new(Downloads::new);
            init(downloads, cx);
        });
        app
    }

    #[test]
    fn ensure_main_window_opens_once_and_reuses_existing_window() {
        let mut app = setup_app();

        let first = app
            .update(ensure_main_window)
            .map(AnyWindowHandle::from)
            .expect("main window should open");
        let second = app
            .update(ensure_main_window)
            .map(AnyWindowHandle::from)
            .expect("main window should be reused");

        assert_eq!(app.windows().len(), 1);
        assert_eq!(first.window_id(), second.window_id());
        app.read_global::<AppState, _>(|state, _| {
            assert!(state.main_window.is_some());
        });
    }

    #[test]
    fn ensure_main_window_recovers_existing_window_when_tracked_handle_is_missing() {
        let mut app = setup_app();

        let original = app
            .update(ensure_main_window)
            .map(AnyWindowHandle::from)
            .expect("main window should open");

        app.update(|cx| {
            cx.global_mut::<AppState>().main_window = None;
        });

        let reopened = app
            .update(ensure_main_window)
            .map(AnyWindowHandle::from)
            .expect("existing main window should be rediscovered");

        assert_eq!(app.windows().len(), 1);
        assert_eq!(original.window_id(), reopened.window_id());
    }

    #[test]
    fn tray_actions_open_main_window_before_overlay_or_settings() {
        let mut app = setup_app();

        app.update(|cx| open_about(&app_menu::About, cx));
        assert_eq!(app.windows().len(), 1);
        app.read_global::<AppState, _>(|state, cx| {
            assert!(state.main_window.is_some());
            assert!(*state.show_about.read(cx));
        });

        app.update(|cx| open_download_modal(&app_menu::OpenDownloadModal, cx));
        app.read_global::<AppState, _>(|state, cx| {
            assert!(state.main_window.is_some());
            assert!(*state.show_download_modal.read(cx));
        });

        app.update(|cx| open_settings(&app_menu::OpenSettings, cx));
        assert_eq!(app.windows().len(), 2);
        app.read_global::<AppState, _>(|state, _| {
            assert!(state.main_window.is_some());
            assert!(state.settings_window.is_some());
        });
    }

    #[test]
    fn tray_intents_reuse_main_window_for_modal_and_settings() {
        let mut app = setup_app();

        app.update(|cx| handle_tray_intent(TrayIntent::OpenDownloadModal, cx));
        assert_eq!(app.windows().len(), 1);
        app.read_global::<AppState, _>(|state, cx| {
            assert!(state.main_window.is_some());
            assert!(*state.show_download_modal.read(cx));
        });

        app.update(|cx| handle_tray_intent(TrayIntent::OpenDownloadModal, cx));
        assert_eq!(app.windows().len(), 1);

        app.update(|cx| handle_tray_intent(TrayIntent::OpenSettings, cx));
        assert_eq!(app.windows().len(), 2);
        app.read_global::<AppState, _>(|state, _| {
            assert!(state.main_window.is_some());
            assert!(state.settings_window.is_some());
        });

        app.update(|cx| handle_tray_intent(TrayIntent::OpenSettings, cx));
        assert_eq!(app.windows().len(), 2);
    }

    #[test]
    fn handle_main_window_close_clears_handles_and_transient_visibility() {
        let mut app = setup_app();

        app.update(|cx| {
            open_about(&app_menu::About, cx);
            open_download_modal(&app_menu::OpenDownloadModal, cx);
            open_settings(&app_menu::OpenSettings, cx);
            handle_main_window_close(cx);
        });

        app.read_global::<AppState, _>(|state, cx| {
            assert!(state.main_window.is_none());
            assert!(state.settings_window.is_none());
            assert!(!*state.show_about.read(cx));
            assert!(!*state.show_download_modal.read(cx));
        });
    }
}
