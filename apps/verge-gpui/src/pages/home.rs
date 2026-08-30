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

use crate::{format, view::MainView};

use super::page_title;

/// 运行模式的中文文案。
pub fn mode_label(mode: RunMode) -> &'static str {
    match mode {
        RunMode::Rule => "规则",
        RunMode::Global => "全局",
        RunMode::Direct => "直连",
    }
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

    let core_status = match view.state.core_status {
        CoreStatus::Running => "运行中",
        CoreStatus::Offline => "离线",
        CoreStatus::Unknown => "未知",
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
    let memory = view
        .state
        .memory
        .as_ref()
        .map_or_else(|| "未知".to_owned(), |memory| format::bytes(memory.inuse));

    let mode_group = ButtonGroup::new("mode-group")
        .outline()
        .small()
        .children(MODES.map(|mode| {
            Button::new(format!("mode-{mode:?}"))
                .label(mode_label(mode))
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
            h_flex().justify_between().child(page_title("概览")).child(
                Button::new("refresh-home")
                    .label("刷新")
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
                .child(stat_tile("内核状态", core_status, cx))
                .child(stat_tile("上传速率", format!("↑ {upload}"), cx))
                .child(stat_tile("下载速率", format!("↓ {download}"), cx))
                .child(stat_tile("内存占用", memory, cx)),
        )
        .child(
            GroupBox::new()
                .id("home-proxy-control")
                .title("代理控制")
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .items_center()
                                .child(Label::new("运行模式"))
                                .child(mode_group),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .items_center()
                                .child(Label::new("系统代理"))
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
