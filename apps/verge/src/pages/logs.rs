mod layout;
pub use layout::LogLayout;
#[cfg(test)]
mod tests;

use std::rc::Rc;

use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _,
    button::Button,
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::{Scrollbar, ScrollbarMode},
    v_flex, v_virtual_list,
};

use crate::{
    i18n::{self, tr},
    view::MainView,
};

use super::components::PageHeader;

/// (文案 key, 级别过滤值)。
const LEVELS: [(&str, Option<&'static str>); 5] = [
    ("logs.level.all", None),
    ("logs.level.info", Some("info")),
    ("logs.level.warning", Some("warning")),
    ("logs.level.error", Some("error")),
    ("logs.level.debug", Some("debug")),
];

/// 日志行高。虚拟列表要求渲染行高与 item_sizes 逐像素一致。
const LOG_ROW_HEIGHT: f32 = 24.;
const LOG_FONT_SIZE: f32 = 12.;
const LOG_INSET: f32 = 8.;

fn display_text(row: &crate::domain::LogEvent) -> String {
    // Preserve physical lines, with identical text for measuring and painting.
    format!("[{}] {}", row.level, row.payload)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ")
}

/// 按级别着色：错误红、警告黄、调试灰、信息默认前景。
fn level_color(level: &str, cx: &Context<MainView>) -> Hsla {
    match level {
        "error" => cx.theme().danger,
        "warning" => cx.theme().warning,
        "debug" => cx.theme().muted_foreground,
        _ => cx.theme().foreground,
    }
}

fn render_log_row(
    source_index: usize,
    row: &crate::domain::LogEvent,
    dimensions: Size<Pixels>,
    cx: &Context<MainView>,
) -> AnyElement {
    div()
        .id(SharedString::from(format!("log-row-{}", source_index)))
        .debug_selector(move || format!("log-row-{source_index}"))
        .h(dimensions.height)
        .w(dimensions.width)
        .px(px(LOG_INSET))
        .flex_shrink_0()
        .whitespace_nowrap()
        .line_height(px(LOG_ROW_HEIGHT))
        .text_size(px(LOG_FONT_SIZE))
        .font_family(cx.theme().mono_font_family.clone())
        .text_color(level_color(&row.level, cx))
        .child(
            div()
                .debug_selector(move || format!("log-text-{source_index}"))
                .child(display_text(row)),
        )
        .into_any_element()
}

pub fn render(view: &mut MainView, window: &mut Window, cx: &mut Context<MainView>) -> AnyElement {
    view.log_layout.sync(&view.state, window, cx);
    let lang = view.lang();
    let filter = view.log_filter;
    let filter_label = LEVELS
        .iter()
        .find(|(_, level)| *level == filter)
        .map_or(tr(lang, "logs.level.all"), |(key, _)| tr(lang, key));

    let view_entity = cx.entity();
    let filter_button = Button::new("log-level-filter")
        .small()
        .h(px(crate::appearance::metrics::COMPACT_CONTROL))
        .outline()
        .label(i18n::fmt_logs_filter(lang, filter_label))
        .dropdown_menu({
            let view_entity = view_entity.downgrade();
            move |menu, _, _| {
                LEVELS.into_iter().fold(menu, |menu, (key, level)| {
                    menu.item(PopupMenuItem::new(tr(lang, key)).on_click({
                        let view = view_entity.clone();
                        move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.log_filter = level;
                                cx.notify();
                            });
                        }
                    }))
                })
            }
        });

    let mut content = v_flex()
        .flex_1()
        .min_h_0()
        .gap_4()
        .child(PageHeader::new(tr(lang, "logs.title")).child(filter_button));

    // Retain only indices; clone/format payloads only for the visible range.
    let rows: Vec<usize> = view
        .state
        .logs
        .iter()
        .enumerate()
        .filter(|(_, log)| filter.is_none_or(|level| log.level == level))
        .map(|(ix, _)| ix)
        .collect();
    if rows.is_empty() {
        let empty = super::EmptyState::new(
            gpui_component::IconName::SquareTerminal,
            tr(lang, "logs.empty.title"),
            if filter.is_some() {
                tr(lang, "logs.empty.filtered")
            } else {
                tr(lang, "logs.empty.unfiltered")
            },
        );
        content = content.child(if filter.is_some() {
            empty.action(
                Button::new("clear-log-filter")
                    .label(tr(lang, "logs.clear_filter"))
                    .small()
                    .h(px(crate::appearance::metrics::COMPACT_CONTROL))
                    .outline()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.log_filter = None;
                        cx.notify();
                    })),
            )
        } else {
            empty
        });
    } else {
        // 按行虚拟列表：每行独立渲染并带级别着色；长日志滚动不卡。
        let width = rows
            .iter()
            .map(|&ix| view.log_layout.size(ix).width)
            .max()
            .unwrap_or_default();
        let item_sizes = Rc::new(
            rows.iter()
                .map(|&ix| size(width, view.log_layout.size(ix).height))
                .collect::<Vec<_>>(),
        );
        let row_sizes = item_sizes.clone();
        content = content.child(
            div()
                .debug_selector(|| "log-viewport".into())
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_lg()
                .p(px(LOG_INSET))
                .child(
                    v_virtual_list(
                        view_entity,
                        "log-list",
                        item_sizes,
                        move |this, range, _, cx| {
                            range
                                .filter_map(|ix| {
                                    rows.get(ix).and_then(|&source| {
                                        this.state
                                            .logs
                                            .get(source)
                                            .map(|log| (source, log, row_sizes[ix]))
                                    })
                                })
                                .map(|(source, log, dimensions)| {
                                    render_log_row(source, log, dimensions, cx)
                                })
                                .collect()
                        },
                    )
                    .track_scroll(&view.log_scroll),
                )
                .child(Scrollbar::new(&view.log_scroll).mode(ScrollbarMode::Always)),
        );
    }
    content.into_any_element()
}
