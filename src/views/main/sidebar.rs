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

use std::rc::Rc;

use crate::app::{Downloads, SidebarStorageSummary};
use crate::app_menu;
use crate::ui::prelude::*;
use gpui::{
    App, Context, Entity, Hsla, Render, RenderOnce, SharedString, Window, div, prelude::*, px,
    relative,
};
use rust_i18n::t;

type ClickHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarMode {
    Expanded,
    Collapsed,
}

impl SidebarMode {
    fn toggled(self) -> Self {
        match self {
            Self::Expanded => Self::Collapsed,
            Self::Collapsed => Self::Expanded,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SidebarLayout {
    pub mode: SidebarMode,
    pub expanded_width: f32,
}

/// Left sidebar
/// logo, new HTTP download button, navigation, storage card
pub struct Sidebar {
    active_item: usize,
    mode: SidebarMode,
    expanded_width: f32,
    downloads: Option<Entity<Downloads>>,
}

impl Sidebar {
    pub fn new(downloads: Entity<Downloads>, cx: &mut Context<Self>) -> Self {
        cx.observe(&downloads, |_, _, cx| cx.notify()).detach();
        Self {
            active_item: 0,
            mode: SidebarMode::Expanded,
            expanded_width: Spacing::SIDEBAR_WIDTH,
            downloads: Some(downloads),
        }
    }

    pub fn active_item(&self) -> usize {
        self.active_item
    }

    pub fn layout(&self) -> SidebarLayout {
        SidebarLayout {
            mode: self.mode,
            expanded_width: self.expanded_width,
        }
    }

    pub fn set_expanded_width(&mut self, width: f32) {
        self.expanded_width = width;
    }

    fn view_model(&self, cx: &App) -> SidebarViewModel {
        SidebarViewModel {
            mode: self.mode,
            nav_items: vec![
                SidebarNavItemModel::new(
                    0,
                    IconName::Inbox,
                    t!("sidebar.transfers").to_string(),
                    self.active_item == 0,
                ),
                SidebarNavItemModel::new(
                    1,
                    IconName::History,
                    t!("sidebar.history").to_string(),
                    self.active_item == 1,
                ),
            ],
            storage: StorageCardModel::from_summary(
                self.downloads
                    .as_ref()
                    .map(|downloads| downloads.read(cx).storage_summary())
                    .unwrap_or_default(),
            ),
        }
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vm = self.view_model(cx);
        let weak = cx.weak_entity();

        let toggle_sidebar: ClickHandler = Rc::new({
            let weak = weak.clone();
            move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.mode = this.mode.toggled();
                    cx.notify();
                });
            }
        });

        let open_http_download: ClickHandler = Rc::new(|window, cx| {
            window.dispatch_action(Box::new(app_menu::OpenDownloadModal), cx);
        });

        div()
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(Colors::border())
            .bg(Colors::sidebar())
            .child(SidebarBrand::new(vm.mode, Rc::clone(&toggle_sidebar)))
            .child(
                div()
                    .mx(px(Spacing::SIDEBAR_SECTION_PADDING))
                    .mb(px(Spacing::SECTION_GAP))
                    .child(SidebarDownloadButton::new(
                        vm.mode,
                        Rc::clone(&open_http_download),
                    )),
            )
            .child(sidebar_separator().mx(px(Spacing::SIDEBAR_SECTION_PADDING)))
            .child(
                v_flex()
                    .px(px(Spacing::SIDEBAR_NAV_PADDING_X))
                    .gap(px(Chrome::MENU_BAR_GAP))
                    .children(vm.nav_items.into_iter().map(|item| {
                        let index = item.index;
                        let on_click: ClickHandler = Rc::new({
                            let weak = weak.clone();
                            move |_, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.active_item = index;
                                    cx.notify();
                                });
                            }
                        });
                        SidebarNavItem::new(item, vm.mode, on_click)
                    })),
            )
            .child(div().flex_1())
            .when(matches!(vm.mode, SidebarMode::Expanded), |this| {
                this.child(
                    div()
                        .m(px(Spacing::SIDEBAR_SECTION_PADDING))
                        .mt(px(Spacing::SECTION_GAP))
                        .child(StorageCard::new(vm.storage)),
                )
            })
    }
}

