use super::{
    components::{SettingsSection, field, v_form},
    *,
};
use crate::domain::SystemProxySettings;
use gpui_component::{
    WindowExt as _,
    input::{InputState, Textarea, TextareaState},
    notification::Notification,
    scroll::ScrollableElement as _,
};

struct ProxyDialog {
    settings: SystemProxySettings,
    host: Entity<InputState>,
    interval: Entity<InputState>,
    bypass: Entity<TextareaState>,
    pac: Entity<TextareaState>,
}

pub(super) fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let runtime = view.state.runtime_settings.clone();
    let enabled = view.state.system_proxy_enabled();
    let busy = view.is_pending(&["system_proxy", "application_settings_write"]);
    SettingsSection::new()
        .id("settings-system-proxy")
        .title(tr(lang, "settings.group.proxy"))
        .child(
            field()
                .label(tr(lang, "proxy.master"))
                .description(status_text(view, lang))
                .child(
                    h_flex()
                        .justify_end()
                        .gap_3()
                        .child(
                            Button::new("system-proxy-options")
                                .icon(IconName::Settings)
                                .ghost()
                                .small()
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_system_proxy_dialog(window, cx)
                                })),
                        )
                        .child(
                            Switch::new("settings-system-proxy-toggle")
                                .checked(enabled)
                                .disabled(busy || (!enabled && runtime.is_none()))
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                    this.dispatch(
                                        UiAction::SetSystemProxy { enabled: *checked },
                                        cx,
                                    );
                                })),
                        ),
                ),
        )
        .into_any_element()
}
fn status_text(view: &MainView, lang: Lang) -> String {
    let Some(state) = &view.state.system_proxy else {
        return tr(lang, "settings.proxy.not_loaded").into();
    };
    let Some(service) = state.services.first() else {
        return tr(lang, "settings.proxy.not_set").into();
    };
    if state.unified_enabled() {
        if service.auto_proxy.enabled {
            format!("PAC · {}", service.auto_proxy.url.as_deref().unwrap_or(""))
        } else {
            format!(
                "{} · {}:{}",
                tr(lang, "proxy.enabled"),
                service.web.endpoint.host,
                service.web.endpoint.port
            )
        }
    } else if state
        .services
        .iter()
        .any(|s| s.web.enabled || s.secure_web.enabled || s.socks.enabled || s.auto_proxy.enabled)
    {
        tr(lang, "proxy.partial").into()
    } else {
        tr(lang, "proxy.disabled").into()
    }
}

