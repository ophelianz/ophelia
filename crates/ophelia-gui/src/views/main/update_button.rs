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

use gpui::{App, IntoElement, RenderOnce, Window, div, prelude::*, px};
use rust_i18n::t;

use crate::ui::prelude::*;
use crate::updater::AutoUpdaterStatus;

#[derive(IntoElement)]
pub struct UpdateHeaderButton {
    status: AutoUpdaterStatus,
}

impl UpdateHeaderButton {
    pub fn new(status: AutoUpdaterStatus) -> Self {
        Self { status }
    }

    pub fn should_render(status: &AutoUpdaterStatus) -> bool {
        !matches!(
            status,
            AutoUpdaterStatus::Idle | AutoUpdaterStatus::Available { .. }
        )
    }
}

impl RenderOnce for UpdateHeaderButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let model = UpdateButtonModel::from_status(&self.status);
        div()
            .id("window-header-update-button")
            .flex()
            .items_center()
            .gap(px(Spacing::SETTINGS_INLINE_GAP))
            .h(px(Chrome::BUTTON_HEIGHT))
            .px(px(Chrome::BUTTON_PADDING_X))
            .rounded(px(Chrome::BUTTON_RADIUS))
            .border_1()
            .border_color(model.border_color)
            .bg(model.background)
            .text_sm()
            .font_weight(gpui::FontWeight::LIGHT)
            .text_color(model.text_color)
            .when(model.clickable, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(Colors::card_hover()))
                    .active(|style| style.bg(Colors::card()))
                    .on_click(|_, _, cx| {
                        crate::updater::perform_primary_action(cx);
                    })
            })
            .child(
                ProgressCircle::new("update-progress-circle")
                    .size(16.0)
                    .thickness(1.6)
                    .loading(model.loading)
                    .value(model.progress)
                    .color(model.progress_color)
                    .track_color(model.track_color),
            )
            .child(div().child(model.label))
    }
}

struct UpdateButtonModel {
    label: String,
    progress: f32,
    loading: bool,
    clickable: bool,
    background: gpui::Rgba,
    border_color: gpui::Rgba,
    text_color: gpui::Rgba,
    progress_color: gpui::Rgba,
    track_color: gpui::Rgba,
}

impl UpdateButtonModel {
    fn from_status(status: &AutoUpdaterStatus) -> Self {
        match status {
            AutoUpdaterStatus::Checking => Self {
                label: t!("updates.checking").to_string(),
                progress: 0.2,
                loading: true,
                clickable: false,
                background: Colors::muted(),
                border_color: Colors::input_border(),
                text_color: Colors::foreground(),
                progress_color: Colors::foreground(),
                track_color: Colors::border(),
            },
            AutoUpdaterStatus::Downloading { progress, .. } => Self {
                label: t!("updates.downloading").to_string(),
                progress: *progress,
                loading: false,
                clickable: false,
                background: Colors::active(),
                border_color: gpui::rgba(0x00000000),
                text_color: Colors::background(),
                progress_color: Colors::background(),
                track_color: gpui::rgba(0x01020233),
            },
            AutoUpdaterStatus::Verifying { .. } => Self {
                label: t!("updates.verifying").to_string(),
                progress: 0.75,
                loading: true,
                clickable: false,
                background: Colors::muted(),
                border_color: Colors::input_border(),
                text_color: Colors::foreground(),
                progress_color: Colors::foreground(),
                track_color: Colors::border(),
            },
            AutoUpdaterStatus::ReadyToInstall { .. } => Self {
                label: t!("updates.install").to_string(),
                progress: 1.0,
                loading: false,
                clickable: true,
                background: Colors::active(),
                border_color: gpui::rgba(0x00000000),
                text_color: Colors::background(),
                progress_color: Colors::background(),
                track_color: gpui::rgba(0x01020233),
            },
            AutoUpdaterStatus::Installing { .. } => Self {
                label: t!("updates.installing").to_string(),
                progress: 0.85,
                loading: true,
                clickable: false,
                background: Colors::muted(),
                border_color: Colors::input_border(),
                text_color: Colors::foreground(),
                progress_color: Colors::foreground(),
                track_color: Colors::border(),
            },
            AutoUpdaterStatus::Updated { .. } => Self {
                label: t!("updates.restart").to_string(),
                progress: 1.0,
                loading: false,
                clickable: true,
                background: Colors::active(),
                border_color: gpui::rgba(0x00000000),
                text_color: Colors::background(),
                progress_color: Colors::background(),
                track_color: gpui::rgba(0x01020233),
            },
            AutoUpdaterStatus::Errored { .. } => Self {
                label: t!("updates.retry").to_string(),
                progress: 1.0,
                loading: false,
                clickable: true,
                background: Colors::muted(),
                border_color: Colors::error(),
                text_color: Colors::foreground(),
                progress_color: Colors::error(),
                track_color: Colors::border(),
            },
            AutoUpdaterStatus::Idle | AutoUpdaterStatus::Available { .. } => Self {
                label: String::new(),
                progress: 0.0,
                loading: false,
                clickable: false,
                background: Colors::muted(),
                border_color: Colors::input_border(),
                text_color: Colors::foreground(),
                progress_color: Colors::foreground(),
                track_color: Colors::border(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::UpdateChannel;
    use crate::updater::{AvailableRelease, UpdateAsset};

    fn sample_release() -> AvailableRelease {
        AvailableRelease {
            channel: UpdateChannel::Nightly,
            version: "1.0.1".into(),
            pub_date: "2026-04-08T18:00:00Z".into(),
            commit: None,
            notes_url: None,
            asset: UpdateAsset {
                url: String::new(),
                size: 0,
                sha256: String::new(),
                minisign_url: String::new(),
            },
        }
    }

    #[test]
    fn update_button_model_maps_ready_and_restart_states() {
        let ready = UpdateButtonModel::from_status(&AutoUpdaterStatus::ReadyToInstall {
            release: sample_release(),
            archive_path: "/tmp/Ophelia.zip".into(),
            working_dir: "/tmp/ophelia".into(),
        });
        let updated = UpdateButtonModel::from_status(&AutoUpdaterStatus::Updated {
            release: sample_release(),
            staged_app_path: "/tmp/Ophelia.app".into(),
            working_dir: "/tmp/ophelia".into(),
        });

        assert_eq!(ready.label, "Install Update");
        assert_eq!(updated.label, "Restart to Update");
        assert!(ready.clickable);
        assert!(updated.clickable);
    }
}
