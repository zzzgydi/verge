mod layout;
pub use layout::LogLayout;
#[cfg(test)]
mod tests;

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme as _, h_flex,
    scroll::{Scrollbar, ScrollbarMode},
    v_flex, v_virtual_list,
};
use gpui_kit::*;

use crate::{i18n::tr, ui::Page, view::MainView};

use super::components::PageHeader;
use super::filters::{self, Choice};

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
    let view_entity = cx.entity();
    let mut content = v_flex().flex_1().min_h_0().gap_4();
    // Retain only indices; clone/format payloads only for the visible range.
    let rows: Vec<usize> = view
        .state
        .logs
        .iter()
        .enumerate()
        .filter(|(_, log)| view.filters.logs.matches(log))
        .map(|(ix, _)| ix)
        .collect();
    content = content
        .child(
            PageHeader::new(tr(lang, "logs.title")).child(filters::count(
                rows.len(),
                view.state.logs.len(),
                cx,
            )),
        )
        .child(toolbar(view, cx));
    if rows.is_empty() {
        content = content.child(super::EmptyState::new(
            gpui_kit::component::IconName::SquareTerminal,
            tr(
                lang,
                if view.filters.active(Page::Logs) {
                    "filters.empty"
                } else {
                    "logs.empty.title"
                },
            ),
            tr(
                lang,
                if view.filters.active(Page::Logs) {
                    "filters.empty.desc"
                } else {
                    "logs.empty.unfiltered"
                },
            ),
        ));
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

fn toolbar(view: &MainView, cx: &mut Context<MainView>) -> impl IntoElement {
    let lang = view.lang();
    let mut options: Vec<(String, String)> = [
        ("problems", "logs.level.problems"),
        ("error", "logs.level.error"),
        ("warning", "logs.level.warning"),
        ("info", "logs.level.info"),
        ("debug", "logs.level.debug"),
    ]
    .into_iter()
    .map(|(value, label)| (value.into(), tr(lang, label).into()))
    .collect();
    for (value, label) in filters::choices(
        view.state
            .logs
            .iter()
            .map(|r| filters::normalized_level(&r.level)),
    ) {
        if !options.iter().any(|(v, _)| v == &value) {
            options.push((value, label));
        }
    }
    h_flex()
        .debug_selector(|| "logs-filters".into())
        .gap_2()
        .h_8()
        .child(filters::search(&view.filters.log_search, "logs-search"))
        .child(filters::search(&view.filters.log_exclude, "logs-exclude"))
        .child(filters::choice(
            view,
            Choice {
                id: "log-level-filter",
                label: "logs.filter.level",
                page: Page::Logs,
                selected: view.filters.logs.level.clone(),
                options,
                set: |v, value| v.filters.logs.level = value,
            },
            cx,
        ))
        .child(filters::clear_button(view, Page::Logs, cx))
}
