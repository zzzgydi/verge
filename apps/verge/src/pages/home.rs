pub mod telemetry;

use super::components::{PageHeader, panel};
use crate::domain::RunMode;
use crate::ui::{CoreStatus, Page, UiAction};
use crate::{
    i18n::{Lang, tr},
    view::MainView,
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
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
        .flex_1()
        .min_w_0()
        .justify_between()
        .child(
            v_flex()
                .gap_4()
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            div()
                                .size_10()
                                .rounded_xl()
                                .bg(cx.theme().accent)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(Icon::new(IconName::Globe).size_5()),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .text_xs()
                                .text_color(status_color)
                                .child(div().size(px(6.)).rounded_full().bg(status_color))
                                .child(core_status),
                        ),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr(lang, "home.active_profile")),
                        )
                        .child(div().text_xl().font_medium().truncate().child(profile_name))
                        .when_some(profile, |this, profile| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} {}",
                                        tr(lang, "profiles.updated_at"),
                                        crate::format::updated_at(lang, profile.updated_at)
                                    )),
                            )
                        })
                        .when(profile.is_none(), |this| {
                            this.child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr(lang, "home.profile_hint")),
                            )
                        }),
                ),
        )
        .child(
            v_flex()
                .gap_4()
                .mt_4()
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
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
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr(lang, "home.run_mode")),
                        )
                        .child(super::components::mode_selector(view, cx)),
                )
                .child(
                    Button::new("manage-profiles")
                        .small()
                        .h(px(crate::appearance::metrics::CONTROL))
                        .primary()
                        .w_full()
                        .label(tr(lang, "home.manage_profiles"))
                        .on_click(cx.listener(|this, _, _, cx| this.navigate(Page::Profiles, cx))),
                ),
        );

    let quick_links = [
        (Page::Proxies, IconName::Globe, "proxies.title"),
        (Page::Rules, IconName::BookOpen, "rules.title"),
        (Page::Logs, IconName::SquareTerminal, "logs.title"),
    ]
    .map(|(page, icon, title)| {
        Button::new(format!("open-{page:?}"))
            .small()
            .outline()
            .flex_1()
            .h(px(40.))
            .icon(Icon::new(icon).size_4())
            .label(tr(lang, title))
            .on_click(cx.listener(move |this, _, _, cx| this.navigate(page, cx)))
    });

    v_flex()
        .gap_4()
        .child(
            PageHeader::new(tr(lang, "home.title")).child(
                Button::new("refresh-home")
                    .outline()
                    .small()
                    .h(px(crate::appearance::metrics::COMPACT_CONTROL))
                    .icon(gpui_kit::assets::IconName::RefreshCw)
                    .label(tr(lang, "common.refresh"))
                    .loading(view.is_pending(&["runtime_settings", "mode", "system_proxy"]))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.dispatch(UiAction::RefreshHome, cx)),
                    ),
            ),
        )
        .child(
            h_flex()
                .gap_4()
                .items_stretch()
                .child(connection)
                .child(div().flex_1().min_w_0().child(view.telemetry.clone())),
        )
        .child(overview_details(view, cx))
        .child(h_flex().gap_3().children(quick_links))
        .into_any_element()
}

fn overview_details(view: &MainView, cx: &mut Context<MainView>) -> impl IntoElement {
    let lang = view.lang();
    let unknown = tr(lang, "common.unknown");
    let enabled = |value: Option<bool>| match value {
        Some(true) => tr(lang, "home.enabled"),
        Some(false) => tr(lang, "home.disabled"),
        None => unknown,
    };
    let stat = |label: &str, value: String| {
        h_flex()
            .gap_3()
            .justify_between()
            .text_sm()
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(label.to_owned()),
            )
            .child(div().font_medium().child(value))
    };
    let traffic = view.state.connections.as_ref();
    let network = view.state.network_settings.as_ref();
    let port = view
        .state
        .runtime_settings
        .as_ref()
        .map(|s| s.system_proxy_endpoint.port.to_string())
        .unwrap_or_else(|| "—".into());
    let network_card = panel(cx)
        .flex_1()
        .min_w_0()
        .gap_3()
        .child(div().font_medium().child(tr(lang, "home.network")))
        .child(stat(tr(lang, "home.port"), port))
        .child(stat("TUN", enabled(network.map(|s| s.tun_enabled)).into()))
        .child(stat("DNS", enabled(network.map(|s| s.dns_enabled)).into()))
        .child(stat(
            "IPv6",
            enabled(network.map(|s| s.ipv6_enabled)).into(),
        ))
        .child(stat(
            tr(lang, "home.total_upload"),
            traffic
                .map(|s| crate::format::bytes(s.upload_total))
                .unwrap_or_else(|| "—".into()),
        ))
        .child(stat(
            tr(lang, "home.total_download"),
            traffic
                .map(|s| crate::format::bytes(s.download_total))
                .unwrap_or_else(|| "—".into()),
        ));
    let mut routes = panel(cx)
        .flex_1()
        .min_w_0()
        .gap_3()
        .child(
            h_flex()
                .justify_between()
                .child(div().font_medium().child(tr(lang, "home.routes")))
                .child(
                    Button::new("home-view-routes")
                        .small()
                        .ghost()
                        .label(tr(lang, "proxies.title"))
                        .on_click(cx.listener(|this, _, _, cx| this.navigate(Page::Proxies, cx))),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!(
                    "{} {} · {} {}",
                    view.state.rules.len(),
                    tr(lang, "home.rule_count"),
                    view.state.providers.len(),
                    tr(lang, "rules.providers")
                )),
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
        .take(4)
        .collect();
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
    for group in groups {
        let selected = group.selected.as_deref().unwrap_or("—");
        let delay = view.state.delays.get(selected).copied().or_else(|| {
            view.state
                .proxies
                .proxies
                .get(selected)
                .and_then(|p| p.delay)
        });
        routes = routes.child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(group.name.clone()),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(selected.to_owned()),
                        )
                        .when_some(delay, |row, delay| {
                            row.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if delay == 0 {
                                        tr(lang, "proxies.timeout").into()
                                    } else {
                                        format!("{delay} ms")
                                    }),
                            )
                        }),
                ),
        );
    }
    h_flex()
        .items_stretch()
        .gap_4()
        .child(network_card)
        .child(routes)
}
