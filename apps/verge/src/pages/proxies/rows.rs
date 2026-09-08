use super::{
    ProxyPage,
    model::{GROUP_HEIGHT, NODES_HEIGHT, Row},
};
use crate::{i18n::tr, ui::UiAction};
use gpui::{prelude::FluentBuilder as _, *};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};

fn badge(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px_1p5()
        .py_0p5()
        .rounded(px(4.))
        .text_xs()
        .line_height(px(14.))
        .bg(cx.theme().muted.opacity(0.55))
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

pub fn render(row: &Row, page: &ProxyPage, cx: &mut Context<ProxyPage>) -> AnyElement {
    match row {
        Row::Group(index) => {
            let ix = *index;
            let group = &page.snapshot.groups[ix];
            let expanded = if page.query.trim().is_empty() {
                page.expanded.contains(&group.name)
            } else {
                !page.filter_collapsed.contains(&group.name)
            };
            let label = format!("proxy-group-{ix}");
            div()
                .h(px(GROUP_HEIGHT))
                .pb_3()
                .child(
                    h_flex()
                        .id(SharedString::from(label.clone()))
                        .debug_selector(move || label.clone())
                        .size_full()
                        .px_4()
                        .gap_3()
                        .rounded_lg()
                        .bg(cx.theme().muted.opacity(0.35))
                        .border_1()
                        .border_color(cx.theme().border)
                        .cursor_pointer()
                        .hover(|s| s.bg(cx.theme().list_hover))
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle(ix, cx)))
                        .child(
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size_4()
                            .text_color(cx.theme().muted_foreground),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_semibold()
                                        .truncate()
                                        .child(group.name.clone()),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .min_w_0()
                                        .child(badge(group.kind.clone(), cx))
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_xs()
                                                .truncate()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(
                                                    group
                                                        .selected
                                                        .clone()
                                                        .unwrap_or_else(|| "—".into()),
                                                ),
                                        ),
                                ),
                        )
                        .child(
                            Button::new(SharedString::from(format!("locate-{ix}")))
                                .debug_selector(move || format!("locate-{ix}"))
                                .label(tr(page.lang, "proxies.locate"))
                                .xsmall()
                                .ghost()
                                .disabled(group.selected.is_none())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.locate(ix, window, cx);
                                })),
                        )
                        .child(
                            div()
                                .min_w(px(28.))
                                .text_right()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(group.members.len().to_string()),
                        ),
                )
                .into_any_element()
        }
        Row::Nodes { group, members } => {
            let mut row = h_flex()
                .w_full()
                .h(px(NODES_HEIGHT))
                .pb_2()
                .gap_2()
                .items_stretch();
            for &member in members {
                row = row.child(card(*group, member, page, cx));
            }
            for _ in members.len()..page.columns {
                row = row.child(div().flex_1().min_w_0());
            }
            row.into_any_element()
        }
    }
}

fn card(group: usize, member: usize, page: &ProxyPage, cx: &mut Context<ProxyPage>) -> AnyElement {
    let entry = &page.snapshot.groups[group];
    let proxy = &entry.members[member];
    let detail = page.snapshot.proxies.get(proxy);
    let selected = entry.selected.as_ref() == Some(proxy);
    let selectable =
        detail.is_some() && matches!(entry.kind.as_str(), "Selector" | "URLTest" | "Fallback");
    let delay = page
        .delays
        .get(proxy)
        .copied()
        .or_else(|| detail.and_then(|p| p.delay));
    let delay_text = match delay {
        Some(0) => tr(page.lang, "proxies.timeout").into(),
        Some(ms) => format!("{ms} ms"),
        None => tr(page.lang, "proxies.test_delay").into(),
    };
    let select_group = entry.name.clone();
    let select_proxy = proxy.clone();
    let test_proxy = proxy.clone();
    let id = format!("proxy-card-{group}-{member}");
    let mut metadata = h_flex()
        .gap_1()
        .min_w_0()
        .overflow_hidden()
        .child(badge(
            detail.map_or("Unknown", |p| p.kind.as_str()).to_owned(),
            cx,
        ))
        .child(badge(
            match detail.and_then(|p| p.udp) {
                Some(true) => "UDP",
                Some(false) => "UDP ×",
                None => "UDP ?",
            },
            cx,
        ));
    if let Some(details) = detail {
        for (enabled, name) in [
            (details.xudp, "XUDP"),
            (details.tfo, "TFO"),
            (details.mptcp, "MPTCP"),
            (details.smux, "SMUX"),
        ] {
            if enabled {
                metadata = metadata.child(badge(name, cx));
            }
        }
        if let Some(selected) = &details.selected {
            metadata = metadata.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(selected.clone()),
            );
        }
    }
    h_flex()
        .id(SharedString::from(id.clone()))
        .debug_selector(move || id.clone())
        .flex_1()
        .min_w_0()
        .h_full()
        .px_3()
        .gap_2()
        .rounded_lg()
        .border_1()
        .border_color(if selected {
            cx.theme().list_active_border
        } else {
            cx.theme().border.opacity(0.7)
        })
        .bg(if selected {
            cx.theme().list_active
        } else {
            cx.theme().background
        })
        .when(selectable, |this| {
            this.cursor_pointer()
                .hover(|this| this.bg(cx.theme().list_hover))
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            if selectable {
                this.dispatch(
                    UiAction::SelectProxy {
                        group: select_group.clone(),
                        proxy: select_proxy.clone(),
                    },
                    cx,
                );
            }
        }))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .truncate()
                                .child(proxy.clone()),
                        )
                        .when(selected, |this| {
                            this.child(Icon::new(IconName::CircleCheck).size_4())
                        }),
                )
                .child(metadata),
        )
        .child(
            Button::new(SharedString::from(format!("delay-{group}-{member}")))
                .debug_selector(move || format!("delay-{group}-{member}"))
                .label(delay_text)
                .xsmall()
                .ghost()
                .disabled(detail.is_none())
                .loading(page.pending.contains(proxy))
                .when_some(delay, |this, ms| {
                    this.text_color(match ms {
                        0 => cx.theme().danger,
                        1..200 => cx.theme().success,
                        200..800 => cx.theme().warning,
                        _ => cx.theme().muted_foreground,
                    })
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
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