#[derive(Clone)]
struct SidebarViewModel {
    mode: SidebarMode,
    nav_items: Vec<SidebarNavItemModel>,
    storage: StorageCardModel,
}

#[derive(Clone)]
struct SidebarNavItemModel {
    index: usize,
    icon_name: IconName,
    label: SharedString,
    active: bool,
}

impl SidebarNavItemModel {
    fn new(
        index: usize,
        icon_name: IconName,
        label: impl Into<SharedString>,
        active: bool,
    ) -> Self {
        Self {
            index,
            icon_name,
            label: label.into(),
            active,
        }
    }
}

#[derive(Clone)]
struct StorageCardModel {
    used: String,
    total: String,
    fraction: f32,
}

impl StorageCardModel {
    fn from_summary(summary: SidebarStorageSummary) -> Self {
        Self {
            used: format_gb(summary.used_bytes),
            total: format_gb(summary.total_bytes),
            fraction: summary.fraction,
        }
    }
}

#[derive(IntoElement)]
struct SidebarBrand {
    mode: SidebarMode,
    on_toggle: ClickHandler,
}

impl SidebarBrand {
    fn new(mode: SidebarMode, on_toggle: ClickHandler) -> Self {
        Self { mode, on_toggle }
    }
}

impl RenderOnce for SidebarBrand {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_toggle = Rc::clone(&self.on_toggle);
        let toggle_icon = match self.mode {
            SidebarMode::Expanded => IconName::PanelLeftClose,
            SidebarMode::Collapsed => IconName::PanelLeftOpen,
        };
        let toggle = div()
            .id("collapse-toggle")
            .flex()
            .items_center()
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                on_toggle(window, cx);
            })
            .child(IconBox::new(toggle_icon, Colors::muted_foreground()));

        match self.mode {
            SidebarMode::Collapsed => div()
                .pt(px(Chrome::SIDEBAR_HEADER_TOP))
                .mb(px(Chrome::SIDEBAR_HEADER_BOTTOM_MARGIN))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(Spacing::CONTROL_GAP))
                .child(OpheliaLogo::new(44.0))
                .child(toggle)
                .into_any_element(),
            SidebarMode::Expanded => div()
                .px(px(Spacing::SIDEBAR_SECTION_PADDING))
                .pt(px(Chrome::SIDEBAR_HEADER_TOP))
                .mb(px(Chrome::SIDEBAR_HEADER_BOTTOM_MARGIN))
                .flex()
                .items_center()
                .justify_between()
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(Spacing::CONTROL_GAP))
                        .child(OpheliaLogo::new(44.0))
                        .child(
                            div()
                                .font_family(TITLE_FONT_FAMILY)
                                .text_xl()
                                .font_weight(gpui::FontWeight::NORMAL)
                                .text_color(Colors::foreground())
                                .child(t!("app.name").to_string()),
                        ),
                )
                .child(toggle)
                .into_any_element(),
        }
    }
}

#[derive(IntoElement)]
struct SidebarDownloadButton {
    mode: SidebarMode,
    on_click: ClickHandler,
}

impl SidebarDownloadButton {
    fn new(mode: SidebarMode, on_click: ClickHandler) -> Self {
        Self { mode, on_click }
    }
}

impl RenderOnce for SidebarDownloadButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_click = Rc::clone(&self.on_click);
        let id = match self.mode {
            SidebarMode::Collapsed => "add-download-btn-collapsed",
            SidebarMode::Expanded => "add-download-btn",
        };
        let button = div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .h(px(Chrome::SIDEBAR_BUTTON_SIZE))
            .cursor_pointer()
            .rounded(px(Chrome::BUTTON_RADIUS))
            .bg(Colors::active())
            .text_color(Colors::background())
            .font_weight(gpui::FontWeight::NORMAL)
            .on_click(move |_, window, cx| {
                on_click(window, cx);
            });

        match self.mode {
            SidebarMode::Collapsed => h_flex()
                .justify_center()
                .child(
                    button
                        .w(px(Chrome::SIDEBAR_BUTTON_SIZE))
                        .child(IconBox::new(IconName::Plus, Colors::background())),
                )
                .into_any_element(),
            SidebarMode::Expanded => button
                .w_full()
                .text_base()
                .child(t!("sidebar.add_download").to_string())
                .into_any_element(),
        }
    }
}

