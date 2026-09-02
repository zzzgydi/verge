use std::rc::Rc;

use gpui::{prelude::FluentBuilder as _, *};
use gpui_component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex, v_virtual_list,
};
use crate::ui::UiAction;

use crate::{i18n::tr, view::MainView};

use super::page_title;

/// 组标题行高。虚拟列表要求渲染行高与 item_sizes 逐像素一致。
const GROUP_ROW_HEIGHT: f32 = 28.;
/// 节点行高。
const NODE_ROW_HEIGHT: f32 = 34.;

/// 代理页拍平后的行：组标题行 + 节点行。
enum ProxyRow {
    Group { name: String, kind: String },
    Node { group: String, proxy: String },
}

impl ProxyRow {
    fn height(&self) -> Pixels {
        match self {
            Self::Group { .. } => px(GROUP_ROW_HEIGHT),
            Self::Node { .. } => px(NODE_ROW_HEIGHT),
        }
    }
}

fn proxy_rows(view: &MainView) -> Vec<ProxyRow> {
    view.state
        .proxy_groups
        .iter()
        .flat_map(|group| {
            std::iter::once(ProxyRow::Group {
                name: group.name.clone(),
                kind: group.kind.clone(),
            })
            .chain(group.members.iter().map(|proxy| ProxyRow::Node {
                group: group.name.clone(),
                proxy: proxy.clone(),
            }))
        })
        .collect()
}

/// 延迟分级颜色：<200ms 绿、<800ms 黄，其余（含超时、未测）灰。
fn delay_color(delay: Option<u32>, cx: &App) -> Hsla {
    match delay {
        Some(ms) if ms < 200 => cx.theme().success,
        Some(ms) if ms < 800 => cx.theme().warning,
        _ => cx.theme().muted_foreground,
    }
}

fn render_row(row: &ProxyRow, view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    match row {
        ProxyRow::Group { name, kind } => div()
            .h(px(GROUP_ROW_HEIGHT))
            .flex()
            .items_center()
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{name} · {kind}")),
            )
            .into_any_element(),
        ProxyRow::Node { group, proxy } => {
            let selected = view
                .state
                .proxy_groups
                .iter()
                .find(|candidate| candidate.name == *group)
                .and_then(|candidate| candidate.selected.as_deref())
                == Some(proxy.as_str());
            let delay = view.state.delays.get(proxy).copied();
            let delay_text = delay.map_or_else(
                || tr(view.lang(), "proxies.test_delay").to_owned(),
                |ms| format!("{ms} ms"),
            );
            let delay_color = delay_color(delay, cx);
            let select_group = group.clone();
            let select_proxy = proxy.clone();
            let test_proxy = proxy.clone();
            let testing = view.state.delay_pending.contains(proxy);
            div()
                .id(SharedString::from(format!("proxy-{group}-{proxy}")))
                .h_flex()
                .w_full()
                .h(px(NODE_ROW_HEIGHT))
                .px_3()
                .gap_2()
                .items_center()
                .justify_between()
                .rounded(cx.theme().radius)
                .when(selected, |this| {
                    this.bg(cx.theme().list_active)
                        .border_1()
                        .border_color(cx.theme().list_active_border)
                })
                .when(!selected, |this| {
                    this.hover(|this| this.bg(cx.theme().list_hover))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.dispatch(
                        UiAction::SelectProxy {
                            group: select_group.clone(),
                            proxy: select_proxy.clone(),
                        },
                        cx,
                    );
                }))
                .child(
                    div().flex_1().overflow_x_hidden().child(
                        super::selectable_text(
                            format!("proxy-name-{group}-{proxy}"),
                            proxy.clone(),
                        )
                        .text_sm(),
                    ),
                )
                .child(
                    Button::new(SharedString::from(format!("delay-{group}-{test_proxy}")))
                        .label(delay_text)
                        .xsmall()
                        .ghost()
                        .loading(testing)
                        .when(delay.is_some(), |this| this.text_color(delay_color))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.dispatch(
                                UiAction::TestDelay {
                                    proxy: test_proxy.clone(),
                                    url: "https://www.gstatic.com/generate_204".into(),
                                    timeout_ms: 5_000,
                                },
                                cx,
                            );
                        })),
                )
                .into_any_element()
        }
    }
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let refreshing = view.is_pending(&["proxy_groups"]);
    let header = h_flex()
        .justify_between()
        .child(page_title(tr(lang, "proxies.title")))
        .child(
            Button::new("refresh-proxies")
                .label(tr(lang, "common.refresh"))
                .small()
                .ghost()
                .loading(refreshing)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.dispatch(UiAction::RefreshProxies, cx);
                })),
        );

    if view.state.proxy_groups.is_empty() {
        let body = if refreshing {
            super::skeleton_rows(3).into_any_element()
        } else {
            super::EmptyState::new(
                IconName::Globe,
                tr(lang, "proxies.empty.title"),
                tr(lang, "proxies.empty.desc"),
            )
            .action(
                Button::new("goto-profiles")
                    .label(tr(lang, "proxies.empty.action"))
                    .small()
                    .outline()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.navigate(crate::ui::Page::Profiles, cx);
                    })),
            )
            .into_any_element()
        };
        return v_flex()
            .size_full()
            .gap_4()
            .child(header)
            .child(body)
            .into_any_element();
    }

    let rows = proxy_rows(view);
    let item_sizes = Rc::new(
        rows.iter()
            .map(|row| size(px(0.), row.height()))
            .collect::<Vec<_>>(),
    );
    let view_entity = cx.entity();
    v_flex()
        .size_full()
        .gap_2()
        .child(header)
        .child(div().flex_1().min_h_0().child(v_virtual_list(
            view_entity,
            "proxy-list",
            item_sizes,
            |this, range, _, cx| {
                let rows = proxy_rows(this);
                range
                    .filter_map(|ix| rows.get(ix))
                    .map(|row| render_row(row, this, cx))
                    .collect()
            },
        )))
        .into_any_element()
}
