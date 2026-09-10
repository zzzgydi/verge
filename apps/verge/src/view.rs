mod navigation;
use std::sync::mpsc;

use crate::domain::{
    CommandActor, CommandApproval, CommandContext, CommandRisk, ProfileId, ThemePreference,
};
use crate::ui::{CoreStatus, Page, UiAction, UiRequestEnvelope, UiState};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Root, Sizable as _, StyledExt as _, TitleBar,
    VirtualListScrollHandle,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, InputState, TextareaState},
    scroll::ScrollableElement as _,
    status_bar::StatusBar,
    table::TableState,
    theme::{Theme, ThemeMode},
    v_flex,
};
use gpui_kit::{prelude::FluentBuilder as _, *};

use crate::{
    i18n::{self, Lang, tr},
    pages::{self, connections::ConnectionsDelegate},
};

actions!(
    verge,
    [
        GoToHome,
        GoToProxies,
        GoToProfiles,
        GoToConnections,
        GoToRules,
        GoToLogs,
        GoToSettings,
        RefreshPage
    ]
);

/// Sheet 加载状态：独立于 MainView 的实体。
///
/// GPUI Kit 的 sheet builder 在渲染时执行（`render_sheet_layer` 调用），
/// 而渲染期间 MainView 实体处于 lease 状态——builder 里直接 `view.read()` 读
/// MainView 会 double-lease panic。因此这些状态放进独立实体，builder 只读它。
#[derive(Default)]
pub struct SheetState {
    /// 正在等待 YAML 加载的配置 id。
    pub pending_yaml: Option<ProfileId>,
    /// 正在等待 Merge 配置加载。
    pub pending_merge: bool,
    /// 正在等待合并结果生成的配置 id。
    pub pending_merged: Option<ProfileId>,
}

pub struct MainView {
    pub state: UiState,
    pub telemetry: Entity<pages::home::telemetry::Telemetry>,
    pub requests: mpsc::Sender<UiRequestEnvelope>,
    pub sheet_state: Entity<SheetState>,
    /// 当前设置分类。
    pub settings_category: pages::settings::SettingsCategory,
    pub network_form: pages::settings::network::NetworkForm,
    pub profile_id: Entity<InputState>,
    pub profile_name: Entity<InputState>,
    pub profile_url: Entity<InputState>,
    pub profile_interval: Entity<InputState>,
    /// 订阅请求的自定义 User-Agent（可选）。
    pub profile_user_agent: Entity<InputState>,
    pub backup_passphrase: Entity<InputState>,
    /// 设置页的设置导入路径输入框。
    pub settings_import_path: Entity<InputState>,
    /// 设置页的日志缓冲条数输入框。
    pub log_limit: Entity<InputState>,
    pub pending_proxy_settings: Option<crate::domain::SystemProxySettings>,
    /// 设置页的全局快捷键输入框。
    pub global_hotkey: Entity<InputState>,
    /// 导入对话框的 YAML 表单。
    pub profile_yaml: Entity<TextareaState>,
    /// YAML 抽屉的编辑器，与导入表单分开，互不覆盖草稿。
    pub yaml_editor: Entity<TextareaState>,
    /// Merge 配置抽屉的编辑器。
    pub merge_editor: Entity<TextareaState>,
    /// 合并结果抽屉的只读编辑器。
    pub merged_editor: Entity<TextareaState>,
    pub connections_table: Entity<TableState<ConnectionsDelegate>>,
    /// 日志页虚拟列表滚动位置。
    pub log_scroll: VirtualListScrollHandle,
    pub log_layout: pages::logs::LogLayout,
    pub proxy_page: Entity<pages::proxies::ProxyPage>,
    pub rule_scroll: VirtualListScrollHandle,
    /// 日志级别过滤，None 表示全部。
    pub log_filter: Option<&'static str>,
    /// 点击“预览导入”后等待预览结果再打开确认弹窗的设置文件路径。
    pub pending_settings_import: Option<String>,
    focus_handle: FocusHandle,
    sidebar_collapsed: bool,
    applied_theme: Option<ThemeMode>,
    /// 已同步进日志缓冲输入框的值；外部变更（如恢复备份）与输入框不一致时回填。
    log_limit_applied: Option<u16>,
    /// 已同步进全局快捷键输入框的值，语义同 `log_limit_applied`。
    global_hotkey_applied: Option<Option<String>>,
    /// 输入框占位文案当前使用的语言；语言切换后由 sync_form_inputs 重设。
    placeholders_lang: Option<Lang>,
    _subscriptions: Vec<Subscription>,
    next_request_id: u64,
    next_operation_id: u64,
}

