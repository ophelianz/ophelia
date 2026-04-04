use gpui::{App, Entity, Window, div, prelude::*, px};

use crate::app::Downloads;
use crate::engine::DownloadStatus;
use crate::ui::prelude::*;
use crate::views::download_row::{DownloadRow, DownloadState};

use rust_i18n::t;

pub struct DownloadList {
    downloads: Entity<Downloads>,
}

impl DownloadList {
    pub fn new(downloads: Entity<Downloads>, cx: &mut Context<Self>) -> Self {
        cx.observe(&downloads, |_, _, cx| cx.notify()).detach();
        Self { downloads }
    }

    fn view_model(&self, cx: &App) -> DownloadListViewModel {
        let entity = self.downloads.clone();
        let downloads = self.downloads.read(cx);

        let rows = (0..downloads.len())
            .map(|i| {
                let id = downloads.ids[i];
                let progress = match downloads.total_bytes[i] {
                    Some(total) if total > 0 => downloads.downloaded_bytes[i] as f32 / total as f32,
                    _ => 0.0,
                };
                let state = match downloads.statuses[i] {
                    DownloadStatus::Downloading => DownloadState::Active,
                    DownloadStatus::Paused => DownloadState::Paused,
                    DownloadStatus::Finished => DownloadState::Finished,
                    DownloadStatus::Error => DownloadState::Error,
                    _ => DownloadState::Queued,
                };

                let on_pause_resume: Option<Box<dyn Fn(&mut Window, &mut App) + 'static>> =
                    match state {
                        DownloadState::Active | DownloadState::Queued => {
                            let entity = entity.clone();
                            Some(Box::new(move |_, cx| {
                                entity.update(cx, |downloads, cx| downloads.pause(id, cx));
                            }))
                        }
                        DownloadState::Paused => {
                            let entity = entity.clone();
                            Some(Box::new(move |_, cx| {
                                entity.update(cx, |downloads, cx| downloads.resume(id, cx));
                            }))
                        }
                        DownloadState::Finished | DownloadState::Error => None,
                    };

                let on_remove: Box<dyn Fn(&mut Window, &mut App) + 'static> = {
                    let entity = entity.clone();
                    Box::new(move |_, cx| {
                        entity.update(cx, |downloads, cx| downloads.remove(id, cx));
                    })
                };

                DownloadRow {
                    id,
                    filename: downloads.filenames[i].clone(),
                    destination: downloads.destinations[i].clone(),
                    progress,
                    speed: format_speed(downloads.speeds[i]).into(),
                    state,
                    on_pause_resume,
                    on_remove: Some(on_remove),
                }
            })
            .collect();

        DownloadListViewModel { rows }
    }
}

impl Render for DownloadList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view_model = self.view_model(cx);

        v_flex()
            .child(
                div()
                    .text_sm()
                    .text_color(Colors::muted_foreground())
                    .font_weight(gpui::FontWeight::EXTRA_BOLD)
                    .mb(px(Spacing::SECTION_LABEL_BOTTOM_MARGIN))
                    .child(t!("downloads.section_label").to_string()),
            )
            .child(
                v_flex()
                    .gap(px(Spacing::LIST_GAP))
                    .children(view_model.rows),
            )
    }
}

struct DownloadListViewModel {
    rows: Vec<DownloadRow>,
}

fn format_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec == 0 {
        return String::new();
    }
    const MB: u64 = 1_000_000;
    const KB: u64 = 1_000;
    if bytes_per_sec >= MB {
        format!("{:.1} MB/s", bytes_per_sec as f64 / MB as f64)
    } else if bytes_per_sec >= KB {
        format!("{:.0} KB/s", bytes_per_sec as f64 / KB as f64)
    } else {
        format!("{} B/s", bytes_per_sec)
    }
}
