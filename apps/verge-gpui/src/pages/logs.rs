use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _,
    button::Button,
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    text::TextView,
    v_flex,
};

use crate::view::MainView;

use super::page_title;

const LEVELS: [(&str, Option<&'static str>); 5] = [
    ("全部", None),
    ("信息", Some("info")),
    ("警告", Some("warning")),
    ("错误", Some("error")),
    ("调试", Some("debug")),
];

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let filter = view.log_filter;
    let filter_label = LEVELS
        .iter()
        .find(|(_, level)| *level == filter)
        .map_or("全部", |(label, _)| label);

    let view_entity = cx.entity();
    let filter_button = Button::new("log-level-filter")
        .small()
        .outline()
        .label(format!("级别：{filter_label}"))
        .dropdown_menu(move |menu, _, _| {
            LEVELS.into_iter().fold(menu, |menu, (label, level)| {
                menu.item(PopupMenuItem::new(label).on_click({
                    let view = view_entity.clone();
                    move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.log_filter = level;
                            cx.notify();
                        });
                    }
                }))
            })
        });

    let mut content = v_flex().size_full().gap_2().child(
        h_flex()
            .justify_between()
            .child(page_title("日志"))
            .child(filter_button),
    );

    // 过滤后的日志按原始顺序（旧→新）拼进一个 Markdown 代码块：逐字保真、等宽、只解析一次。
    // 代价是失去按级别着色；scrollable 模式内部用虚拟列表渲染，长日志不卡。
    let lines: Vec<String> = view
        .state
        .logs
        .iter()
        .filter(|log| filter.is_none_or(|level| log.level == level))
        .map(|log| format!("[{}] {}", log.level, log.payload))
        .collect();
    if lines.is_empty() {
        let empty = super::EmptyState::new(
            gpui_component::IconName::SquareTerminal,
            "没有符合条件的日志",
            if filter.is_some() {
                "当前级别过滤下暂无日志，可清除过滤或等待新日志。"
            } else {
                "内核产生日志后会显示在这里。"
            },
        );
        content = content.child(if filter.is_some() {
            empty.action(
                Button::new("clear-log-filter")
                    .label("清除过滤")
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
        content = content.child(
            div().flex_1().min_h_0().child(
                TextView::markdown("log-block", format!("```\n{}\n```", lines.join("\n")))
                    .selectable(true)
                    .scrollable(true)
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone()),
            ),
        );
    }
    content.into_any_element()
}
