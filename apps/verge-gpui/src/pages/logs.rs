use std::rc::Rc;

use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _,
    button::Button,
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex, v_virtual_list,
};

use crate::{i18n::{self, tr}, view::MainView};

use super::page_title;

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

/// 过滤后的日志行（级别 + 内容）。
struct LogRow {
    level: String,
    payload: String,
}

fn filtered_rows(view: &MainView) -> Vec<LogRow> {
    let filter = view.log_filter;
    view.state
        .logs
        .iter()
        .filter(|log| filter.is_none_or(|level| log.level == level))
        .map(|log| LogRow {
            level: log.level.clone(),
            payload: log.payload.clone(),
        })
        .collect()
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

fn render_log_row(row: &LogRow, cx: &Context<MainView>) -> AnyElement {
    div()
        .id(SharedString::from(format!("log-{}-{}", row.level, row.payload)))
        .h(px(LOG_ROW_HEIGHT))
        .px_2()
        .flex_shrink_0()
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
            let view_entity = view_entity.clone();
            move |menu, _, _| {
            LEVELS.into_iter().fold(menu, |menu, (key, level)| {
                menu.item(PopupMenuItem::new(tr(lang, key)).on_click({
                    let view = view_entity.clone();
                    move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.log_filter = level;
                            cx.notify();
                        });
                    }
                }))
            })
            }
        });

    let mut content = v_flex().size_full().gap_2().child(
        h_flex()
            .justify_between()
            .child(page_title(tr(lang, "logs.title")))
            .child(filter_button),
    );

    let rows = filtered_rows(view);
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
        content = content.child(div().flex_1().min_h_0().child(v_virtual_list(
            view_entity,
            "log-list",
            item_sizes,
            |this, range, _, cx| {
                let rows = filtered_rows(this);
                range
                    .filter_map(|ix| rows.get(ix))
                    .map(|row| render_log_row(row, cx))
                    .collect()
            },
        )));
    }
    content.into_any_element()
}