impl MainView {
    /// 当前界面语言：从设置快照取；设置未加载时回退英文（与 domain 默认值一致）。
    pub fn lang(&self) -> Lang {
        Lang::from_code(
            self.state
                .application_settings
                .as_ref()
                .map_or("en", |snapshot| snapshot.settings.language.as_str()),
        )
    }

    pub fn new(
        requests: mpsc::Sender<UiRequestEnvelope>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe_window_appearance(window, |view, window, cx| {
            view.sync_theme(window, cx);
        })
        .detach();
        let actions = cx.entity().downgrade();
        let proxy_page = cx.new(|cx| pages::proxies::ProxyPage::new(actions.clone(), window, cx));
        let connections_table = cx.new(|cx| {
            TableState::new(ConnectionsDelegate::new(actions), window, cx)
                .col_selectable(false)
                .col_movable(false)
        });
        let focus_handle = cx.focus_handle();
        // 窗口级键盘路径（页面切换、刷新）挂在这个焦点上。
        focus_handle.focus(window, cx);
        let log_limit = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("100 – 5000")
                .min(100.)
                .max(5000.)
                .step(50.)
        });
        // 日志缓冲输入合法且与当前设置不同时即保存。
        let log_limit_subscription = cx.subscribe_in(
            &log_limit,
            window,
            |this, state, event: &InputEvent, _, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let Ok(limit) = state.read(cx).value().parse::<u16>() else {
                    return;
                };
                let Some(snapshot) = this.state.application_settings.clone() else {
                    return;
                };
                if snapshot.settings.log_limit != limit {
                    let mut settings = snapshot.settings;
                    settings.log_limit = limit;
                    this.dispatch(UiAction::UpdateSettings(settings), cx);
                }
            },
        );
        // 创建时设置尚未到达，占位文案先用英文；sync_form_inputs 会按实际语言重设。
        let lang = Lang::En;
        Self {
            state: UiState::default(),
            telemetry: cx.new(|_| pages::home::telemetry::Telemetry::new()),
            requests,
            network_form: pages::settings::network::NetworkForm::new(window, cx),
            profile_id: cx.new(|cx| {
                InputState::new(window, cx).placeholder(tr(lang, "placeholder.profile_id"))
            }),
            profile_name: cx.new(|cx| {
                InputState::new(window, cx).placeholder(tr(lang, "placeholder.profile_name"))
            }),
            profile_url: cx.new(|cx| {
                InputState::new(window, cx).placeholder("https://example.com/profile.yaml")
            }),
            profile_interval: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(tr(lang, "placeholder.profile_interval"))
                    .default_value("3600")
            }),
            profile_user_agent: cx.new(|cx| {
                InputState::new(window, cx).placeholder(tr(lang, "placeholder.profile_user_agent"))
            }),
            backup_passphrase: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(tr(lang, "placeholder.backup_passphrase"))
                    .masked(true)
            }),
            settings_import_path: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(tr(lang, "placeholder.settings_import_path"))
            }),
            log_limit,
            pending_proxy_settings: None,
            global_hotkey: cx.new(|cx| {
                InputState::new(window, cx).placeholder(tr(lang, "placeholder.global_hotkey"))
            }),
            profile_yaml: cx.new(|cx| {
                TextareaState::new(window, cx).default_value(
                    "mode: rule\nmixed-port: 7890\nproxies: []\nproxy-groups: []\nrules: []\n",
                )
            }),
            yaml_editor: cx.new(|cx| TextareaState::new(window, cx)),
            merge_editor: cx.new(|cx| TextareaState::new(window, cx)),
            merged_editor: cx.new(|cx| TextareaState::new(window, cx)),
            connections_table,
            log_scroll: VirtualListScrollHandle::new(),
            log_layout: pages::logs::LogLayout::default(),
            proxy_page,
            rule_scroll: VirtualListScrollHandle::new(),
            log_filter: None,
            pending_settings_import: None,
            sheet_state: cx.new(|_| SheetState::default()),
            settings_category: pages::settings::SettingsCategory::General,
            focus_handle,
            sidebar_collapsed: false,
            applied_theme: None,
            log_limit_applied: None,
            global_hotkey_applied: None,
            placeholders_lang: Some(lang),
            _subscriptions: vec![log_limit_subscription],
            next_request_id: 1,
            next_operation_id: 1,
        }
    }

    /// 输入框占位文案随语言切换更新（InputState 只在创建时带占位，语言变了要显式重设）。
    fn sync_placeholders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = self.lang();
        if self.placeholders_lang == Some(lang) {
            return;
        }
        self.placeholders_lang = Some(lang);
        let search = self.proxy_page.read(cx).search.clone();
        search.update(cx, |input, cx| {
            input.set_placeholder(tr(lang, "proxies.search"), window, cx)
        });
        let fields: [(Entity<InputState>, &'static str); 7] = [
            (self.profile_id.clone(), "placeholder.profile_id"),
            (self.profile_name.clone(), "placeholder.profile_name"),
            (
                self.profile_interval.clone(),
                "placeholder.profile_interval",
            ),
            (
                self.profile_user_agent.clone(),
                "placeholder.profile_user_agent",
            ),
            (
                self.backup_passphrase.clone(),
                "placeholder.backup_passphrase",
            ),
            (
                self.settings_import_path.clone(),
                "placeholder.settings_import_path",
            ),
            (self.global_hotkey.clone(), "placeholder.global_hotkey"),
        ];
        for (input, key) in fields {
            input.update(cx, |input, cx| {
                input.set_placeholder(tr(lang, key), window, cx)
            });
        }
    }

    /// 设置变化后把日志缓冲、全局快捷键同步进输入框；输入框聚焦（用户正在编辑）时跳过，失焦后补同步。
    pub fn sync_form_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = self.lang();
        self.telemetry
            .update(cx, |telemetry, cx| telemetry.set_language(lang, cx));
        self.sync_placeholders(window, cx);
        self.network_form.sync(
            self.state.core_network_settings.as_ref(),
            self.state.core_network_revision,
            window,
            cx,
        );
        let Some(snapshot) = self.state.application_settings.clone() else {
            return;
        };
        if self.pending_proxy_settings.as_ref() == Some(snapshot.settings.system_proxy.as_ref()) {
            self.pending_proxy_settings = None;
            gpui_kit::component::WindowExt::close_dialog(window, cx);
        }
        let current = snapshot.settings.log_limit;
        if self.log_limit_applied != Some(current)
            && !self.log_limit.read(cx).focus_handle(cx).is_focused(window)
        {
            self.log_limit.update(cx, |input, cx| {
                input.set_value(current.to_string(), window, cx)
            });
            self.log_limit_applied = Some(current);
        }

        let hotkey = snapshot.settings.global_hotkey.clone();
        if self.global_hotkey_applied != Some(hotkey.clone())
            && !self
                .global_hotkey
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        {
            self.global_hotkey.update(cx, |input, cx| {
                input.set_value(hotkey.clone().unwrap_or_default(), window, cx)
            });
            self.global_hotkey_applied = Some(hotkey);
        }
    }

    pub fn dispatch(&mut self, action: UiAction, cx: &mut Context<Self>) {
        self.dispatch_with_context(
            action,
            CommandContext {
                actor: CommandActor::UserInterface,
                approval: None,
            },
            cx,
        );
    }

    pub fn dispatch_confirmed(&mut self, action: UiAction, cx: &mut Context<Self>) {
        self.dispatch_with_context(
            action,
            CommandContext {
                actor: CommandActor::UserInterface,
                approval: Some(CommandApproval {
                    max_risk: CommandRisk::Destructive,
                }),
            },
            cx,
        );
    }

    fn dispatch_with_context(
        &mut self,
        action: UiAction,
        context: CommandContext,
        cx: &mut Context<Self>,
    ) {
        if let UiAction::Navigate(page) = action {
            self.state.navigate(page);
            cx.notify();
            return;
        }
        if matches!(
            action,
            UiAction::SetSystemProxy { .. } | UiAction::UpdateSystemProxySettings(_)
        ) && !self
            .state
            .daemon_capabilities
            .iter()
            .any(|capability| capability == crate::ipc::protocol::UNIFIED_SYSTEM_PROXY)
        {
            self.pending_proxy_settings = None;
            self.state.last_error = Some(crate::domain::AppError::new(
                crate::domain::ErrorCode::Conflict,
                tr(self.lang(), "proxy.restart_required"),
            ));
            cx.notify();
            return;
        }
        let empty_refresh = self.state.selected_profile.is_none()
            && matches!(
                action,
                UiAction::RefreshHome
                    | UiAction::RefreshSettings
                    | UiAction::UpdateCoreNetworkSettings(_)
                    | UiAction::UpdateSystemProxySettings(_)
            );
        let requests: Vec<_> = action
            .requests()
            .into_iter()
            .filter(|request| {
                // Before a profile is selected, Overview is an onboarding state.
                // Avoid core reads that can only return "no active profile".
                !empty_refresh
                    || !matches!(
                        request,
                        crate::ui::UiRequest::Profile(
                            crate::domain::AppCommand::GetRuntimeSettings
                        ) | crate::ui::UiRequest::Runtime(
                            crate::domain::RuntimeCommand::GetMode
                                | crate::domain::RuntimeCommand::GetNetworkSettings
                                | crate::domain::RuntimeCommand::StartRealtime { .. }
                        )
                    )
            })
            .collect();
        let operation_id = self.next_operation_id;
        self.next_operation_id = self.next_operation_id.wrapping_add(1).max(1);
        let operation_len = requests.len();
        for (operation_index, request) in requests.into_iter().enumerate() {
            let envelope = UiRequestEnvelope {
                request_id: self.next_request_id,
                operation_id,
                operation_index,
                operation_len,
                context,
                request,
            };
            self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
            self.state.begin_envelope(&envelope);
            let _ = self.requests.send(envelope);
        }
        self.sync_proxies(cx);
        cx.notify();
    }

    /// Update retained table state on data arrival, never from render.
    pub fn sync_connections(&self, cx: &mut Context<Self>) {
        let snapshot = self.state.connections.clone();
        let lang = self.lang();
        self.connections_table.update(cx, |table, cx| {
            let same = match (&table.delegate().snapshot, &snapshot) {
                (Some(old), Some(new)) => std::sync::Arc::ptr_eq(old, new),
                (None, None) => true,
                _ => false,
            };
            let language_changed = table.delegate_mut().set_language(lang);
            if !same || language_changed {
                table.delegate_mut().snapshot = snapshot;
                table.refresh(cx);
            }
        });
    }

    pub fn sync_proxies(&self, cx: &mut Context<Self>) {
        self.proxy_page
            .update(cx, |page, cx| page.sync(&self.state, self.lang(), cx));
    }

    /// 任一请求仍在途时返回 true，用于刷新按钮的加载态（防重复提交）。
    pub fn is_pending(&self, keys: &[&'static str]) -> bool {
        keys.iter().any(|key| self.state.pending.contains(key))
    }

    /// 切换页面，并触发该页需要的首批数据加载。
    pub fn navigate(&mut self, page: Page, cx: &mut Context<Self>) {
        self.dispatch(UiAction::Navigate(page), cx);
        match page {
            Page::Profiles => self.dispatch(UiAction::RefreshProfiles, cx),
            Page::Rules => self.dispatch(UiAction::RefreshRules, cx),
            Page::Proxies => self.dispatch(UiAction::RefreshProxies, cx),
            Page::Settings => self.dispatch(UiAction::RefreshSettings, cx),
            _ => {}
        }
    }

    /// 刷新当前页；连接和日志走实时推送，没有对应的刷新请求。
    fn refresh_current_page(&mut self, cx: &mut Context<Self>) {
        match self.state.page {
            Page::Home => self.dispatch(UiAction::RefreshHome, cx),
            Page::Proxies => self.dispatch(UiAction::RefreshProxies, cx),
            Page::Profiles => self.dispatch(UiAction::RefreshProfiles, cx),
            Page::Rules => self.dispatch(UiAction::RefreshRules, cx),
            Page::Settings => self.dispatch(UiAction::RefreshSettings, cx),
            Page::Connections | Page::Logs => {}
        }
    }

    /// 按设置里的主题偏好同步 GPUI Kit 主题；`System` 跟随当前窗口外观。
    pub fn sync_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let preference = self
            .state
            .application_settings
            .as_ref()
            .map_or(ThemePreference::System, |snapshot| snapshot.settings.theme);
        let mode = match preference {
            ThemePreference::Light => ThemeMode::Light,
            ThemePreference::Dark => ThemeMode::Dark,
            ThemePreference::System => ThemeMode::from(window.appearance()),
        };
        if self.applied_theme != Some(mode) {
            self.applied_theme = Some(mode);
            Theme::change(mode, Some(window), cx);
            crate::appearance::apply(mode, cx);
        }
    }

    /// TitleBar 右侧的主题切换按钮：System → Light → Dark 循环。
    fn cycle_theme(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.state.application_settings.clone() else {
            return;
        };
        let mut settings = snapshot.settings;
        settings.theme = match settings.theme {
            ThemePreference::System => ThemePreference::Light,
            ThemePreference::Light => ThemePreference::Dark,
            ThemePreference::Dark => ThemePreference::System,
        };
        self.dispatch(UiAction::UpdateSettings(settings), cx);
    }

    fn title_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang();
        let preference = self
            .state
            .application_settings
            .as_ref()
            .map_or(ThemePreference::System, |snapshot| snapshot.settings.theme);
        let (icon, tooltip) = match preference {
            ThemePreference::System => (IconName::Palette, tr(lang, "titlebar.theme.system")),
            ThemePreference::Light => (IconName::Sun, tr(lang, "titlebar.theme.light")),
            ThemePreference::Dark => (IconName::Moon, tr(lang, "titlebar.theme.dark")),
        };
        TitleBar::new()
            .when(window.is_fullscreen(), |bar| bar.pl_0())
            .child(div().flex_1())
            .child(
                div().pr_3().child(
                    Button::new("theme-toggle")
                        .ghost()
                        .small()
                        .icon(Icon::new(icon).size_4())
                        .tooltip(tooltip)
                        .on_click(cx.listener(|this, _, _, cx| this.cycle_theme(cx))),
                ),
            )
    }

    fn status_bar(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang();
        let core_status = match self.state.core_status {
            CoreStatus::Running => tr(lang, "status.core_running"),
            CoreStatus::Offline => tr(lang, "status.core_offline"),
            CoreStatus::Unknown => tr(lang, "status.core_state_unknown"),
        };
        let mode = self
            .state
            .mode
            .map(|mode| pages::home::mode_label(lang, mode))
            .unwrap_or_else(|| tr(lang, "status.mode_unknown"));
        let connections = self.state.connections.as_ref().map_or_else(
            || tr(lang, "status.connections_waiting").to_owned(),
            |snapshot| i18n::fmt_statusbar_connections(lang, snapshot.connection_count),
        );
        // 流量与内存留给首页统计卡（避免状态栏与首页信息重复）。
        StatusBar::new()
            .left(core_status)
            .left(mode)
            .right(connections)
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // GPUI Kit 的 Root::render 不挂载 overlay 层；dialog / sheet / 通知
        // 必须由应用自己在渲染树里显式挂载，否则窗口状态注册了但屏幕上不显示
        // （表现为"点了没反应"）。
        let sheet_layer = Root::render_sheet_layer(window, cx);
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);

        let page = self.state.page;
        let content = match page {
            Page::Home => pages::home::render(self, cx),
            Page::Proxies => pages::proxies::render(self, cx),
            Page::Profiles => pages::profiles::render(self, cx),
            Page::Connections => pages::connections::render(self, cx),
            Page::Rules => pages::rules::render(self, cx),
            Page::Logs => pages::logs::render(self, window, cx),
            Page::Settings => pages::settings::render(self, cx),
        };

        // 长数据页内部是虚拟化组件，自己滚动（带可见滚动条），不再包滚动容器。
        let content_area: AnyElement = if matches!(
            page,
            Page::Connections | Page::Logs | Page::Proxies | Page::Rules
        ) {
            v_flex()
                .id("page-content")
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .p(px(crate::appearance::metrics::PAGE_INSET))
                .child(content)
                .into_any_element()
        } else {
            v_flex()
                .id("page-content")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p(px(crate::appearance::metrics::PAGE_INSET))
                .child(content)
                .into_any_element()
        };
        let error = self.state.last_error.clone();

        v_flex()
            .id("verge-root")
            .key_context("Verge")
            .track_focus(&self.focus_handle)
            .on_action(|_: &crate::gui::CloseWindow, window, _| window.remove_window())
            .on_action(cx.listener(|this, _: &GoToHome, _, cx| this.navigate(Page::Home, cx)))
            .on_action(cx.listener(|this, _: &GoToProxies, _, cx| this.navigate(Page::Proxies, cx)))
            .on_action(
                cx.listener(|this, _: &GoToProfiles, _, cx| this.navigate(Page::Profiles, cx)),
            )
            .on_action(
                cx.listener(|this, _: &GoToConnections, _, cx| {
                    this.navigate(Page::Connections, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &GoToRules, _, cx| this.navigate(Page::Rules, cx)))
            .on_action(cx.listener(|this, _: &GoToLogs, _, cx| this.navigate(Page::Logs, cx)))
            .on_action(
                cx.listener(|this, _: &GoToSettings, _, cx| this.navigate(Page::Settings, cx)),
            )
            .on_action(cx.listener(|this, _: &RefreshPage, _, cx| this.refresh_current_page(cx)))
            .size_full()
            .bg(cx.theme().background)
            .text_size(px(crate::appearance::metrics::BODY))
            .text_color(cx.theme().foreground)
            .child(self.title_bar(window, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .pb_3()
                    .gap_2()
                    .child(self.sidebar(cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .bg(cx.theme().tiles)
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(px(16.))
                            .overflow_hidden()
                            .when_some(error, |this, error| {
                                this.child(
                                    div().px_4().pt_4().child(
                                        Alert::error(
                                            "global-operation-error",
                                            error.message.clone(),
                                        )
                                        .title(i18n::fmt_alert_title(
                                            self.lang(),
                                            &format!("{:?}", error.code),
                                        ))
                                        .on_close(
                                            cx.listener(|this, _, _, cx| {
                                                this.state.clear_error();
                                                cx.notify();
                                            }),
                                        ),
                                    ),
                                )
                            })
                            .child(content_area),
                    ),
            )
            .child(self.status_bar(cx))
            // overlay 层必须最后挂载（位于内容之上）。
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
    }
}
