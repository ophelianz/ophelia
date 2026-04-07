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

use std::time::Duration;

use gpui::{App, Image, ImageFormat, SharedString, Tray, TrayIntent, TrayMenuItem};
use rust_i18n::t;

use crate::app_actions;

const TRAY_ICON_BYTES: &[u8] = include_bytes!("../assets/icons/logo.svg");
const TRAY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const TRAY_REFRESH_INTERVAL_TICKS: u8 = 10;

pub fn init(cx: &mut App) {
    refresh(cx, true);

    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let mut ticks_until_refresh = TRAY_REFRESH_INTERVAL_TICKS;
        loop {
            cx.background_executor().timer(TRAY_POLL_INTERVAL).await;
            let _ = cx.update(|cx| {
                drain_tray_intents(cx);
                ticks_until_refresh = ticks_until_refresh.saturating_sub(1);
                if ticks_until_refresh == 0 {
                    refresh(cx, false);
                    ticks_until_refresh = TRAY_REFRESH_INTERVAL_TICKS;
                }
            });
        }
    })
    .detach();
}

pub fn refresh(cx: &mut App, force: bool) {
    if !cx.has_global::<app_actions::AppState>() {
        return;
    }

    let Some(downloads) = app_actions::downloads(cx) else {
        return;
    };
    let title = tray_title_for_speed(downloads.read(cx).download_speed_bps());

    let should_refresh = force || {
        let state = cx.global::<app_actions::AppState>();
        state.last_tray_title.as_ref() != title.as_ref()
    };
    if !should_refresh {
        return;
    }

    cx.global_mut::<app_actions::AppState>().last_tray_title = title.clone();
    cx.set_tray(build_tray(title));
}

fn build_tray(title: Option<String>) -> Tray {
    let mut tray = Tray::new()
        .tooltip(t!("app.name").to_string())
        .icon(Image::from_bytes(
            ImageFormat::Svg,
            TRAY_ICON_BYTES.to_vec(),
        ))
        .icon_template(true)
        .menu_items(build_menu());

    if let Some(title) = title {
        tray = tray.title(title);
    }

    tray
}

fn build_menu() -> Vec<TrayMenuItem> {
    vec![
        TrayMenuItem::Action {
            name: t!("menu.open_ophelia").to_string().into(),
            intent: TrayIntent::OpenMainWindow,
        },
        TrayMenuItem::Action {
            name: t!("menu.new_download").to_string().into(),
            intent: TrayIntent::OpenDownloadModal,
        },
        TrayMenuItem::Action {
            name: t!("menu.settings").to_string().into(),
            intent: TrayIntent::OpenSettings,
        },
        TrayMenuItem::Action {
            name: t!("menu.about").to_string().into(),
            intent: TrayIntent::OpenAbout,
        },
        TrayMenuItem::Separator,
        TrayMenuItem::Action {
            name: t!("menu.quit").to_string().into(),
            intent: TrayIntent::Quit,
        },
    ]
}

fn drain_tray_intents(cx: &mut App) {
    for intent in cx.take_tray_intents() {
        app_actions::handle_tray_intent(intent, cx);
    }
}

fn format_tray_title(speed_bps: u64) -> String {
    format!("\u{2193} {}", format_speed(speed_bps))
}

fn tray_title_for_speed(speed_bps: u64) -> Option<String> {
    (speed_bps > 0).then(|| format_tray_title(speed_bps))
}

fn format_speed(speed_bps: u64) -> SharedString {
    const KB: f64 = 1_000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;

    let speed = speed_bps as f64;
    if speed >= GB {
        format!("{:.1} GB/s", speed / GB).into()
    } else if speed >= MB {
        format!("{:.1} MB/s", speed / MB).into()
    } else if speed >= KB {
        format!("{:.1} KB/s", speed / KB).into()
    } else {
        format!("{speed_bps} B/s").into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_title_hides_zero_and_formats_human_speed() {
        assert_eq!(tray_title_for_speed(0), None);
        assert_eq!(tray_title_for_speed(512).as_deref(), Some("↓ 512 B/s"));
        assert_eq!(tray_title_for_speed(1_500).as_deref(), Some("↓ 1.5 KB/s"));
        assert_eq!(
            tray_title_for_speed(12_300_000).as_deref(),
            Some("↓ 12.3 MB/s")
        );
    }
}