#[derive(IntoElement)]
struct SidebarNavItem {
    item: SidebarNavItemModel,
    mode: SidebarMode,
    on_click: ClickHandler,
}

impl SidebarNavItem {
    fn new(item: SidebarNavItemModel, mode: SidebarMode, on_click: ClickHandler) -> Self {
        Self {
            item,
            mode,
            on_click,
        }
    }
}

impl RenderOnce for SidebarNavItem {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let bg: Hsla = if self.item.active {
            Colors::muted().into()
        } else {
            gpui::transparent_black()
        };
        let text: Hsla = if self.item.active {
            Colors::foreground().into()
        } else {
            Colors::muted_foreground().into()
        };
        let on_click = Rc::clone(&self.on_click);

        div()
            .id(("sidebar-nav-item", self.item.index))
            .flex()
            .items_center()
            .when(matches!(self.mode, SidebarMode::Collapsed), |this| {
                this.justify_center()
            })
            .gap(px(Chrome::HEADER_GAP))
            .px(px(Chrome::SIDEBAR_NAV_ITEM_PADDING_X))
            .py(px(Chrome::SIDEBAR_NAV_ITEM_PADDING_Y))
            .rounded(px(Chrome::BUTTON_RADIUS))
            .bg(bg)
            .text_color(text)
            .text_sm()
            .font_weight(gpui::FontWeight::NORMAL)
            .cursor_pointer()
            .when(self.item.active, |this| {
                this.border_l_2().border_color(Colors::ring())
            })
            .on_click(move |_, window, cx| {
                on_click(window, cx);
            })
            .child(IconBox::medium(self.item.icon_name, text))
            .when(matches!(self.mode, SidebarMode::Expanded), |this| {
                this.child(self.item.label)
            })
    }
}

#[derive(IntoElement)]
struct StorageCard {
    model: StorageCardModel,
}

impl StorageCard {
    fn new(model: StorageCardModel) -> Self {
        Self { model }
    }
}

impl RenderOnce for StorageCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(Spacing::LIST_GAP))
            .p(px(Spacing::ROW_PADDING_Y))
            .rounded(px(Chrome::BUTTON_RADIUS))
            .border_1()
            .border_color(Colors::border())
            .bg(Colors::card())
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(Colors::finished())
                    .child(IconBox::new(IconName::Database, Colors::finished()))
                    .child(t!("sidebar.storage").to_string()),
            )
            .child(
                h_flex()
                    .items_end()
                    .gap(px(3.0))
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(Colors::foreground())
                            .child(self.model.used),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::NORMAL)
                            .text_color(Colors::muted_foreground())
                            .mb(px(1.0))
                            .child("/"),
                    )
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui::FontWeight::NORMAL)
                            .text_color(Colors::muted_foreground())
                            .child(self.model.total),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(Colors::finished())
                    .child(t!("sidebar.storage_used").to_string()),
            )
            .child(
                div()
                    .w_full()
                    .h(px(Chrome::STORAGE_BAR_HEIGHT))
                    .rounded_full()
                    .bg(Colors::muted())
                    .child(
                        div()
                            .h_full()
                            .rounded_full()
                            .bg(Colors::finished())
                            .w(relative(self.model.fraction)),
                    ),
            )
    }
}

fn sidebar_separator() -> gpui::Div {
    div().mb(px(10.0)).h(px(1.0)).bg(Colors::border())
}

fn format_gb(bytes: u64) -> String {
    const GB: f64 = 1_000_000_000.0;
    const TB: f64 = 1_000_000_000_000.0;
    let b = bytes as f64;
    if b >= TB {
        format!("{:.1} TB", b / TB)
    } else {
        format!("{:.1} GB", b / GB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapse_preserves_last_expanded_width() {
        let mut sidebar = Sidebar {
            active_item: 0,
            mode: SidebarMode::Expanded,
            expanded_width: 280.0,
            downloads: None,
        };

        sidebar.mode = sidebar.mode.toggled();
        let layout = sidebar.layout();
        assert_eq!(layout.mode, SidebarMode::Collapsed);
        assert_eq!(layout.expanded_width, 280.0);

        sidebar.mode = sidebar.mode.toggled();
        let layout = sidebar.layout();
        assert_eq!(layout.mode, SidebarMode::Expanded);
        assert_eq!(layout.expanded_width, 280.0);
    }
}
