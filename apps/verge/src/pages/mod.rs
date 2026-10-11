pub mod ai;
pub mod components;
pub mod connections;
pub mod filters;
pub mod home;
pub mod logs;
pub mod profiles;
pub mod proxies;
pub mod rules;
pub mod settings;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, StyledExt as _, skeleton::Skeleton, text::TextView,
};
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, IntoElement, ParentElement as _, RenderOnce,
    SharedString, Styled, Window, div, prelude::FluentBuilder as _,
};

use crate::view::MainView;

/// 次要说明文字样式。
pub fn muted(text: impl Into<SharedString>, cx: &Context<MainView>) -> Div {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// 可选中复制的展示文字；ElementId 用域 ID（节点名、配置 id 等），不用列表索引。
pub fn selectable_text(id: impl Into<ElementId>, text: impl Into<SharedString>) -> TextView {
    let text = text.into();
    TextView::markdown(id, literal_markdown(&text))
        .selectable(true)
        .selection_format(gpui_kit::component::text::SelectionFormat::Plain)
}

// The locked TextView has no plain-text constructor. Character references keep
// punctuation and indentation literal without creating links, images or blocks.
// Copy uses the rendered text, never this escaped source.
fn literal_markdown(text: &str) -> String {
    use std::fmt::Write as _;
    let mut source = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\n' {
            source.push_str("\\\n");
        } else if ch.is_ascii_punctuation() || ch.is_ascii_whitespace() {
            write!(source, "&#{};", ch as u32).unwrap();
        } else {
            source.push(ch);
        }
    }
    source
}

/// 数据加载中的占位骨架行。
pub fn skeleton_rows(rows: usize) -> impl IntoElement {
    gpui_kit::component::v_flex()
        .gap_2()
        .children((0..rows).map(|_| Skeleton::new().h_8().w_full()))
}

/// 统一空态：图标 + 标题 + 一句说明 + 可选的下一步动作。
#[derive(IntoElement)]
pub struct EmptyState {
    icon: IconName,
    title: SharedString,
    description: SharedString,
    action: Option<AnyElement>,
}

impl EmptyState {
    pub fn new(
        icon: IconName,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
    ) -> Self {
        Self {
            icon,
            title: title.into(),
            description: description.into(),
            action: None,
        }
    }

    /// 空态的下一步动作按钮。
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl RenderOnce for EmptyState {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        gpui_kit::component::v_flex()
            .w_full()
            .items_center()
            .justify_center()
            .gap_2()
            .py_12()
            .child(
                Icon::new(self.icon)
                    .size_8()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(div().text_sm().font_semibold().child(self.title))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.description),
            )
            .when_some(self.action, |this, action| {
                this.child(div().pt_2().child(action))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::literal_markdown;
    use gpui_kit::component::text::{SelectionFormat, TextViewState};
    use gpui_kit::{AppContext as _, TestAppContext};

    #[gpui_kit::test]
    fn selectable_values_preserve_literal_text_and_plain_copy(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for value in [
            "**process**_name_ [host](https://example.test) `payload`",
            r"C:\Users\[name]\a_b &amp; <tag> ![image](https://example.test/x)",
            "www.example.test user@example.test https://example.test/a?q=1&b=2",
            "  中文路径 /应用程序/测试.app  ",
            "# heading\n    indented\n- item\n| a | b |",
        ] {
            let source = literal_markdown(value);
            // Validate the actual locked TextView parser and clipboard format,
            // not merely the escape function's output.
            let state = cx.new(|cx| {
                TextViewState::markdown(&source, cx).selection_format(SelectionFormat::Plain)
            });
            cx.run_until_parked();
            state.update(cx, |state, cx| {
                // TextView includes one paragraph terminator in plain select-all.
                let expected = format!("{value}\n");
                assert_eq!(state.rendered_text().as_str(), expected);
                state.select_all(cx);
                assert_eq!(state.selected_text(), expected);
            });
        }
    }
}
