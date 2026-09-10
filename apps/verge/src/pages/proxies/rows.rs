use super::{
    ProxyPage,
    model::{GROUP_HEIGHT, NODES_HEIGHT, Row},
};
use crate::{appearance::metrics, i18n::tr, ui::UiAction};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{prelude::FluentBuilder as _, *};

fn badge(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(px(4.))
        .rounded(px(4.))
        .text_size(px(12.))
        .line_height(px(16.))
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
                .pb(px(metrics::ITEM_GAP))
                .child(
                    h_flex()
                        .id(SharedString::from(label.clone()))
                        .debug_selector(move || label.clone())
                        .size_full()
                        .px(px(metrics::ITEM_INSET))
                        .gap(px(metrics::ITEM_GAP))
                        .rounded_lg()
                        .bg(cx.theme().muted.opacity(0.35))
                        .border_1()
                        .border_color(cx.theme().border)
                        .cursor_pointer()
                        .hover(|s| s.bg(cx.theme().list_hover))
                        .on_click(cx.listener(move |this, _, _, cx| this.toggle(ix, cx)))
                        .child(
                            h_flex()
                                .size(px(20.))
                                .flex_shrink_0()
                                .justify_center()
                                .child(
                                    Icon::new(if expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .size(px(16.))
                                    .text_color(cx.theme().muted_foreground),
                                ),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(px(14.))
                                        .line_height(px(20.))
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
                                                .text_size(px(12.))
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
                                .small()
                                .debug_selector(move || format!("locate-{ix}"))
                                .label(tr(page.lang, "proxies.locate"))
                                .h(px(metrics::COMPACT_CONTROL))
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
                                .text_sm()
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
                .pb(px(metrics::ITEM_GAP))
                .gap(px(metrics::ITEM_GAP))
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
    let error = page.delay_errors.get(proxy);
    let pending = page.pending.contains(proxy);
    let delay_view = DelayPresentation::new(pending, delay, error, page.lang, cx);
    let select_group = entry.name.clone();
    let select_proxy = proxy.clone();
    let test_proxy = proxy.clone();
    let id = format!("proxy-card-{group}-{member}");
    let mut metadata = h_flex().gap_1p5().min_w_0().overflow_hidden().child(badge(
        detail.map_or("Unknown", |p| p.kind.as_str()).to_owned(),
        cx,
    ));
    if let Some(details) = detail {
        for (enabled, name) in [
            (details.udp == Some(true), "UDP"),
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
        .px(px(metrics::ITEM_INSET))
        .gap(px(metrics::ITEM_GAP))
        .rounded_lg()
        .border_1()
        .border_color(if selected {
            selection_color(cx)
        } else {
            cx.theme().border.opacity(0.7)
        })
        .bg(if selected {
            cx.theme().list_active
        } else {
            cx.theme().background
        })
        .when(selectable, |this| {
            this.cursor_pointer().when(!selected, |this| {
                this.hover(|this| this.bg(cx.theme().list_hover))
            })
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
            h_flex()
                .debug_selector(move || format!("proxy-marker-{group}-{member}"))
                .size(px(20.))
                .flex_shrink_0()
                .justify_center()
                .rounded_full()
                .border_1()
                .border_color(if selected {
                    selection_color(cx)
                } else {
                    cx.theme().border
                })
                .when(selected, |this| {
                    this.bg(selection_color(cx)).child(
                        Icon::new(IconName::Check)
                            .size(px(12.))
                            .text_color(cx.theme().tiles),
                    )
                }),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .debug_selector(move || format!("proxy-name-{group}-{member}"))
                        .flex_1()
                        .min_w_0()
                        .text_size(px(14.))
                        .line_height(px(20.))
                        .font_medium()
                        .truncate()
                        .child(proxy.clone()),
                )
                .child(metadata),
        )
        .child(
            Button::new(SharedString::from(format!("delay-{group}-{member}")))
                .debug_selector(move || format!("delay-{group}-{member}"))
                .accessibility_label(format!("{}: {}", proxy, delay_view.label))
                .min_w(px(72.))
                .h(px(metrics::COMPACT_CONTROL))
                .flex_shrink_0()
                .custom(delay_view.button_style(cx))
                // Keep the foreground on the content as well: pointer/disabled styles
                // must never replace the color of a newly received measurement.
                .child(
                    div()
                        .text_size(px(12.))
                        .font_semibold()
                        .text_color(delay_view.color)
                        .child(delay_view.label),
                )
                .disabled(detail.is_none())
                .loading(pending)
                .when_some(error, |this, error| {
                    this.tooltip(format!(
                        "{}: {}",
                        tr(page.lang, "proxies.delay_retry"),
                        error.message
                    ))
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

fn selection_color(cx: &App) -> Hsla {
    if cx.theme().is_dark() {
        rgb(0xe1e5f2).into()
    } else {
        rgb(0x4b5f86).into()
    }
}

/// Text and color are derived together, including while the pointer stays on the button.
struct DelayPresentation {
    label: String,
    color: Hsla,
}

impl DelayPresentation {
    fn new(
        pending: bool,
        delay: Option<u32>,
        error: Option<&crate::domain::AppError>,
        lang: crate::i18n::Lang,
        cx: &App,
    ) -> Self {
        let (label, color) = if pending {
            (
                tr(lang, "proxies.testing").into(),
                cx.theme().muted_foreground,
            )
        } else if let Some(error) = error {
            (
                tr(
                    lang,
                    if error.code == crate::domain::ErrorCode::RequestTimeout {
                        "proxies.timeout"
                    } else {
                        "proxies.delay_failed"
                    },
                )
                .into(),
                cx.theme().danger,
            )
        } else {
            match delay {
                Some(0) => (tr(lang, "proxies.timeout").into(), cx.theme().danger),
                Some(ms) => (
                    format!("{ms} ms"),
                    match ms {
                        1..200 => cx.theme().success,
                        200..800 => cx.theme().warning,
                        _ => cx.theme().muted_foreground,
                    },
                ),
                None => (
                    tr(lang, "proxies.test_delay").into(),
                    cx.theme().muted_foreground,
                ),
            }
        };
        Self { label, color }
    }

    fn button_style(&self, cx: &App) -> ButtonCustomVariant {
        ButtonCustomVariant::new(cx)
            .foreground(self.color)
            .color(self.color.opacity(0.09))
            .hover(self.color.opacity(0.16))
            .active(self.color.opacity(0.22))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use std::{cell::RefCell, rc::Rc};

    struct DelayProbe {
        pending: bool,
        delay: Option<u32>,
        color: Rc<RefCell<Option<Hsla>>>,
    }
    impl Render for DelayProbe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let presentation =
                DelayPresentation::new(self.pending, self.delay, None, crate::i18n::Lang::En, cx);
            let color = self.color.clone();
            Button::new("delay-probe")
                .debug_selector(|| "delay-probe".into())
                .w(px(120.))
                .h(px(metrics::COMPACT_CONTROL))
                .custom(presentation.button_style(cx))
                .loading(self.pending)
                .label(presentation.label)
                .child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            *color.borrow_mut() = Some(window.text_style().color);
                        },
                    )
                    .size(px(1.)),
                )
                .on_click(|_, _, _| {})
        }
    }

    #[gpui_kit::test]
    fn completed_delay_keeps_its_color_under_hover_and_press(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let color = Rc::new(RefCell::new(None));
        let probe_color = color.clone();
        let (probe, cx) = cx.add_window_view(|_, _| DelayProbe {
            pending: true,
            delay: None,
            color: probe_color,
        });
        cx.run_until_parked();
        let bounds = cx.debug_bounds("delay-probe").unwrap();
        cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
        // The pointer does not move when a result arrives.
        cx.update(|_, cx| {
            probe.update(cx, |probe, cx| {
                probe.pending = false;
                probe.delay = Some(42);
                cx.notify();
            })
        });
        cx.run_until_parked();
        let expected = cx.update(|_, cx| cx.theme().success);
        assert_eq!(*color.borrow(), Some(expected));
        cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        assert_eq!(*color.borrow(), Some(expected));
        cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(point(px(300.), px(300.)), None, Modifiers::default());
        cx.run_until_parked();
        assert_eq!(*color.borrow(), Some(expected));
    }
}
