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

use gpui::{App, RenderOnce, SharedString, Window, div, prelude::*, px};
use rust_i18n::t;

use crate::app::{Downloads, TransferListRow};
use crate::engine::{ChunkMapCellState, TransferChunkMapState};
use crate::ui::prelude::*;

#[derive(IntoElement)]
pub struct ChunkMapCard {
    model: ChunkMapCardModel,
}

#[derive(Clone)]
pub struct ChunkMapCardModel {
    filename: Option<SharedString>,
    total_size_label: Option<SharedString>,
    state: ChunkMapCardState,
}

#[derive(Clone)]
enum ChunkMapCardState {
    Empty,
    Unsupported,
    Loading,
    Http(Vec<Vec<ChunkMapCellState>>),
}

const CHUNK_MAP_COLUMNS: usize = 16;
const CHUNK_MAP_CELL_HEIGHT: f32 = 10.0;
const CHUNK_MAP_CELL_RADIUS: f32 = 2.0;
const CHUNK_MAP_GRID_GAP: f32 = 4.0;

impl ChunkMapCard {
    pub fn new(model: ChunkMapCardModel) -> Self {
        Self { model }
    }
}

impl ChunkMapCardModel {
    pub fn from_transfer_rows(rows: &[TransferListRow], downloads: &Downloads) -> Self {
        let Some((row, state)) = rows
            .iter()
            .find_map(|row| {
                let state = downloads.transfer_chunk_map_state(row.id);
                (!matches!(state, TransferChunkMapState::Unsupported))
                    .then_some((row, state))
            })
            .or_else(|| {
                rows.first()
                    .map(|row| (row, downloads.transfer_chunk_map_state(row.id)))
            })
        else {
            return Self {
                filename: None,
                total_size_label: None,
                state: ChunkMapCardState::Empty,
            };
        };

        let filename = Some(row.filename.clone());

        match state {
            TransferChunkMapState::Unsupported => Self {
                filename,
                total_size_label: None,
                state: ChunkMapCardState::Unsupported,
            },
            TransferChunkMapState::Loading => Self {
                filename,
                total_size_label: None,
                state: ChunkMapCardState::Loading,
            },
            TransferChunkMapState::Http(snapshot) => Self {
                filename,
                total_size_label: Some(format_bytes(snapshot.total_bytes).into()),
                state: ChunkMapCardState::Http(chunk_rows(&snapshot.cells)),
            },
        }
    }
}

impl RenderOnce for ChunkMapCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let state = self.model.state.clone();

        div()
            .flex()
            .flex_col()
            .h_full()
            .justify_between()
            .gap(px(Spacing::ROW_GAP))
            .p(px(Chrome::STATS_CARD_PADDING))
            .rounded(px(Chrome::PANEL_RADIUS))
            .border_1()
            .border_color(Colors::border())
            .bg(Colors::card())
            .children(
                self.model
                    .filename
                    .map(|filename| {
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(Spacing::CONTROL_GAP))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(Colors::foreground())
                                    .truncate()
                                    .child(filename),
                            )
                            .children(
                                self.model
                                    .total_size_label
                                    .map(|total_size| {
                                        div()
                                            .px(px(10.0))
                                            .py(px(4.0))
                                            .rounded_full()
                                            .bg(Colors::muted())
                                            .text_xs()
                                            .text_color(Colors::muted_foreground())
                                            .child(total_size)
                                            .into_any_element()
                                    }),
                            )
                            .into_any_element()
                    }),
            )
            .child(match state {
                ChunkMapCardState::Empty => state_message(
                    t!("stats.chunk_map_empty").to_string(),
                    t!("stats.chunk_map_empty_detail").to_string(),
                )
                .into_any_element(),
                ChunkMapCardState::Unsupported => state_message(
                    t!("stats.chunk_map_unavailable").to_string(),
                    t!("stats.chunk_map_unavailable_detail").to_string(),
                )
                .into_any_element(),
                ChunkMapCardState::Loading => state_message(
                    t!("stats.chunk_map_loading").to_string(),
                    t!("stats.chunk_map_loading_detail").to_string(),
                )
                .into_any_element(),
                ChunkMapCardState::Http(rows) => chunk_grid(rows).into_any_element(),
            })
            .when(matches!(self.model.state, ChunkMapCardState::Http(_)), |this| {
                this.child(chunk_map_legend())
            })
    }
}

fn chunk_grid(rows: Vec<Vec<ChunkMapCellState>>) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .justify_center()
        .gap(px(CHUNK_MAP_GRID_GAP))
        .children(rows.into_iter().map(|row| {
            h_flex()
                .flex_1()
                .gap(px(CHUNK_MAP_GRID_GAP))
                .children(row.into_iter().map(|cell| {
                    div()
                        .flex_1()
                        .h(px(CHUNK_MAP_CELL_HEIGHT))
                        .rounded(px(CHUNK_MAP_CELL_RADIUS))
                        .bg(cell_color(cell))
                        .into_any_element()
                }))
                .into_any_element()
        }))
}

fn chunk_map_legend() -> impl IntoElement {
    h_flex()
        .items_center()
        .gap(px(Spacing::ROW_GAP))
        .text_xs()
        .text_color(Colors::muted_foreground())
        .children([
            legend_item(
                ChunkMapCellState::Empty,
                t!("stats.chunk_map_legend_empty").to_string(),
            )
            .into_any_element(),
            legend_item(
                ChunkMapCellState::Partial,
                t!("stats.chunk_map_legend_partial").to_string(),
            )
            .into_any_element(),
            legend_item(
                ChunkMapCellState::Complete,
                t!("stats.chunk_map_legend_complete").to_string(),
            )
            .into_any_element(),
        ])
}

fn legend_item(cell: ChunkMapCellState, label: String) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap(px(Spacing::LIST_GAP))
        .child(
            div()
                .w(px(10.0))
                .h(px(10.0))
                .rounded(px(2.0))
                .bg(cell_color(cell)),
        )
        .child(div().child(label))
}

fn state_message(title: String, detail: String) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(Spacing::LIST_GAP))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(Colors::foreground())
                .text_center()
                .child(title),
        )
        .child(
            div()
                .max_w(px(240.0))
                .text_xs()
                .text_center()
                .text_color(Colors::muted_foreground())
                .line_clamp(2)
                .child(detail),
        )
}

fn cell_color(cell: ChunkMapCellState) -> gpui::Rgba {
    match cell {
        ChunkMapCellState::Empty => Colors::border(),
        ChunkMapCellState::Partial => Colors::finished(),
        ChunkMapCellState::Complete => Colors::active(),
    }
}

fn chunk_rows(cells: &[ChunkMapCellState]) -> Vec<Vec<ChunkMapCellState>> {
    cells.chunks(CHUNK_MAP_COLUMNS).map(|row| row.to_vec()).collect()
}

fn format_bytes(bytes: u64) -> String {
    const GB: u64 = 1_000_000_000;
    const MB: u64 = 1_000_000;
    const KB: u64 = 1_000;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_rows_break_fixed_width_cells_into_grid_rows() {
        let cells = vec![ChunkMapCellState::Complete; 128];
        let rows = chunk_rows(&cells);

        assert_eq!(rows.len(), 8);
        assert!(rows.iter().all(|row| row.len() == 16));
    }

    #[test]
    fn format_bytes_uses_human_readable_units() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(12_300), "12 KB");
        assert_eq!(format_bytes(4_500_000), "4.5 MB");
        assert_eq!(format_bytes(2_300_000_000), "2.3 GB");
    }
}
