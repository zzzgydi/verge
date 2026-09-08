pub mod telemetry;

use super::components::{PageHeader, panel};
use crate::domain::RunMode;
use crate::ui::{CoreStatus, Page, UiAction};
use crate::{
    i18n::{Lang, tr},
    view::MainView,
};
use gpui::{prelude::FluentBuilder as _, *};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    switch::Switch,
    v_flex,
};

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
    let proxy_settings = view.state.runtime_settings.clone();
    let proxy_available = proxy_settings.is_some();

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
                                .disabled(!proxy_available)
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                    if let Some(settings) = &proxy_settings {
                                        this.dispatch(
                                            UiAction::SetSystemProxy {
                                                enabled: *checked,
                                                services: settings.system_proxy_services.clone(),
                                                endpoint: settings.system_proxy_endpoint.clone(),
                                            },
                                            cx,
                                        );
                                    }
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
                    .icon(IconName::Redo)
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
        .child(h_flex().gap_3().children(quick_links))
        .into_any_element()
}
