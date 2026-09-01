use gpui::{prelude::FluentBuilder as _, *};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    collapsible::Collapsible,
    form::{field, v_form},
    group_box::GroupBox,
    h_flex,
    input::{Input, NumberInput},
    switch::Switch,
    v_flex,
};
use verge_domain::{HelperStatus, SettingsScope, ThemePreference};
use verge_ui::UiAction;

use crate::{
    i18n::{self, Lang, tr},
    view::MainView,
};

use super::{muted, page_title};

fn helper_label(lang: Lang, view: &MainView) -> String {
    match &view.state.helper_status {
        Some(HelperStatus::Ready { protocol_version }) => {
            i18n::fmt_helper_ready(lang, *protocol_version)
        }
        Some(HelperStatus::NotInstalled) => tr(lang, "settings.helper.not_installed").into(),
        Some(HelperStatus::Incompatible { message }) => {
            i18n::fmt_helper_incompatible(lang, message)
        }
        None => tr(lang, "common.unknown").into(),
    }
}

/// 可折叠设置分组的标题行（箭头 + 组名），点击切换折叠状态。
fn group_trigger<'a>(
    view: &MainView,
    id: &'static str,
    title: &'a str,
    cx: &mut Context<MainView>,
) -> impl IntoElement + 'a {
    let collapsed = view.settings_collapsed.borrow().contains(id);
    let view_entity = cx.entity();
    let title_owned = SharedString::from(title);
    let icon = if collapsed {
        IconName::ChevronRight
    } else {
        IconName::ChevronDown
    };
    h_flex()
        .id(SharedString::from(format!("settings-toggle-{id}")))
        .gap_2()
        .items_center()
        .px_1()
        .py_1()
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            {
                let mut collapsed_set = view_entity.read(cx).settings_collapsed.borrow_mut();
                if collapsed {
                    collapsed_set.remove(id);
                } else {
                    collapsed_set.insert(id);
                }
            }
            cx.notify(view_entity.entity_id());
        })
        .child(
            Icon::new(icon)
                .size_4()
                .text_color(cx.theme().muted_foreground),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title_owned),
        )
}

