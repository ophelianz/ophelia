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
    App, Context, Entity, IntoElement, Render, RenderOnce, SharedString, UniformListScrollHandle,
    Window, div, prelude::*, px, uniform_list,
};
use std::path::Path;
use std::rc::Rc;

use crate::app::{Downloads, TransferListRow};
use crate::engine::{DownloadId, DownloadStatus};
use crate::settings::{Settings, suggested_destination_rule_icon_name};
use crate::ui::prelude::*;
use crate::views::main::transfer_row::TransferRow;
use crate::views::main::transfer_row::default_transfer_icon_name_for_filename;

use rust_i18n::t;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransferFilter {
    All,
    Active,
    Finished,
    Paused,
    Failed,
}

impl TransferFilter {
    fn matches(self, status: DownloadStatus) -> bool {
        match self {
            Self::All => true,
            Self::Active => matches!(
                status,
                DownloadStatus::Downloading | DownloadStatus::Pending
            ),
            Self::Finished => status == DownloadStatus::Finished,
            Self::Paused => status == DownloadStatus::Paused,
            Self::Failed => matches!(status, DownloadStatus::Error | DownloadStatus::Cancelled),
        }
    }
}

pub struct TransferList {
    downloads: Entity<Downloads>,
    filter: TransferFilter,
    selected_id: Option<DownloadId>,
    scroll_handle: UniformListScrollHandle,
}

pub struct TransferListSelectionChanged {
    pub id: Option<DownloadId>,
}

impl gpui::EventEmitter<TransferListSelectionChanged> for TransferList {}

impl TransferList {
    pub fn new(downloads: Entity<Downloads>, cx: &mut Context<Self>) -> Self {
        cx.observe(&downloads, |_, _, cx| cx.notify()).detach();
        Self {
            downloads,
            filter: TransferFilter::All,
            selected_id: None,
            scroll_handle: UniformListScrollHandle::new(),
        }
    }

    fn view_model(
        &self,
        rows: Vec<TransferListRow>,
        selected_id: Option<DownloadId>,
        settings: &Settings,
    ) -> TransferListViewModel {
        let downloads = self.downloads.clone();
        let filters = vec![
            TransferFilterChipModel::new(
                0,
                TransferFilter::All,
                t!("transfers.filter_all").to_string(),
                self.filter == TransferFilter::All,
            ),
            TransferFilterChipModel::new(
                1,
                TransferFilter::Active,
                t!("transfers.filter_active").to_string(),
                self.filter == TransferFilter::Active,
            ),
            TransferFilterChipModel::new(
                2,
                TransferFilter::Finished,
                t!("transfers.filter_finished").to_string(),
                self.filter == TransferFilter::Finished,
            ),
            TransferFilterChipModel::new(
                3,
                TransferFilter::Paused,
                t!("transfers.filter_paused").to_string(),
                self.filter == TransferFilter::Paused,
            ),
            TransferFilterChipModel::new(
                4,
                TransferFilter::Failed,
                t!("transfers.filter_failed").to_string(),
                self.filter == TransferFilter::Failed,
            ),
        ];

        let rows = rows
            .into_iter()
            .map(|row| {
                let id = row.id;
                let selected = selected_id == Some(id);
                let icon_name = resolved_transfer_icon_name(&row, settings);
                let on_pause_resume: Option<Rc<dyn Fn(&mut Window, &mut App) + 'static>> =
                    if row.available_actions.pause {
                        let downloads = downloads.clone();
                        Some(Rc::new(move |_window: &mut Window, app: &mut App| {
                            downloads.update(app, |downloads, cx| downloads.pause(id, cx));
                        }))
                    } else if row.available_actions.resume {
                        let downloads = downloads.clone();
                        Some(Rc::new(move |_window: &mut Window, app: &mut App| {
                            downloads.update(app, |downloads, cx| downloads.resume(id, cx));
                        }))
                    } else {
                        None
                    };

                let on_remove = if row.available_actions.delete_artifact {
                    let downloads = downloads.clone();
                    Some(Rc::new(move |_window: &mut Window, app: &mut App| {
                        downloads.update(app, |downloads, cx| downloads.remove(id, cx));
                    })
                        as Rc<dyn Fn(&mut Window, &mut App) + 'static>)
                } else {
                    None
                };

                TransferRowModel {
                    id,
                    filename: row.filename,
                    destination: row.destination,
                    icon_name: icon_name.into(),
                    downloaded_bytes: row.downloaded_bytes,
                    total_bytes: row.total_bytes,
                    progress: row.progress,
                    state: row.display_state,
                    selected,
                    on_pause_resume,
                    on_remove,
                }
            })
            .collect();

        TransferListViewModel { filters, rows }
    }

    pub fn visible_transfer_rows(&self, cx: &App) -> Vec<TransferListRow> {
        self.downloads
            .read(cx)
            .transfer_rows()
            .into_iter()
            .filter(|row| self.filter.matches(row.status))
            .collect()
    }

    fn set_selected_id(&mut self, selected_id: Option<DownloadId>, cx: &mut Context<Self>) {
        if self.selected_id == selected_id {
            return;
        }

        self.selected_id = selected_id;
        cx.emit(TransferListSelectionChanged { id: selected_id });
        cx.notify();
    }
}

impl Render for TransferList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (rows, settings) = {
            let downloads = self.downloads.read(cx);
            (
                downloads
                    .transfer_rows()
                    .into_iter()
                    .filter(|row| self.filter.matches(row.status))
                    .collect::<Vec<_>>(),
                downloads.settings.clone(),
            )
        };
        let selected_id = resolve_selected_transfer_id(&rows, self.selected_id);
        if selected_id != self.selected_id {
            self.selected_id = selected_id;
            cx.emit(TransferListSelectionChanged { id: selected_id });
        }

