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

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::app_menu;
use crate::ui::prelude::*;
use gpui::{
    App, Context, Hsla, Render, RenderOnce, SharedString, Window, div, prelude::*, px, relative,
};
use rust_i18n::t;

type ClickHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// Left sidebar
/// logo, new download button, navigation, storage card
pub struct Sidebar {
    pub active_item: usize,
    pub collapsed: bool,
    pub download_dir: PathBuf,
}

impl Sidebar {
    fn view_model(&self) -> SidebarViewModel {
        SidebarViewModel {
            collapsed: self.collapsed,
            width: if self.collapsed {
                Spacing::SIDEBAR_COLLAPSED_WIDTH
            } else {
                Spacing::SIDEBAR_WIDTH
            },
            nav_items: vec![
                SidebarNavItemModel::new(
                    0,
                    IconName::Inbox,
                    t!("sidebar.transfers").to_string(),
                    self.active_item == 0,
                ),
                SidebarNavItemModel::new(
                    1,
                    IconName::Database,
                    t!("sidebar.history").to_string(),
                    self.active_item == 1,
                ),
            ],
            storage: StorageCardModel::from_path(&self.download_dir),
        }
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vm = self.view_model();
        let weak = cx.weak_entity();

        let toggle_sidebar: ClickHandler = Rc::new({
            let weak = weak.clone();
            move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.collapsed = !this.collapsed;
                    cx.notify();
                });
            }
        });

        let open_download: ClickHandler = Rc::new(|window, cx| {
            window.dispatch_action(Box::new(app_menu::OpenDownloadModal), cx);
        });

        div()
            .flex()
            .flex_col()
            .w(px(vm.width))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(Colors::border())
            .bg(Colors::sidebar())
            .child(SidebarBrand::new(vm.collapsed, Rc::clone(&toggle_sidebar)))
            .child(
                div()
                    .mx(px(Spacing::SIDEBAR_SECTION_PADDING))
                    .mb(px(Spacing::SECTION_GAP))
                    .child(SidebarDownloadButton::new(
                        vm.collapsed,
                        Rc::clone(&open_download),
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
                        SidebarNavItem::new(item, vm.collapsed, on_click)
                    })),
            )
            .child(div().flex_1())
            .when(!vm.collapsed, |this| {
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
    collapsed: bool,
    width: f32,
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
    fn from_path(path: &Path) -> Self {
        let (used, total) = query_disk(path);
        let fraction = if total > 0 {
            (used as f32 / total as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };

        Self {
            used: format_gb(used),
            total: format_gb(total),
            fraction,
        }
    }
}

#[derive(IntoElement)]
struct SidebarBrand {
    collapsed: bool,
    on_toggle: ClickHandler,
}

impl SidebarBrand {
    fn new(collapsed: bool, on_toggle: ClickHandler) -> Self {
        Self {
            collapsed,
            on_toggle,
        }
    }
}

impl RenderOnce for SidebarBrand {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_toggle = Rc::clone(&self.on_toggle);
        let toggle = div()
            .id("collapse-toggle")
            .flex()
            .items_center()
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                on_toggle(window, cx);
            })
            .child(icon_sm(
                if self.collapsed {
                    IconName::PanelLeftOpen
                } else {
                    IconName::PanelLeftClose
                },
                Colors::muted_foreground(),
            ));

        if self.collapsed {
            div()
                .pt(px(Chrome::SIDEBAR_HEADER_TOP))
                .mb(px(Chrome::SIDEBAR_HEADER_BOTTOM_MARGIN))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(Spacing::CONTROL_GAP))
                .child(OpheliaLogo::new(44.0))
                .child(toggle)
                .into_any_element()
        } else {
            div()
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
                                .text_xl()
                                .font_weight(gpui::FontWeight::EXTRA_BOLD)
                                .text_color(Colors::foreground())
                                .child(t!("app.name").to_string()),
                        ),
                )
                .child(toggle)
                .into_any_element()
        }
    }
}

#[derive(IntoElement)]
struct SidebarDownloadButton {
    collapsed: bool,
    on_click: ClickHandler,
}

impl SidebarDownloadButton {
    fn new(collapsed: bool, on_click: ClickHandler) -> Self {
        Self {
            collapsed,
            on_click,
        }
    }
}

impl RenderOnce for SidebarDownloadButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let on_click = Rc::clone(&self.on_click);
        let button = div()
            .id(if self.collapsed {
                "add-download-btn-collapsed"
            } else {
                "add-download-btn"
            })
            .flex()
            .items_center()
            .justify_center()
            .h(px(Chrome::SIDEBAR_BUTTON_SIZE))
            .cursor_pointer()
            .rounded(px(Chrome::BUTTON_RADIUS))
            .bg(Colors::active())
            .text_color(Colors::background())
            .font_weight(gpui::FontWeight::BOLD)
            .on_click(move |_, window, cx| {
                on_click(window, cx);
            });

        if self.collapsed {
            h_flex()
                .justify_center()
                .child(
                    button
                        .w(px(Chrome::SIDEBAR_BUTTON_SIZE))
                        .child(icon_sm(IconName::Plus, Colors::background())),
                )
                .into_any_element()
        } else {
            button
                .w_full()
                .text_base()
                .child(t!("sidebar.add_download").to_string())
                .into_any_element()
        }
    }
}

#[derive(IntoElement)]
struct SidebarNavItem {
    item: SidebarNavItemModel,
    collapsed: bool,
    on_click: ClickHandler,
}

impl SidebarNavItem {
    fn new(item: SidebarNavItemModel, collapsed: bool, on_click: ClickHandler) -> Self {
        Self {
            item,
            collapsed,
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
            .when(self.collapsed, |this| this.justify_center())
            .gap(px(Chrome::HEADER_GAP))
            .px(px(Chrome::SIDEBAR_NAV_ITEM_PADDING_X))
            .py(px(Chrome::SIDEBAR_NAV_ITEM_PADDING_Y))
            .rounded(px(Chrome::BUTTON_RADIUS))
            .bg(bg)
            .text_color(text)
            .text_sm()
            .font_weight(gpui::FontWeight::BOLD)
            .cursor_pointer()
            .when(self.item.active, |this| {
                this.border_l_2().border_color(Colors::ring())
            })
            .on_click(move |_, window, cx| {
                on_click(window, cx);
            })
            .child(icon(self.item.icon_name, px(20.0), text))
            .when(!self.collapsed, |this| this.child(self.item.label))
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
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(Colors::finished())
                    .child(icon_sm(IconName::Database, Colors::finished()))
                    .child(t!("sidebar.storage").to_string()),
            )
            .child(
                h_flex()
                    .items_end()
                    .gap(px(3.0))
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui::FontWeight::EXTRA_BOLD)
                            .text_color(Colors::foreground())
                            .child(self.model.used),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(Colors::muted_foreground())
                            .mb(px(1.0))
                            .child("/"),
                    )
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui::FontWeight::BOLD)
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

fn query_disk(path: &Path) -> (u64, u64) {
    use std::ffi::CString;

    let Ok(cpath) = CString::new(path.to_string_lossy().as_bytes()) else {
        return (0, 0);
    };
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(cpath.as_ptr(), &mut stat) } != 0 {
        return (0, 0);
    }
    let block = stat.f_frsize as u64;
    let total = block * stat.f_blocks as u64;
    let avail = block * stat.f_bavail as u64;
    (total.saturating_sub(avail), total)
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