impl MainView {
    pub(crate) fn open_system_proxy_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self
            .state
            .daemon_capabilities
            .iter()
            .any(|c| c == crate::ipc::protocol::UNIFIED_SYSTEM_PROXY)
        {
            window.push_notification(
                Notification::error(tr(self.lang(), "proxy.restart_required")),
                cx,
            );
            return;
        }
        let Some(snapshot) = &self.state.application_settings else {
            return;
        };
        self.pending_proxy_settings = None;
        let settings = snapshot.settings.system_proxy.as_ref().clone();
        let lang = self.lang();
        let status = status_text(self, lang);
        let host = cx.new(|cx| InputState::new(window, cx).default_value(settings.host.clone()));
        let interval = cx.new(|cx| {
            InputState::new(window, cx).default_value(settings.guard_interval_secs.to_string())
        });
        let bypass =
            cx.new(|cx| TextareaState::new(window, cx).default_value(settings.bypass.join("\n")));
        let pac =
            cx.new(|cx| TextareaState::new(window, cx).default_value(settings.pac_script.clone()));
        let draft = cx.new(|_| ProxyDialog {
            settings,
            host,
            interval,
            bypass,
            pac,
        });
        let owner = cx.entity();
        let observation = std::rc::Rc::new(cx.observe(&draft, |_, _, cx| cx.notify()));
        window.open_dialog(cx, move |dialog, _, cx| {
            let _observation = &observation;
            let state = draft.read(cx);
            let mut content = v_form()
                .child(
                    div()
                        .p_3()
                        .mb_2()
                        .rounded_lg()
                        .bg(cx.theme().muted)
                        .text_sm()
                        .child(status.clone()),
                )
                .child(
                    field()
                        .label(tr(lang, "proxy.host"))
                        .child(Input::new(&state.host)),
                );
            for (id, key, value) in [
                ("proxy-pac-mode", "proxy.pac_mode", state.settings.pac_mode),
                ("proxy-guard", "proxy.guard", state.settings.guard_enabled),
            ] {
                let draft = draft.clone();
                content = content.child(
                    field().label(tr(lang, key)).child(
                        h_flex().justify_end().child(
                            div().debug_selector(move || id.into()).child(
                                Switch::new(id)
                                    .checked(value)
                                    .on_click(move |checked, _, cx| {
                                        draft.update(cx, |draft, cx| {
                                            if id == "proxy-pac-mode" {
                                                draft.settings.pac_mode = *checked;
                                            } else {
                                                draft.settings.guard_enabled = *checked;
                                            }
                                            cx.notify();
                                        })
                                    }),
                            ),
                        ),
                    ),
                );
            }
            content = content.child(
                field()
                    .label(tr(lang, "proxy.guard_interval"))
                    .description(tr(lang, "proxy.guard.desc"))
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(div().w(px(96.)).child(
                                Input::new(&state.interval).disabled(!state.settings.guard_enabled),
                            ))
                            .child(tr(lang, "proxy.seconds")),
                    ),
            );
            if !state.settings.pac_mode {
                for (id, key, value) in [
                    (
                        "proxy-default-bypass",
                        "proxy.default_bypass",
                        state.settings.use_default_bypass,
                    ),
                    (
                        "proxy-validate-bypass",
                        "proxy.validate_bypass",
                        state.settings.validate_bypass,
                    ),
                ] {
                    let draft = draft.clone();
                    content = content.child(
                        field().label(tr(lang, key)).child(
                            h_flex()
                                .justify_end()
                                .child(Switch::new(id).checked(value).on_click(
                                    move |checked, _, cx| {
                                        draft.update(cx, |draft, cx| {
                                            if id == "proxy-default-bypass" {
                                                draft.settings.use_default_bypass = *checked;
                                            } else {
                                                draft.settings.validate_bypass = *checked;
                                            }
                                            cx.notify();
                                        });
                                    },
                                )),
                        ),
                    );
                }
                if state.settings.use_default_bypass {
                    content = content.child(h_flex().py_3().gap_2().flex_wrap().children(
                        crate::domain::DEFAULT_PROXY_BYPASS.iter().map(|domain| {
                            div()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .text_xs()
                                .bg(cx.theme().muted)
                                .child(*domain)
                        }),
                    ));
                }
                content = content
                    .child(div().py_2().text_sm().child(tr(lang, "proxy.bypass")))
                    .child(
                        div()
                            .debug_selector(|| "proxy-bypass-editor".into())
                            .child(Textarea::new(&state.bypass).h(px(100.))),
                    )
                    .child(
                        div()
                            .pt_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr(lang, "proxy.bypass.help")),
                    );
            } else {
                content = content
                    .child(div().py_2().text_sm().child(tr(lang, "proxy.pac_script")))
                    .child(Textarea::new(&state.pac).h(px(160.)))
                    .child(
                        div()
                            .pt_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr(lang, "proxy.pac.help")),
                    );
            }
            let save = draft.clone();
            let owner = owner.clone();
            dialog
                .title(tr(lang, "proxy.title"))
                .width(px(640.))
                .child(
                    div()
                        .id("system-proxy-dialog-scroll")
                        .debug_selector(|| "system-proxy-dialog-scroll".into())
                        .h(px(420.))
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .child(content),
                )
                .footer(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("proxy-dialog-cancel")
                                .label(tr(lang, "common.cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("proxy-dialog-save")
                                .debug_selector(|| "proxy-dialog-save".into())
                                .label(tr(lang, "common.save"))
                                .primary()
                                .on_click(move |_, window, cx| {
                                    let draft = save.read(cx);
                                    let mut settings = draft.settings.clone();
                                    settings.host = draft
                                        .host
                                        .read(cx)
                                        .value()
                                        .trim()
                                        .trim_start_matches('[')
                                        .trim_end_matches(']')
                                        .to_owned();
                                    let Ok(interval) =
                                        draft.interval.read(cx).value().trim().parse::<u64>()
                                    else {
                                        window.push_notification(
                                            Notification::error(tr(lang, "proxy.interval.invalid")),
                                            cx,
                                        );
                                        return;
                                    };
                                    settings.guard_interval_secs = interval;
                                    settings.bypass = draft
                                        .bypass
                                        .read(cx)
                                        .value()
                                        .split([',', ';', '\n', '\r'])
                                        .map(str::trim)
                                        .filter(|s| !s.is_empty())
                                        .map(str::to_owned)
                                        .collect();
                                    settings.pac_script = draft.pac.read(cx).value().to_string();
                                    if let Err(error) = settings.validate() {
                                        window.push_notification(
                                            Notification::error(error.message),
                                            cx,
                                        );
                                        return;
                                    }
                                    owner.update(cx, |this, cx| {
                                        if this.is_pending(&[
                                            "application_settings_write",
                                            "system_proxy",
                                        ]) {
                                            return;
                                        }
                                        let Some(latest) = this
                                            .state
                                            .application_settings
                                            .as_ref()
                                            .map(|s| s.settings.clone())
                                        else {
                                            return;
                                        };
                                        if *latest.system_proxy == settings {
                                            window.close_dialog(cx);
                                            return;
                                        }
                                        this.pending_proxy_settings = Some(settings.clone());
                                        this.dispatch(
                                            UiAction::UpdateSystemProxySettings(Box::new(settings)),
                                            cx,
                                        );
                                    });
                                }),
                        ),
                )
        });
        cx.notify();
    }
}