/// 把分组包成可折叠组件（默认展开）。
fn collapsible_group(
    view: &MainView,
    id: &'static str,
    title: &str,
    group: GroupBox,
    cx: &mut Context<MainView>,
) -> Collapsible {
    let collapsed = view.settings_collapsed.borrow().contains(id);
    Collapsible::new()
        .open(!collapsed)
        .child(group_trigger(view, id, title, cx))
        .content(group)
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let Some(snapshot) = view.state.application_settings.clone() else {
        return v_flex()
            .gap_4()
            .child(page_title(tr(lang, "settings.title")))
            .child(super::skeleton_rows(4))
            .into_any_element();
    };
    let settings = snapshot.settings;
    let data_directory = snapshot.data_directory;
    let app_version = snapshot.app_version;
    let diagnostic_path = format!("{data_directory}/diagnostics.json");
    let settings_export_path = format!("{data_directory}/settings-export.json");
    let helper = helper_label(lang, view);
    // 未安装/不兼容 → 提供“安装/修复”；就绪 → 提供“卸载”（确认弹窗 + 显式授权）。
    let helper_installable = matches!(
        view.state.helper_status,
        Some(HelperStatus::NotInstalled | HelperStatus::Incompatible { .. })
    );
    let helper_ready = matches!(view.state.helper_status, Some(HelperStatus::Ready { .. }));
    let mono = cx.theme().mono_font_family.clone();

    let general_group = GroupBox::new()
        .id("settings-general")
        .title(tr(lang, "settings.group.general"))
        .child(
            v_form()
                .child(
                    field().label(tr(lang, "settings.theme")).child(
                        h_flex().gap_2().children(
                            [
                                (ThemePreference::System, tr(lang, "settings.theme.system")),
                                (ThemePreference::Light, tr(lang, "settings.theme.light")),
                                (ThemePreference::Dark, tr(lang, "settings.theme.dark")),
                            ]
                            .map(|(theme, label)| {
                                let mut updated = settings.clone();
                                updated.theme = theme;
                                Button::new(format!("theme-{theme:?}"))
                                    .label(label)
                                    .small()
                                    .outline()
                                    .selected(settings.theme == theme)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.dispatch(
                                            UiAction::UpdateSettings(updated.clone()),
                                            cx,
                                        );
                                    }))
                            }),
                        ),
                    ),
                )
                .child(
                    field()
                        .label(tr(lang, "settings.language"))
                        .child(h_flex().gap_2().children(
                            [("en", "English"), ("zh-CN", "中文")].map(|(language, label)| {
                                let mut updated = settings.clone();
                                updated.language = language.into();
                                Button::new(format!("language-{language}"))
                                    .label(label)
                                    .small()
                                    .outline()
                                    .selected(settings.language == language)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.dispatch(
                                            UiAction::UpdateSettings(updated.clone()),
                                            cx,
                                        );
                                    }))
                            }),
                        )),
                )
                .child(
                    field()
                        .label(tr(lang, "settings.log_limit"))
                        .description(tr(lang, "settings.log_limit.desc"))
                        .child(NumberInput::new(&view.log_limit)),
                )
                .child(
                    field()
                        .label(tr(lang, "settings.launch_at_login"))
                        .description(tr(lang, "settings.launch_at_login.desc"))
                        .child({
                            let current = settings.clone();
                            h_flex().child(
                                Switch::new("switch-launch-at-login")
                                    .checked(settings.launch_at_login)
                                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                        let mut updated = current.clone();
                                        updated.launch_at_login = *checked;
                                        this.dispatch(UiAction::UpdateSettings(updated), cx);
                                    })),
                            )
                        }),
                )
                .child(
                    field()
                        .label(tr(lang, "settings.global_hotkey"))
                        .description(tr(lang, "settings.global_hotkey.desc"))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(Input::new(&view.global_hotkey))
                                .child(
                                    Button::new("save-global-hotkey")
                                        .label(tr(lang, "common.save"))
                                        .small()
                                        .outline()
                                        .loading(view.is_pending(&["application_settings_write"]))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let value = this
                                                .global_hotkey
                                                .read(cx)
                                                .value()
                                                .trim()
                                                .to_string();
                                            let Some(snapshot) =
                                                this.state.application_settings.clone()
                                            else {
                                                return;
                                            };
                                            let mut updated = snapshot.settings;
                                            updated.global_hotkey =
                                                if value.is_empty() { None } else { Some(value) };
                                            this.dispatch(UiAction::UpdateSettings(updated), cx);
                                        })),
                                ),
                        ),
                )
                .child(
                    field()
                        .label(tr(lang, "settings.reset_default"))
                        .description(tr(lang, "settings.reset_default.desc"))
                        .child(
                            h_flex().gap_2().children(
                                [
                                    (
                                        SettingsScope::Appearance,
                                        tr(lang, "settings.scope.appearance"),
                                    ),
                                    (SettingsScope::Network, tr(lang, "settings.scope.network")),
                                    (SettingsScope::System, tr(lang, "settings.scope.system")),
                                ]
                                .map(|(scope, label)| {
                                    Button::new(format!("reset-scope-{scope:?}"))
                                        .label(label)
                                        .small()
                                        .outline()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.confirm_reset_scope(scope, window, cx);
                                        }))
                                }),
                            ),
                        ),
                ),
        );

    let mut network_group = GroupBox::new()
        .id("settings-network")
        .title(tr(lang, "settings.group.network"));
    if let Some(network) = view.state.network_settings.clone() {
        network_group = network_group.child(
            v_form()
                .child(field().label(tr(lang, "settings.network.tun")).child({
                    let n = network.clone();
                    h_flex().child(
                        Switch::new("switch-tun")
                            .checked(network.tun_enabled)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                let mut settings = n.clone();
                                settings.tun_enabled = *checked;
                                this.dispatch(UiAction::UpdateNetworkSettings(settings), cx);
                            })),
                    )
                }))
                .child(field().label("Mihomo DNS").child({
                    let n = network.clone();
                    h_flex().child(
                        Switch::new("switch-dns")
                            .checked(network.dns_enabled)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                let mut settings = n.clone();
                                settings.dns_enabled = *checked;
                                this.dispatch(UiAction::UpdateNetworkSettings(settings), cx);
                            })),
                    )
                }))
                .child(field().label("IPv6").child({
                    let n = network.clone();
                    h_flex().child(
                        Switch::new("switch-ipv6")
                            .checked(network.ipv6_enabled)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                let mut settings = n.clone();
                                settings.ipv6_enabled = *checked;
                                this.dispatch(UiAction::UpdateNetworkSettings(settings), cx);
                            })),
                    )
                })),
        );
    } else {
        network_group = network_group.child(muted(tr(lang, "settings.network.not_loaded"), cx));
    }

    let proxy_state = view.state.system_proxy.clone();
    let first_service = proxy_state
        .as_ref()
        .and_then(|state| state.services.first().cloned());
    let socks_enabled = proxy_state.as_ref().is_some_and(|state| {
        !state.services.is_empty() && state.services.iter().all(|service| service.socks.enabled)
    });
    let socks_runtime_endpoint = view
        .state
        .runtime_settings
        .as_ref()
        .and_then(|settings| settings.system_proxy_socks_endpoint.clone());
    let socks_current = first_service.as_ref().map(|service| {
        (
            service.socks.enabled,
            format!(
                "{}:{}",
                service.socks.endpoint.host, service.socks.endpoint.port
            ),
        )
    });
    let pac_current = first_service
        .as_ref()
        .map(|service| service.auto_proxy.clone());
    let bypass_current = first_service
        .as_ref()
        .map(|service| service.bypass.clone())
        .unwrap_or_default();

    let proxy_group = GroupBox::new()
        .id("settings-system-proxy")
        .title(tr(lang, "settings.group.proxy"))
        .child(if proxy_state.is_some() {
            v_form()
                .child(
                    field()
                        .label(tr(lang, "settings.proxy.socks"))
                        .description(match &socks_current {
                            Some((true, endpoint)) => i18n::fmt_current(lang, endpoint),
                            _ if socks_runtime_endpoint.is_none() => {
                                tr(lang, "settings.proxy.socks.unavailable").into()
                            }
                            _ => tr(lang, "settings.proxy.socks.available").into(),
                        })
                        .child(h_flex().child({
                            let socks_runtime_endpoint = socks_runtime_endpoint.clone();
                            let current_endpoint = first_service
                                .as_ref()
                                .map(|service| service.socks.endpoint.clone());
                            Switch::new("switch-socks")
                                .checked(socks_enabled)
                                .disabled(!socks_enabled && socks_runtime_endpoint.is_none())
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                    let endpoint = if *checked {
                                        socks_runtime_endpoint.clone()
                                    } else {
                                        current_endpoint
                                            .clone()
                                            .or_else(|| socks_runtime_endpoint.clone())
                                    };
                                    if let Some(endpoint) = endpoint {
                                        this.dispatch(
                                            UiAction::SetSocksProxy {
                                                enabled: *checked,
                                                endpoint,
                                            },
                                            cx,
                                        );
                                    }
                                }))
                        })),
                )
                .child(
                    field()
                        .label(tr(lang, "settings.proxy.pac"))
                        .description(match &pac_current {
                            Some(state) if state.enabled => i18n::fmt_current(
                                lang,
                                state
                                    .url
                                    .as_deref()
                                    .unwrap_or(tr(lang, "settings.proxy.pac.enabled")),
                            ),
                            _ => tr(lang, "settings.proxy.not_set").into(),
                        })
                        .child(
                            h_flex()
                                .gap_2()
                                .child(Input::new(&view.pac_url))
                                .child(
                                    Button::new("apply-pac")
                                        .label(tr(lang, "common.enable"))
                                        .small()
                                        .outline()
                                        .loading(view.is_pending(&["system_proxy"]))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let url =
                                                this.pac_url.read(cx).value().trim().to_string();
                                            if !url.is_empty() {
                                                this.dispatch(
                                                    UiAction::SetAutoProxy { url: Some(url) },
                                                    cx,
                                                );
                                            }
                                        })),
                                )
                                .when(
                                    pac_current.as_ref().is_some_and(|state| state.enabled),
                                    |this| {
                                        this.child(
                                            Button::new("disable-pac")
                                                .label(tr(lang, "common.disable"))
                                                .small()
                                                .outline()
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.dispatch(
                                                        UiAction::SetAutoProxy { url: None },
                                                        cx,
                                                    );
                                                })),
                                        )
                                    },
                                ),
                        ),
                )
                .child(
                    field()
                        .label(tr(lang, "settings.proxy.bypass"))
                        .description(if bypass_current.is_empty() {
                            tr(lang, "settings.proxy.bypass.empty").into()
                        } else {
                            i18n::fmt_current(lang, &bypass_current.join(", "))
                        })
                        .child(
                            h_flex()
                                .gap_2()
                                .child(Input::new(&view.proxy_bypass))
                                .child(
                                    Button::new("save-bypass")
                                        .label(tr(lang, "common.save"))
                                        .small()
                                        .outline()
                                        .loading(view.is_pending(&["system_proxy"]))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let domains = this
                                                .proxy_bypass
                                                .read(cx)
                                                .value()
                                                .split([',', ';', ' ', '\n'])
                                                .map(str::trim)
                                                .filter(|domain| !domain.is_empty())
                                                .map(str::to_owned)
                                                .collect();
                                            this.dispatch(UiAction::SetProxyBypass { domains }, cx);
                                        })),
                                ),
                        ),
                )
                .into_any_element()
        } else {
            muted(tr(lang, "settings.proxy.not_loaded"), cx).into_any_element()
        });

    let core_group = GroupBox::new()
        .id("settings-core")
        .title(tr(lang, "settings.group.core"))
        .child(
            v_form()
                .child(
                    field()
                        .label(tr(lang, "settings.core.update"))
                        .description(tr(lang, "settings.core.update.desc"))
                        .child(
                            h_flex().child(
                                Button::new("update-mihomo")
                                    .label(tr(lang, "common.update_now"))
                                    .small()
                                    .outline()
                                    .loading(view.is_pending(&["application_settings_write"]))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.dispatch(UiAction::UpdateMihomo, cx);
                                    })),
                            ),
                        ),
                )
                .when_some(view.state.mihomo_version.clone(), |form, version| {
                    form.child(
                        field()
                            .label(tr(lang, "settings.core.version"))
                            .child(muted(i18n::fmt_core_installed(lang, &version), cx)),
                    )
                }),
        );

    let app_update_group = GroupBox::new()
        .id("settings-app-update")
        .title(tr(lang, "settings.group.app_update"))
        .child(
            v_form()
                .child(
                    field()
                        .label(tr(lang, "settings.core.version"))
                        .child(muted(
                            app_version.map_or_else(
                                || tr(lang, "settings.app.version_dev").into(),
                                |version| format!("v{version}"),
                            ),
                            cx,
                        )),
                )
                .child(
                    field()
                        .label(tr(lang, "settings.app.check"))
                        .description(tr(lang, "settings.app.check.desc"))
                        .child(
                            h_flex().child(
                                Button::new("check-app-update")
                                    .label(tr(lang, "settings.app.check"))
                                    .small()
                                    .outline()
                                    .loading(view.is_pending(&["app_update"]))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.dispatch(UiAction::CheckAppUpdate, cx);
                                    })),
                            ),
                        ),
                )
                .when_some(view.state.app_update.clone(), |form, status| {
                    form.child(field().label(tr(lang, "settings.app.latest")).child(muted(
                        i18n::fmt_app_latest(lang, &status.latest_version, status.update_available),
                        cx,
                    )))
                })
                .when(
                    view.state
                        .app_update
                        .as_ref()
                        .is_some_and(|status| status.update_available),
                    |form| {
                        form.child(
                            field().label(tr(lang, "settings.app.update")).child(
                                h_flex().child(
                                    Button::new("update-application")
                                        .label(tr(lang, "settings.app.update.download"))
                                        .small()
                                        .outline()
                                        .loading(view.is_pending(&["app_update_write"]))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.confirm_update_application(window, cx);
                                        })),
                                ),
                            ),
                        )
                    },
                )
                .when_some(view.state.app_update_installed.clone(), |form, version| {
                    form.child(
                        field()
                            .label(tr(lang, "settings.app.pending_restart"))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(muted(i18n::fmt_pending_restart(lang, &version), cx))
                                    .child(
                                        Button::new("restart-application")
                                            .label(tr(lang, "settings.app.restart"))
                                            .small()
                                            .primary()
                                            .loading(view.is_pending(&["app_update_write"]))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.confirm_restart_application(window, cx);
                                            })),
                                    ),
                            ),
                    )
                }),
        );

    let system_group = GroupBox::new()
        .id("settings-system")
        .title(tr(lang, "settings.group.system"))
        .child(
            v_form()
                .child(
                    field().label(tr(lang, "settings.system.data_dir")).child(
                        super::selectable_text("settings-data-dir", data_directory)
                            .font_family(mono)
                            .text_sm(),
                    ),
                )
                .child(
                    field().label(tr(lang, "settings.system.helper")).child(
                        h_flex()
                            .gap_2()
                            .child(muted(helper, cx))
                            .when(helper_installable, |row| {
                                row.child(
                                    Button::new("install-helper")
                                        .label(tr(lang, "settings.system.helper.install"))
                                        .small()
                                        .outline()
                                        .loading(view.is_pending(&["helper_write"]))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.dispatch(UiAction::InstallHelper, cx);
                                        })),
                                )
                            })
                            .when(helper_ready, |row| {
                                row.child(
                                    Button::new("uninstall-helper")
                                        .label(tr(lang, "settings.system.helper.uninstall"))
                                        .small()
                                        .outline()
                                        .danger()
                                        .loading(view.is_pending(&["helper_write"]))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.confirm_uninstall_helper(window, cx);
                                        })),
                                )
                            }),
                    ),
                )
                .child(
                    field()
                        .label(tr(lang, "settings.system.diagnostics"))
                        .child(
                            h_flex().child(
                                Button::new("export-diagnostics")
                                    .label(tr(lang, "settings.system.diagnostics.export"))
                                    .small()
                                    .outline()
                                    .loading(view.is_pending(&["application_settings_write"]))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.dispatch(
                                            UiAction::ExportDiagnostics {
                                                destination: diagnostic_path.clone(),
                                            },
                                            cx,
                                        );
                                    })),
                            ),
                        ),
                )
                .child(
                    field()
                        .label(tr(lang, "settings.system.settings_export"))
                        .description(tr(lang, "settings.system.settings_export.desc"))
                        .child(
                            h_flex().child(
                                Button::new("export-application-settings")
                                    .label(tr(lang, "settings.system.settings_export.button"))
                                    .small()
                                    .outline()
                                    .loading(view.is_pending(&["application_settings_write"]))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.dispatch(
                                            UiAction::ExportApplicationSettings {
                                                destination: settings_export_path.clone(),
                                            },
                                            cx,
                                        );
                                    })),
                            ),
                        ),
                )
                .child(
                    field()
                        .label(tr(lang, "settings.system.settings_import"))
                        .description(tr(lang, "settings.system.settings_import.desc"))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(Input::new(&view.settings_import_path))
                                .child(
                                    Button::new("preview-settings-import")
                                        .label(tr(lang, "settings.system.settings_import.button"))
                                        .small()
                                        .outline()
                                        .loading(view.is_pending(&["settings_import_preview"]))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let _ = this.preview_settings_import(cx);
                                        })),
                                ),
                        ),
                ),
        );

    let backup_group = GroupBox::new()
        .id("settings-backup")
        .title(tr(lang, "settings.group.backup"))
        .child(
            v_form()
                .child(
                    field()
                        .label(tr(lang, "settings.backup.passphrase"))
                        .description(tr(lang, "settings.backup.passphrase.desc"))
                        .child(Input::new(&view.backup_passphrase).mask_toggle()),
                )
                .child(
                    field().label(tr(lang, "settings.backup.actions")).child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("export-encrypted-backup")
                                    .label(tr(lang, "settings.backup.export"))
                                    .small()
                                    .outline()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        let passphrase =
                                            this.backup_passphrase.read(cx).value().to_string();
                                        this.dispatch(
                                            UiAction::ExportEncryptedBackup { passphrase },
                                            cx,
                                        );
                                        this.backup_passphrase.update(cx, |input, cx| {
                                            input.set_value("", window, cx)
                                        });
                                    })),
                            )
                            .child(
                                Button::new("restore-encrypted-backup")
                                    .label(tr(lang, "settings.backup.restore"))
                                    .small()
                                    .outline()
                                    .danger()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_restore_backup(window, cx);
                                    })),
                            ),
                    ),
                ),
        )
        .when_some(view.state.last_diagnostic_path.clone(), |group, path| {
            group.child(muted(i18n::fmt_exported(lang, &path), cx))
        });

    v_flex()
        .gap_3()
        .child(page_title(tr(lang, "settings.title")))
        .child(collapsible_group(
            view,
            "general",
            tr(lang, "settings.group.general"),
            general_group,
            cx,
        ))
        .child(collapsible_group(
            view,
            "network",
            tr(lang, "settings.group.network"),
            network_group,
            cx,
        ))
        .child(collapsible_group(
            view,
            "proxy",
            tr(lang, "settings.group.proxy"),
            proxy_group,
            cx,
        ))
        .child(collapsible_group(
            view,
            "core",
            tr(lang, "settings.group.core"),
            core_group,
            cx,
        ))
        .child(collapsible_group(
            view,
            "app-update",
            tr(lang, "settings.group.app_update"),
            app_update_group,
            cx,
        ))
        .child(collapsible_group(
            view,
            "system",
            tr(lang, "settings.group.system"),
            system_group,
            cx,
        ))
        .child(collapsible_group(
            view,
            "backup",
            tr(lang, "settings.group.backup"),
            backup_group,
            cx,
        ))
        .into_any_element()
}
