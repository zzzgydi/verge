use gpui::*;
use gpui_component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonGroup, ButtonVariants as _},
    group_box::GroupBox,
    h_flex,
    label::Label,
    switch::Switch,
    v_flex,
};
use verge_domain::RunMode;
use verge_ui::{CoreStatus, UiAction};

use crate::{
    format,
    i18n::{Lang, tr},
    view::MainView,
};

use super::page_title;

/// 运行模式的界面文案。
pub fn mode_label(lang: Lang, mode: RunMode) -> &'static str {
    let key = match mode {
        RunMode::Rule => "home.mode.rule",
        RunMode::Global => "home.mode.global",
        RunMode::Direct => "home.mode.direct",
    };
    tr(lang, key)
}

/// 概览页统计卡片。
fn stat_tile(label: &'static str, value: String, cx: &Context<MainView>) -> impl IntoElement {
    v_flex()
        .flex_1()
        .gap_1()
        .p_4()
        .rounded(cx.theme().radius_lg)
        .bg(cx.theme().tiles)
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            super::selectable_text(format!("stat-{label}"), value)
                .text_lg()
                .font_semibold(),
        )
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    const MODES: [RunMode; 3] = [RunMode::Rule, RunMode::Global, RunMode::Direct];

    let lang = view.lang();
    let core_status = match view.state.core_status {
        CoreStatus::Running => tr(lang, "home.core.running"),
        CoreStatus::Offline => tr(lang, "home.core.offline"),
        CoreStatus::Unknown => tr(lang, "common.unknown"),
    }
    .to_owned();
    let proxy_enabled = view.state.system_proxy_enabled();
    let proxy_settings = view.state.runtime_settings.clone();
    let proxy_available = proxy_settings.is_some();
    let upload = view
        .state
        .traffic
        .as_ref()
        .map_or_else(|| "—".to_owned(), |traffic| format::rate(traffic.up));
    let download = view
        .state
        .traffic
        .as_ref()
        .map_or_else(|| "—".to_owned(), |traffic| format::rate(traffic.down));
    let memory = view.state.memory.as_ref().map_or_else(
        || tr(lang, "common.unknown").to_owned(),
        |memory| format::bytes(memory.inuse),
    );
    let connections = view.state.connections.as_ref().map_or_else(
        || "—".to_owned(),
        |connections| connections.connection_count.to_string(),
    );

    let mode_group = ButtonGroup::new("mode-group")
        .outline()
        .small()
        .children(MODES.map(|mode| {
            Button::new(format!("mode-{mode:?}"))
                .label(mode_label(lang, mode))
                .selected(view.state.mode == Some(mode))
        }))
        .on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
            if let Some(&ix) = clicked.first() {
                this.dispatch(UiAction::SetMode(MODES[ix]), cx);
            }
        }));

    v_flex()
        .gap_4()
        .child(
            h_flex()
                .justify_between()
                .child(page_title(tr(lang, "home.title")))
                .child(
                    Button::new("refresh-home")
                        .label(tr(lang, "common.refresh"))
                        .small()
                        .ghost()
                        .loading(view.is_pending(&["runtime_settings", "mode", "system_proxy"]))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dispatch(UiAction::RefreshHome, cx);
                        })),
                ),
        )
        .child(
            h_flex()
                .gap_3()
                .child(stat_tile(tr(lang, "home.tile.core"), core_status, cx))
                .child(stat_tile(
                    tr(lang, "home.tile.upload"),
                    format!("↑ {upload}"),
                    cx,
                ))
                .child(stat_tile(
                    tr(lang, "home.tile.download"),
                    format!("↓ {download}"),
                    cx,
                ))
                .child(stat_tile(tr(lang, "home.tile.memory"), memory, cx))
                .child(stat_tile(
                    tr(lang, "home.tile.connections"),
                    connections,
                    cx,
                )),
        )
        .child(
            GroupBox::new()
                .id("home-proxy-control")
                .title(tr(lang, "home.proxy_control"))
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .items_center()
                                .child(Label::new(tr(lang, "home.run_mode")))
                                .child(mode_group),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .items_center()
                                .child(Label::new(tr(lang, "home.system_proxy")))
                                .child(
                                    Switch::new("system-proxy")
                                        .checked(proxy_enabled)
                                        .disabled(!proxy_available)
                                        .on_click(cx.listener(
                                            move |this, checked: &bool, _, cx| {
                                                if let Some(settings) = &proxy_settings {
                                                    this.dispatch(
                                                        UiAction::SetSystemProxy {
                                                            enabled: *checked,
                                                            services: settings
                                                                .system_proxy_services
                                                                .clone(),
                                                            endpoint: settings
                                                                .system_proxy_endpoint
                                                                .clone(),
                                                        },
                                                        cx,
                                                    );
                                                }
                                            },
                                        )),
                                ),
                        ),
                ),
        )
        .into_any_element()
}
