pub mod telemetry;

use super::components::{Metric, PageHeader, panel};
use crate::domain::{ProfileSource, RunMode};
use crate::ui::{CoreStatus, Page, UiAction};
use crate::{
    i18n::{Lang, tr},
    view::MainView,
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    switch::Switch,
    v_flex,
};
use gpui_kit::{prelude::FluentBuilder as _, *};

pub fn mode_label(lang: Lang, mode: RunMode) -> &'static str {
    tr(
        lang,
        match mode {
            RunMode::Rule => "home.mode.rule",
            RunMode::Global => "home.mode.global",
            RunMode::Direct => "home.mode.direct",
        },
    )
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let (core_status, status_color) = match view.state.core_status {
        CoreStatus::Running => (tr(lang, "home.core.running"), cx.theme().success),
        CoreStatus::Offline => (tr(lang, "home.core.offline"), cx.theme().muted_foreground),
        CoreStatus::Unknown => (tr(lang, "common.unknown"), cx.theme().muted_foreground),
    };
    let profile = view
        .state
        .profiles
        .iter()
        .find(|p| Some(&p.id) == view.state.selected_profile.as_ref());
    let profile_name = profile.map_or_else(
        || tr(lang, "home.no_profile").to_owned(),
        |p| p.name.clone(),
    );
    let proxy_available =
        view.state.runtime_settings.is_some() || view.state.system_proxy_enabled();

    let connection = panel(cx)
        .p_5()
        .gap_3()
        .child(
            h_flex()
                .gap_3()
                .child(
                    div()
                        .size_10()
                        .rounded_xl()
                        .bg(cx.theme().accent)
                        .text_color(cx.theme().primary)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(IconName::Globe).size_5()),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr(
                                    lang,
                                    if profile.is_some() {
                                        "home.active_profile"
                                    } else {
                                        "home.profile_hint"
                                    },
                                )),
                        )
                        .child(
                            div()
                                .id("home-profile-name")
                                .debug_selector(|| "home-profile-name".into())
                                .line_height(px(24.))
                                .text_xl()
                                .font_medium()
                                .truncate()
                                .child(profile_name.clone())
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(profile_name.clone())
                                        .build(window, cx)
                                }),
                        )
                        .when_some(profile, |this, profile| {
                            this.child(
                                div()
                                    .text_xs()
                                    .line_height(px(16.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} · {} {}",
                                        tr(
                                            lang,
                                            match profile.source {
                                                ProfileSource::Local => "profiles.source.local",
                                                ProfileSource::Remote { .. } =>
                                                    "profiles.source.remote",
                                            }
                                        ),
                                        tr(lang, "profiles.updated_at"),
                                        crate::format::updated_at(lang, profile.updated_at),
                                    )),
                            )
                        }),
                )
                .child(
                    v_flex()
                        .items_end()
                        .gap_1()
                        .text_xs()
                        .child(
                            h_flex()
                                .gap_2()
                                .text_color(status_color)
                                .child(div().size(px(6.)).rounded_full().bg(status_color))
                                .child(core_status),
                        )
                        .when_some(view.state.mihomo_version.as_ref(), |this, version| {
                            this.child(
                                div()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("Mihomo {version}")),
                            )
                        }),
                )
                .child(
                    Button::new("manage-profiles")
                        .small()
                        .ghost()
                        .label(tr(lang, "home.manage_profiles"))
                        .on_click(cx.listener(|this, _, _, cx| this.navigate(Page::Profiles, cx))),
                ),
        )
        .child(
            h_flex()
                .justify_between()
                .gap_4()
                .pt_3()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .gap_3()
                        .text_sm()
                        .child(tr(lang, "home.system_proxy"))
                        .child(
                            Switch::new("system-proxy")
                                .checked(view.state.system_proxy_enabled())
                                .disabled(
                                    crate::identity::AppChannel::current().is_dev()
                                        || !proxy_available
                                        || view.is_pending(&["system_proxy"]),
                                )
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                    this.dispatch(
                                        UiAction::SetSystemProxy { enabled: *checked },
                                        cx,
                                    );
                                })),
                        ),
                )
                .child(super::components::mode_selector(view, cx)),
        )
        .child(network_status(view, cx));

    v_flex()
        .flex_1()
        .min_h_0()
        .gap_4()
        .child(
            PageHeader::new(tr(lang, "home.title")).child(
                Button::new("refresh-home")
                    .ghost()
                    .small()
                    .h(px(crate::appearance::metrics::COMPACT_CONTROL))
                    .icon(gpui_kit::assets::IconName::RefreshCw)
                    .tooltip(tr(lang, "common.refresh"))
                    .accessibility_label(tr(lang, "common.refresh"))
                    .loading(view.is_pending(&["runtime_settings", "mode", "system_proxy"]))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.dispatch(UiAction::RefreshHome, cx)),
                    ),
            ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .id("home-body")
                .child(
                    v_flex().gap_4().child(connection).child(
                        div()
                            .grid()
                            .grid_cols(2)
                            .gap_4()
                            .items_stretch()
                            .child(overview_details(view, cx))
                            .child(view.telemetry.clone()),
                    ),
                ),
        )
        .into_any_element()
}

