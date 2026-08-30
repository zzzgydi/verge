use gpui::{prelude::FluentBuilder as _, *};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    form::{field, v_form},
    group_box::GroupBox,
    h_flex,
    input::{Input, NumberInput},
    switch::Switch,
    v_flex,
};
use verge_domain::{HelperStatus, SettingsScope, ThemePreference};
use verge_ui::UiAction;

use crate::view::MainView;

use super::{muted, page_title};

fn helper_label(view: &MainView) -> String {
    match &view.state.helper_status {
        Some(HelperStatus::Ready { protocol_version }) => {
            format!("就绪（协议版本 {protocol_version}）")
        }
        Some(HelperStatus::NotInstalled) => "未安装".into(),
        Some(HelperStatus::Incompatible { message }) => format!("需要修复：{message}"),
        None => "未知".into(),
    }
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let Some(snapshot) = view.state.application_settings.clone() else {
        return v_flex()
            .gap_4()
            .child(page_title("设置"))
            .child(super::skeleton_rows(4))
            .into_any_element();
    };
    let settings = snapshot.settings;
    let data_directory = snapshot.data_directory;
    let diagnostic_path = format!("{data_directory}/diagnostics.json");
    let settings_export_path = format!("{data_directory}/settings-export.json");
    let helper = helper_label(view);
    let mono = cx.theme().mono_font_family.clone();

    let general_group = GroupBox::new().id("settings-general").title("通用").child(
        v_form()
            .child(
                field().label("主题").child(
                    h_flex().gap_2().children(
                        [
                            (ThemePreference::System, "跟随系统"),
                            (ThemePreference::Light, "浅色"),
                            (ThemePreference::Dark, "深色"),
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
                                    this.dispatch(UiAction::UpdateSettings(updated.clone()), cx);
                                }))
                        }),
                    ),
                ),
            )
            .child(
                field().label("语言").child(h_flex().gap_2().children(
                    [("en", "English"), ("zh-CN", "中文")].map(|(language, label)| {
                        let mut updated = settings.clone();
                        updated.language = language.into();
                        Button::new(format!("language-{language}"))
                            .label(label)
                            .small()
                            .outline()
                            .selected(settings.language == language)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.dispatch(UiAction::UpdateSettings(updated.clone()), cx);
                            }))
                    }),
                )),
            )
            .child(
                field()
                    .label("日志缓冲")
                    .description("100 – 5000 条，超出后丢弃最旧的日志")
                    .child(NumberInput::new(&view.log_limit)),
            )
            .child(field().label("开机启动").description("登录 macOS 后自动启动 Verge，需打包为 .app 才能生效").child({
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
            }))
            .child(
                field()
                    .label("全局快捷键")
                    .description("显示/隐藏主窗口；留空保存即禁用，组合键被占用时会回滚到旧快捷键")
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Input::new(&view.global_hotkey))
                            .child(
                                Button::new("save-global-hotkey")
                                    .label("保存")
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
                    .label("恢复默认")
                    .description("按作用域恢复默认值，不影响配置和备份")
                    .child(h_flex().gap_2().children(
                        [
                            (SettingsScope::Appearance, "外观"),
                            (SettingsScope::Network, "网络"),
                            (SettingsScope::System, "系统"),
                        ]
                        .map(|(scope, label)| {
                            Button::new(format!("reset-scope-{scope:?}"))
                                .label(label)
                                .small()
                                .outline()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.confirm_reset_scope(scope, label, window, cx);
                                }))
                        }),
                    )),
            ),
    );

    let mut network_group = GroupBox::new().id("settings-network").title("网络");
    if let Some(network) = view.state.network_settings.clone() {
        network_group = network_group.child(
            v_form()
                .child(field().label("TUN 模式").child({
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
        network_group = network_group.child(muted("网络设置尚未加载。", cx));
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
        .title("系统代理")
        .child(if proxy_state.is_some() {
            v_form()
                .child(
                    field()
                        .label("SOCKS 代理")
                        .description(match &socks_current {
                            Some((true, endpoint)) => format!("当前：{endpoint}"),
                            _ if socks_runtime_endpoint.is_none() => {
                                "配置未声明 mixed-port 或 socks-port，无法启用".into()
                            }
                            _ => "使用配置的 mixed-port 或 socks-port".into(),
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
                        .label("自动代理（PAC）")
                        .description(match &pac_current {
                            Some(state) if state.enabled => {
                                format!("当前：{}", state.url.as_deref().unwrap_or("已启用"))
                            }
                            _ => "未设置".into(),
                        })
                        .child(
                            h_flex()
                                .gap_2()
                                .child(Input::new(&view.pac_url))
                                .child(
                                    Button::new("apply-pac")
                                        .label("启用")
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
                                                .label("关闭")
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
                        .label("代理绕过列表")
                        .description(if bypass_current.is_empty() {
                            "未设置；输入框留空保存即清空".into()
                        } else {
                            format!("当前：{}", bypass_current.join(", "))
                        })
                        .child(
                            h_flex()
                                .gap_2()
                                .child(Input::new(&view.proxy_bypass))
                                .child(
                                    Button::new("save-bypass")
                                        .label("保存")
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
            muted("系统代理状态尚未加载。", cx).into_any_element()
        });

    let core_group = GroupBox::new()
        .id("settings-core")
        .title("Mihomo 内核")
        .child(
            v_form()
                .child(
                    field()
                        .label("内核更新")
                        .description("下载、校验并更新 Mihomo 内核")
                        .child(
                            h_flex().child(
                                Button::new("update-mihomo")
                                    .label("立即更新")
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
                            .label("当前版本")
                            .child(muted(format!("已安装并校验：{version}"), cx)),
                    )
                }),
        );

    let system_group = GroupBox::new().id("settings-system").title("系统").child(
        v_form()
            .child(
                field().label("数据目录").child(
                    super::selectable_text("settings-data-dir", data_directory)
                        .font_family(mono)
                        .text_sm(),
                ),
            )
            .child(field().label("特权 Helper").child(muted(helper, cx)))
            .child(
                field().label("诊断导出").child(
                    h_flex().child(
                        Button::new("export-diagnostics")
                            .label("导出脱敏诊断")
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
                    .label("设置导出")
                    .description("明文设置文件，不含密钥和订阅凭据，可跨机器迁移")
                    .child(h_flex().child(
                        Button::new("export-application-settings")
                            .label("导出设置")
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
                    )),
            )
            .child(
                field()
                    .label("设置导入")
                    .description("先预览字段差异，确认后才应用")
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Input::new(&view.settings_import_path))
                            .child(
                                Button::new("preview-settings-import")
                                    .label("预览差异并导入")
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
        .title("加密备份")
        .child(
            v_form()
                .child(
                    field()
                        .label("备份口令")
                        .description("导出和恢复使用同一个口令，至少 12 个字符")
                        .child(Input::new(&view.backup_passphrase).mask_toggle()),
                )
                .child(
                    field().label("备份操作").child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("export-encrypted-backup")
                                    .label("导出加密备份")
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
                                    .label("恢复备份")
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
            group.child(muted(format!("已导出：{path}"), cx))
        });

    v_flex()
        .gap_4()
        .child(page_title("设置"))
        .child(general_group)
        .child(network_group)
        .child(proxy_group)
        .child(core_group)
        .child(system_group)
        .child(backup_group)
        .into_any_element()
}
