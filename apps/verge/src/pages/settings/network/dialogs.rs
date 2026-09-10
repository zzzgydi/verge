use super::*;
use gpui_kit::component::input::{Textarea, TextareaState};

struct NetworkDialog {
    settings: CoreNetworkSettings,
    address: Entity<InputState>,
    secret: Entity<InputState>,
    dns: Entity<TextareaState>,
}
impl MainView {
    pub(crate) fn open_core_network_dialog(
        &mut self,
        dns: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(settings) = self.state.core_network_settings.clone() else {
            return;
        };
        self.network_form.pending_dialog = None;
        let lang = self.lang();
        let address = cx.new(|cx| {
            InputState::new(window, cx).default_value(settings.external_controller.address.clone())
        });
        let secret = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.external_controller.secret.clone())
                .masked(true)
        });
        let dns_input =
            cx.new(|cx| TextareaState::new(window, cx).default_value(settings.dns_yaml.clone()));
        let draft = cx.new(|_| NetworkDialog {
            settings,
            address,
            secret,
            dns: dns_input,
        });
        let owner = cx.entity();
        let observation = std::rc::Rc::new(cx.observe(&draft, |_, _, cx| cx.notify()));
        window.open_dialog(cx, move |dialog, _, cx| {
            let _observation = &observation;
            let state = draft.read(cx);
            let toggle = draft.clone();
            let content =
                if dns {
                    v_form()
                        .child(
                            field().label(tr(lang, "network.dns")).child(
                                div()
                                    .id("dns-toggle-wrapper")
                                    .debug_selector(|| "dialog-dns-enabled".into())
                                    .child(
                                        Switch::new("dialog-dns-enabled")
                                            .checked(state.settings.dns_override)
                                            .on_click(move |checked, _, cx| {
                                                toggle.update(cx, |draft, cx| {
                                                    draft.settings.dns_override = *checked;
                                                    cx.notify();
                                                })
                                            }),
                                    ),
                            ),
                        )
                        .child(Textarea::new(&state.dns).h(px(240.)))
                } else {
                    v_form()
                        .child(
                            field().label(tr(lang, "network.controller.enable")).child(
                                Switch::new("dialog-controller-enabled")
                                    .checked(state.settings.external_controller.enabled)
                                    .on_click(move |checked, _, cx| {
                                        toggle.update(cx, |draft, cx| {
                                            draft.settings.external_controller.enabled = *checked;
                                            cx.notify();
                                        })
                                    }),
                            ),
                        )
                        .child(field().label(tr(lang, "network.controller.address")).child(
                            copy_input(
                                "copy-controller-address",
                                &state.address,
                                !state.settings.external_controller.enabled,
                            ),
                        ))
                        .child(field().label(tr(lang, "network.controller.secret")).child(
                            copy_input(
                                "copy-controller-secret",
                                &state.secret,
                                !state.settings.external_controller.enabled,
                            ),
                        ))
                };
            let save = draft.clone();
            let owner = owner.clone();
            dialog
                .title(tr(
                    lang,
                    if dns {
                        "network.dns.title"
                    } else {
                        "network.controller"
                    },
                ))
                .width(px(640.))
                .child(content)
                .footer(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("network-dialog-cancel")
                                .label(tr(lang, "common.cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("network-dialog-save")
                                .debug_selector(|| "network-dialog-save".into())
                                .label(tr(lang, "common.save"))
                                .primary()
                                .on_click(move |_, window, cx| {
                                    let draft = save.read(cx);
                                    let mut settings = draft.settings.clone();
                                    if dns {
                                        settings.dns_yaml = draft.dns.read(cx).value().to_string();
                                    } else {
                                        settings.external_controller.address =
                                            draft.address.read(cx).value().trim().to_owned();
                                        settings.external_controller.secret =
                                            draft.secret.read(cx).value().to_string();
                                    }
                                    if let Err(error) = settings.validate() {
                                        window.push_notification(
                                            Notification::error(error.message),
                                            cx,
                                        );
                                        return;
                                    }
                                    owner.update(cx, |this, cx| {
                                        if this.is_pending(&["core_network_write"]) {
                                            return;
                                        }
                                        // Merge only the edited section into the latest committed snapshot.
                                        let Some(mut latest) =
                                            this.state.core_network_settings.clone()
                                        else {
                                            return;
                                        };
                                        if dns {
                                            latest.dns_yaml = settings.dns_yaml;
                                            latest.dns_override = settings.dns_override;
                                        } else {
                                            latest.external_controller =
                                                settings.external_controller;
                                        }
                                        if let Err(error) = latest.validate() {
                                            window.push_notification(
                                                Notification::error(error.message),
                                                cx,
                                            );
                                            return;
                                        }
                                        this.network_form.pending_dialog =
                                            Some(this.state.core_network_revision);
                                        this.dispatch(
                                            UiAction::UpdateCoreNetworkSettings(latest),
                                            cx,
                                        );
                                    });
                                }),
                        ),
                )
        });
    }
}
fn copy_input(id: &'static str, input: &Entity<InputState>, disabled: bool) -> impl IntoElement {
    let copy = input.clone();
    h_flex()
        .gap_2()
        .child(Input::new(input).disabled(disabled))
        .child(
            Button::new(id)
                .icon(IconName::Copy)
                .ghost()
                .disabled(disabled)
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        copy.read(cx).value().to_string(),
                    ))
                }),
        )
}