        let view_model = self.view_model(rows, selected_id, &settings);
        let weak = cx.weak_entity();
        let rows_empty = view_model.rows.is_empty();
        let row_models = Rc::new(view_model.rows);
        let scroll_handle = self.scroll_handle.clone();

        v_flex()
            .size_full()
            .min_h_0()
            .pt(px(Spacing::SECTION_GAP))
            .child(
                div()
                    .text_sm()
                    .text_color(Colors::muted_foreground())
                    .font_weight(gpui::FontWeight::EXTRA_BOLD)
                    .mb(px(Spacing::SECTION_LABEL_BOTTOM_MARGIN))
                    .child(t!("transfers.section_label").to_string()),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(Chrome::MENU_BAR_GAP))
                    .mb(px(Spacing::SECTION_GAP))
                    .children(view_model.filters.into_iter().map(|filter_model| {
                        let filter = filter_model.filter;
                        FilterChip::new(
                            ("transfer-filter", filter_model.id),
                            filter_model.label,
                            filter_model.active,
                        )
                        .on_click({
                            let weak = weak.clone();
                            move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.filter = filter;
                                    cx.notify();
                                });
                            }
                        })
                        .into_any_element()
                    })),
            )
            .child(if rows_empty {
                TransferListEmptyState.into_any_element()
            } else {
                uniform_list(
                    "transfers-rows",
                    row_models.len(),
                    cx.processor(move |_, range: std::ops::Range<usize>, _window, _cx| {
                        range
                            .map(|ix| {
                                let model = row_models[ix].clone();
                                let id = model.id;
                                let weak = weak.clone();
                                TransferRow {
                                    id: model.id,
                                    filename: model.filename,
                                    destination: model.destination,
                                    icon_name: model.icon_name,
                                    downloaded_bytes: model.downloaded_bytes,
                                    total_bytes: model.total_bytes,
                                    progress: model.progress,
                                    state: model.state,
                                    selected: model.selected,
                                    on_select: Some(Box::new(
                                        move |_window: &mut Window, app: &mut App| {
                                            let _ = weak.update(app, |this, cx| {
                                                this.set_selected_id(Some(id), cx);
                                            });
                                        },
                                    )),
                                    on_pause_resume: model.on_pause_resume.as_ref().map({
                                        |handler| {
                                            let handler = Rc::clone(handler);
                                            Box::new(move |window: &mut Window, cx: &mut App| {
                                                handler(window, cx);
                                            })
                                                as Box<dyn Fn(&mut Window, &mut App) + 'static>
                                        }
                                    }),
                                    on_remove: model.on_remove.as_ref().map(|handler| {
                                        let handler = Rc::clone(handler);
                                        Box::new(move |window: &mut Window, cx: &mut App| {
                                            handler(window, cx);
                                        })
                                            as Box<dyn Fn(&mut Window, &mut App) + 'static>
                                    }),
                                }
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&scroll_handle)
                .flex_1()
                .min_h_0()
                .w_full()
                .pb(px(Spacing::LIST_GAP))
                .with_sizing_behavior(gpui::ListSizingBehavior::Auto)
                .with_width_from_item(Some(0))
                .into_any_element()
            })
    }
}

struct TransferListViewModel {
    filters: Vec<TransferFilterChipModel>,
    rows: Vec<TransferRowModel>,
}

#[derive(Clone)]
struct TransferRowModel {
    id: DownloadId,
    filename: SharedString,
    destination: SharedString,
    icon_name: SharedString,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    progress: f32,
    state: crate::app::TransferDisplayState,
    selected: bool,
    on_pause_resume: Option<Rc<dyn Fn(&mut Window, &mut App) + 'static>>,
    on_remove: Option<Rc<dyn Fn(&mut Window, &mut App) + 'static>>,
}

#[derive(Clone)]
struct TransferFilterChipModel {
    id: usize,
    filter: TransferFilter,
    label: SharedString,
    active: bool,
}

impl TransferFilterChipModel {
    fn new(
        id: usize,
        filter: TransferFilter,
        label: impl Into<SharedString>,
        active: bool,
    ) -> Self {
        Self {
            id,
            filter,
            label: label.into(),
            active,
        }
    }
}

#[derive(IntoElement)]
struct TransferListEmptyState;

impl RenderOnce for TransferListEmptyState {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(Colors::muted_foreground())
            .child(t!("transfers.empty_state").to_string())
    }
}

fn resolve_selected_transfer_id(
    rows: &[TransferListRow],
    selected_id: Option<DownloadId>,
) -> Option<DownloadId> {
    match selected_id {
        Some(selected_id) if rows.iter().any(|row| row.id == selected_id) => Some(selected_id),
        _ => rows.first().map(|row| row.id),
    }
}

fn resolved_transfer_icon_name(row: &TransferListRow, settings: &Settings) -> String {
    destination_rule_icon_name_for_transfer(row, settings).unwrap_or_else(|| {
        default_transfer_icon_name_for_filename(row.filename.as_ref()).to_string()
    })
}

fn destination_rule_icon_name_for_transfer(
    row: &TransferListRow,
    settings: &Settings,
) -> Option<String> {
    if !settings.destination_rules_enabled {
        return None;
    }

    let destination_dir = Path::new(row.destination.as_ref()).parent()?;
    let filename_extension = normalized_filename_extension(row.filename.as_ref())?;

    settings
        .destination_rules
        .iter()
        .find(|rule| {
            rule.enabled
                && destination_dir == rule.target_dir.as_path()
                && rule
                    .extensions
                    .iter()
                    .filter_map(|extension| normalized_rule_extension(extension))
                    .any(|extension| extension == filename_extension)
        })
        .map(|rule| {
            rule.icon_name
                .as_deref()
                .unwrap_or_else(|| {
                    suggested_destination_rule_icon_name(&rule.label, &rule.extensions)
                })
                .to_string()
        })
}

fn normalized_filename_extension(filename: &str) -> Option<String> {
    Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{}", extension.to_ascii_lowercase()))
}

fn normalized_rule_extension(extension: &str) -> Option<String> {
    let trimmed = extension.trim();
    if trimmed.is_empty() {
        None
    } else if trimmed.starts_with('.') {
        Some(trimmed.to_ascii_lowercase())
    } else {
        Some(format!(".{}", trimmed.to_ascii_lowercase()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{TransferAvailableActions, TransferDisplayState};
    use crate::settings::DestinationRule;
    use gpui::{
        Bounds, Context, MouseButton, Pixels, TestApp, Window, WindowBounds, WindowOptions, point,
        px, size,
    };
    use std::ops::Range;
    use std::path::PathBuf;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    const PROBE_ROW_HEIGHT: f32 = 88.0;
    const PROBE_ITEM_COUNT: usize = 200;
    const PROBE_TOP_HEIGHT: f32 = 220.0;

    #[test]
    fn selection_defaults_to_first_visible_row() {
        let rows = vec![test_row(DownloadId(3)), test_row(DownloadId(7))];

        assert_eq!(
            resolve_selected_transfer_id(&rows, None),
            Some(DownloadId(3))
        );
    }

    #[test]
    fn selection_preserves_current_row_when_still_visible() {
        let rows = vec![test_row(DownloadId(3)), test_row(DownloadId(7))];

        assert_eq!(
            resolve_selected_transfer_id(&rows, Some(DownloadId(7))),
            Some(DownloadId(7))
        );
    }

    #[test]
    fn selection_falls_back_when_row_is_removed_or_filtered_out() {
        let rows = vec![test_row(DownloadId(11)), test_row(DownloadId(14))];

        assert_eq!(
            resolve_selected_transfer_id(&rows, Some(DownloadId(99))),
            Some(DownloadId(11))
        );
    }

    #[test]
    fn selection_clears_when_no_rows_are_visible() {
        assert_eq!(resolve_selected_transfer_id(&[], Some(DownloadId(5))), None);
    }

    #[test]
    fn matched_destination_rule_icon_wins_over_filename_heuristic() {
        let mut settings = Settings::default();
        settings.destination_rules_enabled = true;
        settings.destination_rules = vec![DestinationRule {
            id: "movies".into(),
            label: "Movies".into(),
            enabled: true,
            target_dir: PathBuf::from("/tmp/Movies"),
            extensions: vec![".mkv".into()],
            icon_name: Some("video".into()),
        }];

        let row = TransferListRow {
            destination: "/tmp/Movies/movie.mkv".into(),
            filename: "movie.mkv".into(),
            ..test_row(DownloadId(1))
        };

        assert_eq!(resolved_transfer_icon_name(&row, &settings), "video");
    }

    #[test]
    fn icon_falls_back_to_filename_when_no_destination_rule_matches() {
        let mut settings = Settings::default();
        settings.destination_rules_enabled = true;
        settings.destination_rules = vec![DestinationRule {
            id: "docs".into(),
            label: "Docs".into(),
            enabled: true,
            target_dir: PathBuf::from("/tmp/Documents"),
            extensions: vec![".pdf".into()],
            icon_name: Some("document".into()),
        }];

        let row = TransferListRow {
            destination: "/tmp/Videos/movie.mkv".into(),
            filename: "movie.mkv".into(),
            ..test_row(DownloadId(1))
        };

        assert_eq!(resolved_transfer_icon_name(&row, &settings), "video");
    }

    struct ProbeTransfersHost {
        render_count: Arc<AtomicUsize>,
        visible_range: Range<usize>,
        scroll_handle: UniformListScrollHandle,
    }

    impl ProbeTransfersHost {
        fn new(
            render_count: Arc<AtomicUsize>,
            _window: &mut Window,
            _cx: &mut Context<Self>,
        ) -> Self {
            Self {
                render_count,
                visible_range: 0..0,
                scroll_handle: UniformListScrollHandle::new(),
            }
        }
    }

    impl Render for ProbeTransfersHost {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let render_count = Arc::clone(&self.render_count);

            div().size_full().child(
                v_resizable("probe-transfers-layout")
                    .child(
                        resizable_panel()
                            .size(px(PROBE_TOP_HEIGHT))
                            .size_range(px(160.0)..px(320.0))
                            .child(div().size_full()),
                    )
                    .child(
                        resizable_panel().size_range(px(120.0)..Pixels::MAX).child(
                            uniform_list(
                                "probe-transfers-list",
                                PROBE_ITEM_COUNT,
                                cx.processor(move |this, range: Range<usize>, _window, _cx| {
                                    this.visible_range = range.clone();
                                    range
                                        .map(|ix| {
                                            ProbeTransferRow::new(ix, Arc::clone(&render_count))
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .track_scroll(&self.scroll_handle)
                            .with_sizing_behavior(gpui::ListSizingBehavior::Auto)
                            .with_width_from_item(Some(0))
                            .size_full(),
                        ),
                    ),
            )
        }
    }

    #[derive(IntoElement)]
    struct ProbeTransferRow {
        ix: usize,
        render_count: Arc<AtomicUsize>,
    }

    impl ProbeTransferRow {
        fn new(ix: usize, render_count: Arc<AtomicUsize>) -> Self {
            Self { ix, render_count }
        }
    }

    impl RenderOnce for ProbeTransferRow {
        fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
            self.render_count.fetch_add(1, Ordering::Relaxed);

            div()
                .w_full()
                .h(px(PROBE_ROW_HEIGHT))
                .border_b_1()
                .border_color(Colors::border())
                .child(format!("Row {}", self.ix))
        }
    }

    fn open_probe_window(
        app: &mut TestApp,
        render_count: Arc<AtomicUsize>,
    ) -> gpui::TestAppWindow<ProbeTransfersHost> {
        let bounds = Bounds::from_corners(point(px(0.0), px(0.0)), point(px(900.0), px(700.0)));
        let mut window = app.open_window_with_options(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| ProbeTransfersHost::new(Arc::clone(&render_count), window, cx),
        );
        window.draw();
        window
    }

    // These tests were added to see if lazy loading was the problem with the
    // performance issues when dragging the bottom resizable panel.
    // might delete later?
    #[test]
    fn lazy_transfer_list_renders_only_a_visible_slice_during_vertical_drag() {
        let render_count = Arc::new(AtomicUsize::new(0));
        let mut app = TestApp::new();
        let mut window = open_probe_window(&mut app, Arc::clone(&render_count));

        let initial_visible = window.read(|host, _| host.visible_range.clone());
        let initial_rendered = render_count.swap(0, Ordering::Relaxed);

        assert!(!initial_visible.is_empty());
        assert!(initial_visible.len() < PROBE_ITEM_COUNT);
        assert!(initial_rendered < 32);

        window.simulate_mouse_down(point(px(450.0), px(PROBE_TOP_HEIGHT)), MouseButton::Left);
        window.simulate_mouse_move(point(px(450.0), px(170.0)));
        window.simulate_mouse_up(point(px(450.0), px(170.0)), MouseButton::Left);

        let resized_visible = window.read(|host, _| host.visible_range.clone());
        let rendered_during_drag = render_count.load(Ordering::Relaxed);

        assert!(resized_visible.len() >= initial_visible.len());
        assert!(resized_visible.len() < PROBE_ITEM_COUNT);
        assert!(rendered_during_drag < 48);
    }

    #[test]
    fn lazy_transfer_list_updates_visible_range_when_window_height_changes() {
        let render_count = Arc::new(AtomicUsize::new(0));
        let mut app = TestApp::new();
        let mut window = open_probe_window(&mut app, render_count);

        let initial_visible = window.read(|host, _| host.visible_range.clone());

        window.simulate_resize(size(px(900.0), px(520.0)));
        window.draw();

        let resized_visible = window.read(|host, _| host.visible_range.clone());

        assert!(!initial_visible.is_empty());
        assert!(!resized_visible.is_empty());
        assert!(resized_visible.len() <= initial_visible.len());
        assert!(resized_visible.len() < PROBE_ITEM_COUNT);
    }

    fn test_row(id: DownloadId) -> TransferListRow {
        TransferListRow {
            id,
            provider_kind: "http".into(),
            source_label: "https://example.com/file.bin".into(),
            filename: "file.bin".into(),
            destination: "/tmp/file.bin".into(),
            status: DownloadStatus::Downloading,
            downloaded_bytes: 512,
            total_bytes: Some(1024),
            progress: 0.5,
            speed_bps: 0,
            display_state: TransferDisplayState::Active,
            available_actions: TransferAvailableActions {
                pause: true,
                resume: false,
                cancel: true,
                delete_artifact: true,
            },
        }
    }
}
