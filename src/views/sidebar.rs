use gpui::{div, prelude::*, px, Hsla, SharedString, Window};
use crate::ui::prelude::*;

/// Left sidebar
/// logo, new download button, navigation, storage card
///
pub struct Sidebar {
    pub active_item: usize,
    pub collapsed: bool,
    pub storage_used_bytes: u64,
    pub storage_total_bytes: u64,
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let nav_items: Vec<(IconName, &str)> = vec![
            (IconName::Inbox, "Downloads"),
            (IconName::ArrowDownToLine, "Active"),
            (IconName::CircleCheck, "Finished"),
            (IconName::CirclePause, "Paused"),
        ];

        let width = if self.collapsed { 56.0 } else { Spacing::SIDEBAR_WIDTH };

        div()
            .flex()
            .flex_col()
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(Colors::border())
            .bg(Colors::sidebar())
            // Logo row — expanded: horizontal with toggle on right
            .when(!self.collapsed, |el| el.child(
                div()
                    .px(px(16.0))
                    .pt(px(14.0))
                    .mb(px(22.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(OpheliaLogo::new(44.0))
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(gpui::FontWeight::EXTRA_BOLD)
                                    .text_color(Colors::foreground())
                                    .child("ophelia")
                            )
                    )
                    .child(
                        div()
                            .id("collapse-toggle")
                            .flex()
                            .items_center()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.collapsed = !this.collapsed;
                                cx.notify();
                            }))
                            .child(icon_sm(IconName::PanelLeftClose, Colors::muted_foreground()))
                    )
            ))
            // Logo row — collapsed: vertical, toggle below logo
            .when(self.collapsed, |el| el.child(
                div()
                    .pt(px(14.0))
                    .mb(px(22.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.0))
                    .child(OpheliaLogo::new(44.0))
                    .child(
                        div()
                            .id("collapse-toggle")
                            .flex()
                            .items_center()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.collapsed = !this.collapsed;
                                cx.notify();
                            }))
                            .child(icon_sm(IconName::PanelLeftOpen, Colors::muted_foreground()))
                    )
            ))

            // Add Download button
            .when(!self.collapsed, |el| el.child(
                div()
                    .px(px(16.0))
                    .mb(px(18.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w_full()
                            .h(px(40.0))
                            .rounded(px(8.0))
                            .bg(Colors::active())
                            .text_color(Colors::background())
                            .text_base()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("+ Add Download"),
                    ),
            ))
            .when(self.collapsed, |el| el.child(
                div()
                    .flex()
                    .justify_center()
                    .mb(px(18.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(40.0))
                            .h(px(40.0))
                            .rounded(px(8.0))
                            .bg(Colors::active())
                            .child(icon_sm(IconName::Plus, Colors::background())),
                    ),
            ))
            // Separator
            .child(
                div()
                    .mx(px(16.0))
                    .mb(px(10.0))
                    .h(px(1.0))
                    .bg(Colors::border()),
            )
            // Navigation items
            .child(
                div()
                    .px(px(10.0))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .children(nav_items.into_iter().enumerate().map(|(i, (icon_name, label))| {
                        let is_active = i == self.active_item;
                        nav_item(icon_name, label, is_active, self.collapsed)
                            .id(i)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.active_item = i;
                                cx.notify();
                            }))
                    })),
            )
            // Spacer pushes storage card to bottom
            .child(div().flex_1())
            // Storage card
            .when(!self.collapsed, |el| el.child(
                div()
                    .p(px(16.0))
                    .child(storage_card(self.storage_used_bytes, self.storage_total_bytes)),
            ))
    }
}

/// A single navigation row: for now
fn nav_item(icon_name: IconName, label: &str, active: bool, collapsed: bool) -> gpui::Div {
    let bg: Hsla = if active {
        Colors::muted().into()
    } else {
        gpui::transparent_black()
    };
    let text: Hsla = if active {
        Colors::foreground().into()
    } else {
        Colors::muted_foreground().into()
    };

    div()
        .flex()
        .items_center()
        .when(collapsed, |el| el.justify_center())
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(10.0))
        .rounded(px(8.0))
        .bg(bg)
        .text_color(text)
        .text_sm()
        .font_weight(gpui::FontWeight::BOLD)
        .child(icon(icon_name, px(20.0), text))
        .when(!collapsed, |el| el.child(SharedString::from(label.to_string())))
}

fn format_storage(bytes: u64) -> String {
    const GB: u64 = 1_000_000_000;
    const TB: u64 = 1_000_000_000_000;
    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else {
        format!("{} GB", bytes / GB)
    }
}

fn storage_card(used: u64, total: u64) -> gpui::Div {
    let fraction = if total > 0 { used as f32 / total as f32 } else { 0.0 };
    let available = total.saturating_sub(used);
    let pct = format!("{}%", (fraction * 100.0) as u32);

    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .p(px(14.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(Colors::border())
        .bg(Colors::card())
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Colors::finished())
                        .child(icon_sm(IconName::Database, Colors::finished()))
                        .child("Storage"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(Colors::muted_foreground())
                        .child(pct),
                ),
        )
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::EXTRA_BOLD)
                .text_color(Colors::muted_foreground())
                .child(format_storage(available)),
        )
        .child(
            div()
                .text_sm()
                .text_color(Colors::finished())
                .child("available"),
        )
        .child(
            div()
                .w_full()
                .h(px(4.0))
                .rounded_full()
                .bg(Colors::muted())
                .child(
                    div()
                        .h_full()
                        .rounded_full()
                        .bg(Colors::finished())
                        .w(px(Spacing::SIDEBAR_WIDTH * fraction * 0.75)),
                ),
        )
    }
