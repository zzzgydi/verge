mod dialogs;
use super::components::{SettingsSection, field, v_form};
use crate::{domain::CoreNetworkSettings, i18n::tr, ui::UiAction, view::MainView};
use gpui::*;
use gpui_component::{
    Disableable as _, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    switch::Switch,
};

pub(crate) struct NetworkForm {
    port: Entity<InputState>,
    applied_port: Option<u16>,
    pub(super) pending_dialog: Option<u64>,
}
impl NetworkForm {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            port: cx.new(|cx| InputState::new(window, cx).placeholder("7897")),
            applied_port: None,
            pending_dialog: None,
        }
    }
    pub fn sync(
        &mut self,
        settings: Option<&CoreNetworkSettings>,
        revision: u64,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(settings) = settings {
            if self.applied_port != Some(settings.mixed_port) {
                self.port.update(cx, |input, cx| {
                    input.set_value(settings.mixed_port.to_string(), window, cx)
                });
                self.applied_port = Some(settings.mixed_port);
            }
            if self
                .pending_dialog
                .is_some_and(|previous| revision != previous)
            {
                self.pending_dialog = None;
                window.close_dialog(cx);
            }
        }
    }
}

pub(super) fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let mut section = SettingsSection::new()
        .id("settings-network")
        .title(tr(lang, "settings.group.network"));
    let Some(settings) = view.state.core_network_settings.clone() else {
        return section
            .child(super::super::skeleton_rows(4))
            .into_any_element();
    };
    let busy = view.is_pending(&["core_network_write"]);
    let mut rows = v_form();
    for (id, title, description, value) in [
        (
            "allow-lan",
            "network.lan",
            "network.lan.desc",
            settings.allow_lan,
        ),
        ("ipv6", "IPv6", "network.ipv6.desc", settings.ipv6),
        (
            "unified-delay",
            "network.delay",
            "network.delay.desc",
            settings.unified_delay,
        ),
    ] {
        rows = rows.child(
            field()
                .label(if title == "IPv6" {
                    title
                } else {
                    tr(lang, title)
                })
                .description(tr(lang, description))
                .child(
                    h_flex().justify_end().child(
                        Switch::new(id)
                            .checked(value)
                            .disabled(busy)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                if let Some(mut settings) = this.state.core_network_settings.clone()
                                {
                                    match id {
                                        "allow-lan" => settings.allow_lan = *checked,
                                        "ipv6" => settings.ipv6 = *checked,
                                        _ => settings.unified_delay = *checked,
                                    }
                                    this.dispatch(
                                        UiAction::UpdateCoreNetworkSettings(settings),
                                        cx,
                                    );
                                }
                            })),
                    ),
                ),
        );
    }
    rows = rows.child(
        field()
            .label(tr(lang, "network.dns"))
            .description(tr(lang, "network.dns.desc"))
            .child(
                h_flex()
                    .justify_end()
                    .gap_3()
                    .child(
                        Button::new("dns-options")
                            .icon(IconName::Settings)
                            .ghost()
                            .small()
                            .h(px(crate::appearance::metrics::COMPACT_CONTROL))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_core_network_dialog(true, window, cx)
                            })),
                    )
                    .child(
                        Switch::new("dns-override")
                            .checked(settings.dns_override)
                            .disabled(busy)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                if let Some(mut settings) = this.state.core_network_settings.clone()
                                {
                                    settings.dns_override = *checked;
                                    this.dispatch(
                                        UiAction::UpdateCoreNetworkSettings(settings),
                                        cx,
                                    );
                                }
                            })),
                    ),
            ),
    );
    let owner = cx.entity();
    rows = rows.child(
        field().label(tr(lang, "network.log")).child(
            h_flex().justify_end().child(
                Button::new("core-log-level")
                    .small()
                    .h(px(crate::appearance::metrics::CONTROL))
                    .label(settings.log_level)
                    .icon(IconName::ChevronDown)
                    .outline()
                    .disabled(busy)
                    .dropdown_menu(move |menu, _, _| {
                        let mut menu = menu;
                        for level in ["silent", "error", "warning", "info", "debug"] {
                            let owner = owner.clone();
                            menu =
                                menu.item(PopupMenuItem::new(level).on_click(move |_, _, cx| {
                                    owner.update(cx, |this, cx| {
                                        if let Some(mut settings) =
                                            this.state.core_network_settings.clone()
                                        {
                                            settings.log_level = level.into();
                                            this.dispatch(
                                                UiAction::UpdateCoreNetworkSettings(settings),
                                                cx,
                                            );
                                        }
                                    });
                                }));
                        }
                        menu
                    }),
            ),
        ),
    );
    rows = rows.child(
        field()
            .label(tr(lang, "network.port"))
            .description(tr(lang, "network.port.desc"))
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&view.network_form.port).disabled(busy))
                    .child(
                        Button::new("save-core-port")
                            .small()
                            .h(px(crate::appearance::metrics::CONTROL))
                            .label(tr(lang, "common.save"))
                            .outline()
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let value = this.network_form.port.read(cx).value();
                                match value.trim().parse::<u16>().ok().filter(|p| *p > 0) {
                                    Some(port) => {
                                        if let Some(mut settings) =
                                            this.state.core_network_settings.clone()
                                        {
                                            settings.mixed_port = port;
                                            match settings.validate() {
                                                Ok(()) => this.dispatch(
                                                    UiAction::UpdateCoreNetworkSettings(settings),
                                                    cx,
                                                ),
                                                Err(error) => window.push_notification(
                                                    Notification::error(error.message),
                                                    cx,
                                                ),
                                            }
                                        }
                                    }
                                    None => window.push_notification(
                                        Notification::error(tr(
                                            this.lang(),
                                            "network.port.invalid",
                                        )),
                                        cx,
                                    ),
                                }
                            })),
                    ),
            ),
    );
    let controller = settings.external_controller;
    rows = rows.child(
        field()
            .label(tr(lang, "network.controller"))
            .description(tr(lang, "network.controller.desc"))
            .child(
                h_flex().justify_end().child(
                    Button::new("external-controller-options")
                        .small()
                        .h(px(crate::appearance::metrics::CONTROL))
                        .label(if controller.enabled {
                            controller.address
                        } else {
                            tr(lang, "network.disabled").into()
                        })
                        .icon(IconName::ChevronRight)
                        .ghost()
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_core_network_dialog(false, window, cx)
                        })),
                ),
            ),
    );
    if let Some(network) = &view.state.network_settings {
        rows = rows.child(
            field().label(tr(lang, "settings.network.tun")).child(
                h_flex().justify_end().child(
                    Switch::new("switch-tun")
                        .checked(network.tun_enabled)
                        .disabled(busy || view.is_pending(&["network_settings_write"]))
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            if let Some(mut settings) = this.state.network_settings.clone() {
                                settings.tun_enabled = *checked;
                                this.dispatch(UiAction::UpdateNetworkSettings(settings), cx);
                            }
                        })),
                ),
            ),
        );
    }
    section = section.child(rows);
    section.into_any_element()
}
