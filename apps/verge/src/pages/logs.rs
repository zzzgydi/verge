use std::rc::Rc;

use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _,
    button::Button,
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex, v_virtual_list,
};

use crate::{
    i18n::{self, tr},
    view::MainView,
};

use super::components::page_heading;

/// (文案 key, 级别过滤值)。
const LEVELS: [(&str, Option<&'static str>); 5] = [
    ("logs.level.all", None),
    ("logs.level.info", Some("info")),
    ("logs.level.warning", Some("warning")),
    ("logs.level.error", Some("error")),
    ("logs.level.debug", Some("debug")),
];

/// 日志行高。虚拟列表要求渲染行高与 item_sizes 逐像素一致。
const LOG_ROW_HEIGHT: f32 = 22.;

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
    cx: &Context<MainView>,
) -> AnyElement {
    div()
        .id(SharedString::from(format!("log-row-{}", source_index)))
        .h(px(LOG_ROW_HEIGHT))
        .w_full()
        .px_2()
        .flex_shrink_0()
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .line_height(px(LOG_ROW_HEIGHT))
        .text_xs()
        .font_family(cx.theme().mono_font_family.clone())
        .text_color(level_color(&row.level, cx))
        .child(format!("[{}] {}", row.level, row.payload))
        .into_any_element()
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let filter = view.log_filter;
    let filter_label = LEVELS
        .iter()
        .find(|(_, level)| *level == filter)
        .map_or(tr(lang, "logs.level.all"), |(key, _)| tr(lang, key));

    let view_entity = cx.entity();
    let filter_button = Button::new("log-level-filter")
        .small()
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

    let mut content = v_flex().flex_1().min_h_0().gap_5().child(
        h_flex()
            .justify_between()
            .flex_shrink_0()
            .child(page_heading(
                tr(lang, "logs.title"),
                tr(lang, "logs.subtitle"),
                cx,
            ))
            .child(filter_button),
    );

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
        let item_sizes = Rc::new(
            rows.iter()
                .map(|_| size(px(0.), px(LOG_ROW_HEIGHT)))
                .collect::<Vec<_>>(),
        );
        content = content.child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_lg()
                .py_2()
                .child(
                    v_virtual_list(
                        view_entity,
                        "log-list",
                        item_sizes,
                        move |this, range, _, cx| {
                            range
                                .filter_map(|ix| {
                                    rows.get(ix).and_then(|&source| {
                                        this.state.logs.get(source).map(|log| (source, log))
                                    })
                                })
                                .map(|(source, log)| render_log_row(source, log, cx))
                                .collect()
                        },
                    )
                    .track_scroll(&view.log_scroll),
                )
                .child(gpui_component::scroll::Scrollbar::vertical(
                    &view.log_scroll,
                )),
        );
    }
    content.into_any_element()
}