fn network_status(view: &MainView, cx: &App) -> impl IntoElement {
    let lang = view.lang();
    let unknown = tr(lang, "common.unknown");
    let enabled = |value: Option<bool>| match value {
        Some(true) => tr(lang, "home.enabled"),
        Some(false) => tr(lang, "home.disabled"),
        None => unknown,
    };
    let stat = |label: &str, value: String| {
        h_flex()
            .min_w_0()
            .gap_2()
            .text_xs()
            .line_height(px(20.))
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(label.to_owned()),
            )
            .child(div().font_medium().child(value))
    };
    let network = view.state.network_settings.as_ref();
    let port = view
        .state
        .runtime_settings
        .as_ref()
        .map(|s| s.system_proxy_endpoint.port.to_string())
        .unwrap_or_else(|| "—".into());
    div()
        .debug_selector(|| "home-network-status".into())
        .grid()
        .grid_cols(4)
        .gap_3()
        .child(stat(tr(lang, "home.port"), port))
        .child(stat("TUN", enabled(network.map(|s| s.tun_enabled)).into()))
        .child(stat("DNS", enabled(network.map(|s| s.dns_enabled)).into()))
        .child(stat(
            "IPv6",
            enabled(network.map(|s| s.ipv6_enabled)).into(),
        ))
}

fn overview_details(view: &MainView, cx: &mut Context<MainView>) -> impl IntoElement {
    let lang = view.lang();
    let mut routes = v_flex().flex_1().gap_3();
    let panel = panel(cx)
        .debug_selector(|| "home-routes-panel".into())
        .min_w_0()
        .min_h(px(440.))
        .p_5()
        .gap_4()
        .child(
            h_flex()
                .h(px(28.))
                .justify_between()
                .child(div().font_medium().child(tr(lang, "home.routes")))
                .child(
                    Button::new("home-view-routes")
                        .small()
                        .ghost()
                        .label(tr(lang, "proxies.title"))
                        .on_click(cx.listener(|this, _, _, cx| this.navigate(Page::Proxies, cx))),
                ),
        );
    let groups: Vec<_> = view
        .state
        .proxies
        .groups
        .iter()
        .filter(|g| {
            view.state.mode != Some(RunMode::Direct)
                && (view.state.mode == Some(RunMode::Global)) == (g.name == "GLOBAL")
                && !view
                    .state
                    .proxies
                    .proxies
                    .get(&g.name)
                    .is_some_and(|p| p.hidden)
        })
        .collect();
    let group_count = groups.len();
    let single_route = groups.len() == 1;
    if single_route || groups.is_empty() {
        routes = routes.justify_center();
    }
    if groups.is_empty() {
        routes = routes.child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(tr(
                    lang,
                    if view.state.mode == Some(RunMode::Direct) {
                        "proxies.direct.desc"
                    } else {
                        "home.no_routes"
                    },
                )),
        );
    }
    for (index, group) in groups.into_iter().take(4).enumerate() {
        let selected = group.selected.as_deref().unwrap_or("—");
        let details = view.state.proxies.proxies.get(selected);
        let delay = view.state.delays.get(selected).copied().or_else(|| {
            view.state
                .proxies
                .proxies
                .get(selected)
                .and_then(|p| p.delay)
        });
        let description = details.map(|details| {
            let mut parts = vec![details.kind.clone()];
            if details.udp == Some(true) {
                parts.push("UDP".into());
            }
            parts.join(" · ")
        });
        let group_name = group.name.clone();
        let selected_name = selected.to_owned();
        routes = routes.child(
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .justify_between()
                        .text_xs()
                        .line_height(px(16.))
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            div()
                                .id(("home-route-group", index))
                                .min_w_0()
                                .truncate()
                                .child(group.name.clone())
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(group_name.clone())
                                        .build(window, cx)
                                }),
                        )
                        .child(div().flex_shrink_0().child(format!(
                            "{} {}",
                            group.members.len(),
                            tr(lang, "home.nodes")
                        ))),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .line_height(px(20.))
                        .child(
                            div()
                                .id(("home-route-node", index))
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .font_medium()
                                .when(single_route, |this| this.text_lg().line_height(px(28.)))
                                .child(selected.to_owned())
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        selected_name.clone(),
                                    )
                                    .build(window, cx)
                                }),
                        )
                        .when_some(delay, |row, delay| {
                            row.child(
                                div()
                                    .text_xs()
                                    .text_color(if delay == 0 {
                                        cx.theme().danger
                                    } else if delay < 200 {
                                        cx.theme().success
                                    } else {
                                        cx.theme().warning
                                    })
                                    .child(if delay == 0 {
                                        tr(lang, "proxies.timeout").into()
                                    } else {
                                        format!("{delay} ms")
                                    }),
                            )
                        }),
                )
                .when_some(description, |this, description| {
                    this.child(
                        div()
                            .text_xs()
                            .line_height(px(16.))
                            .text_color(cx.theme().muted_foreground)
                            .child(description),
                    )
                }),
        );
    }
    panel.child(routes).child(
        h_flex()
            .gap_4()
            .pt_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(Metric::new(
                tr(lang, "home.groups"),
                group_count.to_string(),
            ))
            .child(Metric::new(
                tr(lang, "rules.title"),
                view.state.rules.len().to_string(),
            ))
            .child(Metric::new(
                tr(lang, "rules.providers"),
                view.state.providers.len().to_string(),
            )),
    )
}
