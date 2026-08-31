use std::rc::Rc;
use std::sync::mpsc;

use gpui::{prelude::FluentBuilder as _, *};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Root, Sizable as _, StyledExt as _,
    TitleBar, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    dialog::DialogButtonProps,
    form::{field, v_form},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    notification::Notification,
    scroll::ScrollableElement as _,
    sidebar::{
        Sidebar, SidebarCollapsible, SidebarGroup, SidebarMenu, SidebarMenuItem,
        SidebarToggleButton,
    },
    status_bar::StatusBar,
    table::TableState,
    theme::{Theme, ThemeMode},
    v_flex,
};
use verge_domain::{
    AppError, CommandActor, CommandApproval, CommandContext, CommandRisk, ErrorCode, ProfileId,
    ProfileSource, SettingsScope, ThemePreference, UpdatePolicy,
};
use verge_ui::{CoreStatus, Page, UiAction, UiRequestEnvelope, UiState};

use crate::{
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
/// gpui-component 的 sheet builder 在渲染时执行（`render_sheet_layer` 调用），
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
    pub requests: mpsc::Sender<UiRequestEnvelope>,
    pub sheet_state: Entity<SheetState>,
    /// 设置页已折叠的分组 id。
    pub settings_collapsed: Rc<std::cell::RefCell<std::collections::HashSet<&'static str>>>,
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
    /// 设置页的自动代理（PAC）地址输入框。
    pub pac_url: Entity<InputState>,
    /// 设置页的代理绕过域名输入框。
    pub proxy_bypass: Entity<InputState>,
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
    /// 日志级别过滤，None 表示全部。
    pub log_filter: Option<&'static str>,
    /// 点击“预览导入”后等待预览结果再打开确认弹窗的设置文件路径。
    pub pending_settings_import: Option<String>,
    /// 组件内回调（表格右键菜单等）回传的待分发动作。
    action_rx: mpsc::Receiver<UiAction>,
    focus_handle: FocusHandle,
    sidebar_collapsed: bool,
    applied_theme: Option<ThemeMode>,
    /// 已同步进日志缓冲输入框的值；外部变更（如恢复备份）与输入框不一致时回填。
    log_limit_applied: Option<u16>,
    /// 已同步进全局快捷键输入框的值，语义同 `log_limit_applied`。
    global_hotkey_applied: Option<Option<String>>,
    _subscriptions: Vec<Subscription>,
    next_request_id: u64,
    next_operation_id: u64,
}

impl MainView {
    pub fn new(
        requests: mpsc::Sender<UiRequestEnvelope>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe_window_appearance(window, |view, window, cx| {
            view.sync_theme(window, cx);
        })
        .detach();
        let (action_tx, action_rx) = mpsc::channel::<UiAction>();
        let connections_table = cx.new(|cx| {
            TableState::new(ConnectionsDelegate::new(action_tx), window, cx)
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
        Self {
            state: UiState::default(),
            requests,
            profile_id: cx.new(|cx| InputState::new(window, cx).placeholder("配置 ID（如 daily）")),
            profile_name: cx.new(|cx| InputState::new(window, cx).placeholder("显示名称")),
            profile_url: cx.new(|cx| {
                InputState::new(window, cx).placeholder("https://example.com/profile.yaml")
            }),
            profile_interval: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("更新间隔（秒）")
                    .default_value("3600")
            }),
            profile_user_agent: cx.new(|cx| {
                InputState::new(window, cx).placeholder("可选，如 ClashX/1.0（留空用默认）")
            }),
            backup_passphrase: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("备份口令（至少 12 个字符）")
                    .masked(true)
            }),
            settings_import_path: cx.new(|cx| {
                InputState::new(window, cx).placeholder("设置导出文件的绝对路径")
            }),
            log_limit,
            pac_url: cx.new(|cx| {
                InputState::new(window, cx).placeholder("http://127.0.0.1:7890/proxy.pac")
            }),
            proxy_bypass: cx.new(|cx| {
                InputState::new(window, cx).placeholder("以逗号分隔，如 *.local, 192.168.0.0/16")
            }),
            global_hotkey: cx.new(|cx| {
                InputState::new(window, cx).placeholder("如 CmdOrCtrl+Shift+V，留空即禁用")
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
            log_filter: None,
            pending_settings_import: None,
            sheet_state: cx.new(|_| SheetState::default()),
            settings_collapsed: Rc::new(std::cell::RefCell::new(std::collections::HashSet::new())),
            action_rx,
            focus_handle,
            sidebar_collapsed: false,
            applied_theme: None,
            log_limit_applied: None,
            global_hotkey_applied: None,
            _subscriptions: vec![log_limit_subscription],
            next_request_id: 1,
            next_operation_id: 1,
        }
    }

    /// 设置变化后把日志缓冲、全局快捷键同步进输入框；输入框聚焦（用户正在编辑）时跳过，失焦后补同步。
    pub fn sync_form_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.state.application_settings.clone() else {
            return;
        };
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
            && !self.global_hotkey.read(cx).focus_handle(cx).is_focused(window)
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
        let requests = action.requests();
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
        cx.notify();
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

    /// 按设置里的主题偏好同步 gpui-component 主题；`System` 跟随当前窗口外观。
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

    pub fn import_profile(&mut self, cx: &mut Context<Self>) -> Result<(), AppError> {
        let id = self.profile_id.read(cx).value().to_string();
        let name = self.profile_name.read(cx).value().to_string();
        let yaml = self.profile_yaml.read(cx).value().to_string();
        match ProfileId::parse(id) {
            Ok(id) => {
                self.dispatch(
                    UiAction::ImportProfile {
                        id,
                        name,
                        yaml,
                        source: ProfileSource::Local,
                        update_policy: UpdatePolicy::Manual,
                    },
                    cx,
                );
                Ok(())
            }
            Err(error) => {
                self.state.set_error(error.clone());
                cx.notify();
                Err(error)
            }
        }
    }

    pub fn import_remote_profile(&mut self, cx: &mut Context<Self>) -> Result<(), AppError> {
        let id = self.profile_id.read(cx).value().to_string();
        let name = self.profile_name.read(cx).value().to_string();
        let url = self.profile_url.read(cx).value().to_string();
        let interval = self.profile_interval.read(cx).value().parse::<u64>();
        let user_agent = self.profile_user_agent.read(cx).value().trim().to_string();
        let user_agent = (!user_agent.is_empty()).then_some(user_agent);
        match (ProfileId::parse(id), interval) {
            (Ok(id), Ok(seconds)) if seconds > 0 => {
                self.dispatch(
                    UiAction::ImportRemoteProfile {
                        id,
                        name,
                        url,
                        update_policy: UpdatePolicy::Interval { seconds },
                        user_agent,
                    },
                    cx,
                );
                Ok(())
            }
            (Err(error), _) => {
                self.state.set_error(error.clone());
                cx.notify();
                Err(error)
            }
            _ => {
                let error = AppError::new(
                    ErrorCode::InvalidInput,
                    "update interval must be a positive integer",
                );
                self.state.set_error(error.clone());
                cx.notify();
                Err(error)
            }
        }
    }

    pub fn set_update_interval(
        &mut self,
        id: ProfileId,
        cx: &mut Context<Self>,
    ) -> Result<(), AppError> {
        match self.profile_interval.read(cx).value().parse::<u64>() {
            Ok(seconds) if seconds > 0 => {
                self.dispatch(
                    UiAction::SetProfileUpdatePolicy {
                        id,
                        update_policy: UpdatePolicy::Interval { seconds },
                    },
                    cx,
                );
                Ok(())
            }
            _ => {
                let error = AppError::new(
                    ErrorCode::InvalidInput,
                    "update interval must be a positive integer",
                );
                self.state.set_error(error.clone());
                cx.notify();
                Err(error)
            }
        }
    }

    /// 导入配置对话框：表单字段持有 MainView 上的输入状态，确认后 dispatch。
    pub fn open_import_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        let id_input = self.profile_id.clone();
        let name_input = self.profile_name.clone();
        let url_input = self.profile_url.clone();
        let interval_input = self.profile_interval.clone();
        let ua_input = self.profile_user_agent.clone();
        let yaml_input = self.profile_yaml.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let view_local = view.clone();
            let view_remote = view.clone();
            dialog
                .title("导入配置")
                .width(rems(35.).to_pixels(window.rem_size()))
                .child(
                    v_form()
                        .child(
                            field()
                                .label("配置 ID")
                                .required(true)
                                .child(Input::new(&id_input)),
                        )
                        .child(
                            field()
                                .label("显示名称")
                                .required(true)
                                .child(Input::new(&name_input)),
                        )
                        .child(
                            field()
                                .label("订阅地址")
                                .description("填写订阅地址后可作为远程配置导入，按间隔自动更新")
                                .child(Input::new(&url_input)),
                        )
                        .child(
                            field()
                                .label("更新间隔（秒）")
                                .child(Input::new(&interval_input)),
                        )
                        .child(
                            field()
                                .label("User-Agent")
                                .description("订阅下载请求的 UA，留空使用默认值")
                                .child(Input::new(&ua_input)),
                        )
                        .child(
                            field()
                                .label("本地 YAML 内容")
                                .child(Textarea::new(&yaml_input).h_32()),
                        ),
                )
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("cancel")
                                .label("取消")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("import-local")
                                .label("导入本地配置")
                                .outline()
                                .on_click(move |_, window, cx| {
                                    match view_local.update(cx, |this, cx| this.import_profile(cx))
                                    {
                                        Ok(()) => window.close_dialog(cx),
                                        Err(error) => window.push_notification(
                                            Notification::error(error.message).autohide(false),
                                            cx,
                                        ),
                                    }
                                }),
                        )
                        .child(
                            Button::new("import-remote")
                                .label("导入远程配置")
                                .primary()
                                .on_click(move |_, window, cx| {
                                    match view_remote
                                        .update(cx, |this, cx| this.import_remote_profile(cx))
                                    {
                                        Ok(()) => window.close_dialog(cx),
                                        Err(error) => window.push_notification(
                                            Notification::error(error.message).autohide(false),
                                            cx,
                                        ),
                                    }
                                }),
                        ),
                )
        });
    }

    /// “设置间隔”对话框：复用更新间隔输入框，保存到指定配置。
    pub fn open_interval_dialog(
        &mut self,
        id: ProfileId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        let interval_input = self.profile_interval.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let view = view.clone();
            let id = id.clone();
            dialog
                .title(format!("更新间隔 · {name}"))
                .width(rems(26.).to_pixels(window.rem_size()))
                .child(
                    v_form().child(
                        field()
                            .label("更新间隔（秒）")
                            .description("仅对远程配置生效")
                            .child(Input::new(&interval_input)),
                    ),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("保存")
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    match view.update(cx, |this, cx| this.set_update_interval(id.clone(), cx)) {
                        Ok(()) => true,
                        Err(error) => {
                            window.push_notification(
                                Notification::error(error.message).autohide(false),
                                cx,
                            );
                            false
                        }
                    }
                })
        });
    }

    /// 删除配置的确认弹窗。
    pub fn confirm_delete_profile(
        &mut self,
        id: ProfileId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            let id = id.clone();
            alert
                .confirm()
                .title(format!("删除“{name}”？"))
                .description("该配置的本地文件将被移除，此操作不可恢复。")
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("删除")
                        .ok_variant(gpui_component::button::ButtonVariant::Danger)
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.dispatch_confirmed(UiAction::DeleteProfile(id.clone()), cx);
                    });
                    true
                })
        });
    }

    /// 恢复加密备份的确认弹窗（覆盖现有配置）。
    pub fn confirm_restore_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .confirm()
                .title("恢复加密备份？")
                .description("现有配置将被备份内容覆盖，此操作不可恢复。")
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("恢复")
                        .ok_variant(gpui_component::button::ButtonVariant::Danger)
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    view.update(cx, |this, cx| {
                        let passphrase = this.backup_passphrase.read(cx).value().to_string();
                        this.dispatch_confirmed(
                            UiAction::RestoreEncryptedBackup { passphrase },
                            cx,
                        );
                        this.backup_passphrase
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    });
                    true
                })
        });
    }

    /// 恢复默认值的确认弹窗：只重置指定作用域，不动配置、备份和其它设置。
    pub fn confirm_reset_scope(
        &mut self,
        scope: SettingsScope,
        label: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .confirm()
                .title(format!("恢复{label}默认值？"))
                .description("只重置该作用域的设置字段，配置、备份和其它设置不受影响。")
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("恢复默认")
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.dispatch(UiAction::ResetSettingsScope(scope), cx);
                    });
                    true
                })
        });
    }

    /// 设置导入第一步：发起差异预览，结果回来后由 maybe_open_import_preview_dialog 打开确认弹窗。
    pub fn preview_settings_import(&mut self, cx: &mut Context<Self>) -> Result<(), AppError> {
        let source = self.settings_import_path.read(cx).value().trim().to_string();
        if source.is_empty() {
            let error = AppError::new(ErrorCode::InvalidInput, "请先填写设置文件的绝对路径");
            self.state.set_error(error.clone());
            cx.notify();
            return Err(error);
        }
        self.state.settings_import_preview = None;
        self.pending_settings_import = Some(source.clone());
        self.dispatch(UiAction::PreviewSettingsImport { source }, cx);
        Ok(())
    }

    /// 预览加载失败时丢弃待处理的确认弹窗（错误由全局提示展示）。
    pub fn fail_import_preview(&mut self) {
        self.pending_settings_import = None;
    }

    /// 预览结果到达后打开确认弹窗：先展示逐字段差异，用户确认后才真正导入。
    pub fn maybe_open_import_preview_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.pending_settings_import.clone() else {
            return;
        };
        let Some(preview) = self.state.settings_import_preview.clone() else {
            return;
        };
        self.pending_settings_import = None;
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, window, _| {
            let view = view.clone();
            let source = source.clone();
            let mut changes = v_flex().gap_1();
            if preview.changes.is_empty() {
                changes = changes.child(
                    div()
                        .text_sm()
                        .child("与当前设置一致，导入后没有字段变化。"),
                );
            } else {
                for change in &preview.changes {
                    changes = changes.child(
                        div()
                            .text_sm()
                            .child(format!("{}：{} → {}", change.field, change.old, change.new)),
                    );
                }
            }
            dialog
                .title("导入设置")
                .width(rems(30.).to_pixels(window.rem_size()))
                .child(v_form().child(field().label("字段差异").child(changes)))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("应用导入")
                        .cancel_text("取消")
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.dispatch(
                            UiAction::ImportApplicationSettings {
                                source: source.clone(),
                            },
                            cx,
                        );
                    });
                    true
                })
        });
    }

    /// 点击后立即打开 YAML Sheet；加载完成后由响应轮询填入编辑器。
    pub fn open_yaml_sheet(&mut self, id: ProfileId, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet_state
            .update(cx, |state, _| state.pending_yaml = Some(id.clone()));
        self.yaml_editor.update(cx, |editor, cx| {
            editor.set_value("正在加载配置 YAML…", window, cx)
        });
        let sheet_state = self.sheet_state.clone();
        let view = cx.entity();
        let editor = self.yaml_editor.clone();
        let mono = cx.theme().mono_font_family.clone();
        let request_id = id.clone();
        window.open_sheet(cx, move |sheet, _, cx| {
            let sheet_state = sheet_state.clone();
            let id = id.clone();
            let editor = editor.clone();
            let mono = mono.clone();
            let loading = sheet_state.read(cx).pending_yaml.as_ref() == Some(&id);
            sheet
                .title(format!("配置 YAML · {}", id.as_str()))
                .size(rems(32.))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .font_family(mono)
                        .text_xs()
                        .child(Textarea::new(&editor).h_full()),
                )
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("copy-yaml")
                                .label("复制")
                                .ghost()
                                .disabled(loading)
                                .on_click({
                                    let editor = editor.clone();
                                    move |_, window, cx| {
                                        let yaml = editor.read(cx).value().to_string();
                                        cx.write_to_clipboard(ClipboardItem::new_string(yaml));
                                        window.push_notification(
                                            Notification::success("已复制到剪贴板"),
                                            cx,
                                        );
                                    }
                                }),
                        )
                        .child(
                            Button::new("save-yaml")
                                .label("保存到该配置")
                                .primary()
                                .disabled(loading)
                                .on_click({
                                    let view = view.clone();
                                    let id = id.clone();
                                    move |_, window, cx| {
                                        let view = view.clone();
                                        let id = id.clone();
                                        window.open_alert_dialog(cx, move |alert, _, _| {
                                            let view = view.clone();
                                            let id = id.clone();
                                            alert
                                                .confirm()
                                                .title(format!("保存到“{}”？", id.as_str()))
                                                .description("编辑器内容将覆盖该配置的现有 YAML。")
                                                .button_props(
                                                    DialogButtonProps::default()
                                                        .ok_text("保存")
                                                        .cancel_text("取消")
                                                        .show_cancel(true),
                                                )
                                                .on_ok(move |_, window, cx| {
                                                    view.update(cx, |this, cx| {
                                                        let yaml = this
                                                            .yaml_editor
                                                            .read(cx)
                                                            .value()
                                                            .to_string();
                                                        this.dispatch(
                                                            UiAction::UpdateProfileYaml {
                                                                id: id.clone(),
                                                                yaml,
                                                            },
                                                            cx,
                                                        );
                                                    });
                                                    window.close_sheet(cx);
                                                    true
                                                })
                                        });
                                    }
                                }),
                        ),
                )
        });
        self.dispatch(UiAction::LoadProfileYaml(request_id), cx);
    }

    /// YAML 加载成功后填入已经打开的 Sheet。
    pub fn maybe_open_yaml_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let want = self.sheet_state.read(cx).pending_yaml.clone();
        let Some(want) = want else {
            return;
        };
        let Some((id, yaml)) = self.state.profile_yaml.clone() else {
            return;
        };
        if id != want {
            return;
        }
        self.sheet_state
            .update(cx, |state, _| state.pending_yaml = None);
        self.yaml_editor
            .update(cx, |editor, cx| editor.set_value(yaml.clone(), window, cx));
    }

    /// YAML 加载失败时保留 Sheet，并在编辑器内直接显示错误。
    pub fn fail_yaml_sheet(
        &mut self,
        error: &AppError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sheet_state
            .update(cx, |state, _| state.pending_yaml = None);
        self.yaml_editor.update(cx, |editor, cx| {
            editor.set_value(
                format!("无法加载配置 YAML：\n{}", error.message),
                window,
                cx,
            )
        });
    }

    /// 点击后立即打开 Merge 配置 Sheet；加载完成后由响应轮询填入编辑器。
    pub fn open_merge_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet_state
            .update(cx, |state, _| state.pending_merge = true);
        self.merge_editor.update(cx, |editor, cx| {
            editor.set_value("正在加载 Merge 配置…", window, cx)
        });
        let view = cx.entity();
        let sheet_state = self.sheet_state.clone();
        let editor = self.merge_editor.clone();
        let mono = cx.theme().mono_font_family.clone();
        window.open_sheet(cx, move |sheet, _, cx| {
            let view = view.clone();
            let sheet_state = sheet_state.clone();
            let editor = editor.clone();
            let mono = mono.clone();
            let loading = sheet_state.read(cx).pending_merge;
            sheet
                .title("全局 Merge 配置")
                .size(rems(32.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("对激活配置的顶层键做受控合并：override / merge / prepend / append / remove。"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .font_family(mono)
                                .text_xs()
                                .child(Textarea::new(&editor).h_full()),
                        ),
                )
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("save-merge")
                                .label("保存 Merge 配置")
                                .primary()
                                .disabled(loading)
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        let view = view.clone();
                                        window.open_alert_dialog(cx, move |alert, _, _| {
                                            let view = view.clone();
                                            alert
                                                .confirm()
                                                .title("保存 Merge 配置？")
                                                .description(
                                                    "将立即应用到当前激活配置，校验或健康检查失败会自动回退。",
                                                )
                                                .button_props(
                                                    DialogButtonProps::default()
                                                        .ok_text("保存")
                                                        .cancel_text("取消")
                                                        .show_cancel(true),
                                                )
                                                .on_ok(move |_, _, cx| {
                                                    view.update(cx, |this, cx| {
                                                        let yaml = this
                                                            .merge_editor
                                                            .read(cx)
                                                            .value()
                                                            .to_string();
                                                        this.dispatch(
                                                            UiAction::SaveMergeConfig { yaml },
                                                            cx,
                                                        );
                                                    });
                                                    true
                                                })
                                        });
                                    }
                                }),
                        ),
                )
        });
        self.dispatch(UiAction::LoadMergeConfig, cx);
    }

    /// Merge 配置加载成功后填入已经打开的 Sheet。
    pub fn maybe_open_merge_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sheet_state.read(cx).pending_merge {
            return;
        }
        let Some(yaml) = self.state.merge_yaml.clone() else {
            return;
        };
        self.sheet_state
            .update(cx, |state, _| state.pending_merge = false);
        self.merge_editor
            .update(cx, |editor, cx| editor.set_value(yaml.clone(), window, cx));
    }

    /// Merge 配置加载失败时保留 Sheet，并在编辑器内直接显示错误。
    pub fn fail_merge_sheet(&mut self, error: &AppError, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet_state
            .update(cx, |state, _| state.pending_merge = false);
        self.merge_editor.update(cx, |editor, cx| {
            editor.set_value(
                format!("无法加载 Merge 配置：\n{}", error.message),
                window,
                cx,
            )
        });
    }

    /// 点击后立即打开合并结果 Sheet（只读）；加载完成后由响应轮询填入内容。
    pub fn open_merged_sheet(&mut self, id: ProfileId, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet_state
            .update(cx, |state, _| state.pending_merged = Some(id.clone()));
        self.merged_editor.update(cx, |editor, cx| {
            editor.set_value("正在生成合并结果…", window, cx)
        });
        let sheet_state = self.sheet_state.clone();
        let editor = self.merged_editor.clone();
        let mono = cx.theme().mono_font_family.clone();
        let request_id = id.clone();
        window.open_sheet(cx, move |sheet, _, cx| {
            let editor = editor.clone();
            let mono = mono.clone();
            let sheet_state = sheet_state.clone();
            let loading = sheet_state.read(cx).pending_merged.is_some();
            sheet
                .title(format!("合并结果 · {}", id.as_str()))
                .size(rems(32.))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .font_family(mono)
                        .text_xs()
                        .child(Textarea::new(&editor).h_full()),
                )
                .footer(
                    h_flex().gap_2().justify_end().child(
                        Button::new("copy-merged")
                            .label("复制")
                            .ghost()
                            .disabled(loading)
                            .on_click({
                                let editor = editor.clone();
                                move |_, window, cx| {
                                    let yaml = editor.read(cx).value().to_string();
                                    cx.write_to_clipboard(ClipboardItem::new_string(yaml));
                                    window.push_notification(
                                        Notification::success("已复制到剪贴板"),
                                        cx,
                                    );
                                }
                            }),
                    ),
                )
        });
        self.dispatch(UiAction::LoadMergedYaml(request_id), cx);
    }

    /// 合并结果加载成功后填入已经打开的 Sheet。
    pub fn maybe_open_merged_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let want = self.sheet_state.read(cx).pending_merged.clone();
        let Some(want) = want else {
            return;
        };
        let Some((id, yaml)) = self.state.merged_yaml.clone() else {
            return;
        };
        if id != want {
            return;
        }
        self.sheet_state
            .update(cx, |state, _| state.pending_merged = None);
        self.merged_editor
            .update(cx, |editor, cx| editor.set_value(yaml.clone(), window, cx));
    }

    /// 合并结果生成失败时保留 Sheet，并在编辑器内直接显示错误。
    pub fn fail_merged_sheet(&mut self, error: &AppError, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet_state
            .update(cx, |state, _| state.pending_merged = None);
        self.merged_editor.update(cx, |editor, cx| {
            editor.set_value(
                format!("无法生成合并结果：\n{}", error.message),
                window,
                cx,
            )
        });
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = self.sidebar_collapsed;
        let item = |page: Page, label: &'static str, icon: IconName, cx: &mut Context<Self>| {
            SidebarMenuItem::new(label)
                .icon(Icon::new(icon).size_4())
                .active(self.state.page == page)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.navigate(page, cx);
                }))
        };
        let proxy_menu = SidebarMenu::new().children([
            item(Page::Home, "概览", IconName::LayoutDashboard, cx),
            item(Page::Proxies, "代理", IconName::Globe, cx),
            item(Page::Rules, "规则", IconName::BookOpen, cx),
            item(Page::Connections, "连接", IconName::Network, cx),
        ]);
        let system_menu = SidebarMenu::new().children([
            item(Page::Profiles, "配置", IconName::File, cx),
            item(Page::Logs, "日志", IconName::SquareTerminal, cx),
            item(Page::Settings, "设置", IconName::Settings, cx),
        ]);

        let header = h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .when(!collapsed, |this| {
                this.child(
                    div()
                        .flex_1()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("导航"),
                )
            })
            .child(
                SidebarToggleButton::new()
                    .collapsed(collapsed)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.sidebar_collapsed = !this.sidebar_collapsed;
                        cx.notify();
                    })),
            );

        let (status_text, status_color) = match self.state.core_status {
            CoreStatus::Running => ("内核运行中", cx.theme().success),
            CoreStatus::Offline => ("内核离线", cx.theme().danger),
            CoreStatus::Unknown => ("状态未知", cx.theme().muted_foreground),
        };
        let dot = div()
            .size_2()
            .flex_shrink_0()
            .rounded(cx.theme().radius_full())
            .bg(status_color);
        let footer = h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .child(dot)
            .when(!collapsed, |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(status_text),
                )
            });

        Sidebar::new("verge-sidebar")
            .w_56()
            .collapsible(SidebarCollapsible::Icon)
            .collapsed(collapsed)
            .header(header)
            .child(SidebarGroup::new("代理").child(proxy_menu))
            .child(SidebarGroup::new("系统").child(system_menu))
            .footer(footer)
    }

    fn title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let preference = self
            .state
            .application_settings
            .as_ref()
            .map_or(ThemePreference::System, |snapshot| snapshot.settings.theme);
        let (icon, tooltip) = match preference {
            ThemePreference::System => (IconName::Palette, "主题：跟随系统（点击切换）"),
            ThemePreference::Light => (IconName::Sun, "主题：浅色（点击切换）"),
            ThemePreference::Dark => (IconName::Moon, "主题：深色（点击切换）"),
        };
        TitleBar::new()
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child("Verge"),
            )
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
        let core_status = match self.state.core_status {
            CoreStatus::Running => "内核运行中",
            CoreStatus::Offline => "内核离线",
            CoreStatus::Unknown => "内核状态未知",
        };
        let mode = self
            .state
            .mode
            .map(pages::home::mode_label)
            .unwrap_or("模式未知");
        let connections = self.state.connections.as_ref().map_or_else(
            || "连接等待中".to_owned(),
            |snapshot| format!("连接 {}", snapshot.connection_count),
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
        // 表格右键菜单等组件回调通过 channel 回到这里统一 dispatch。
        while let Ok(action) = self.action_rx.try_recv() {
            self.dispatch(action, cx);
        }

        // gpui-component 的 Root::render 不挂载 overlay 层；dialog / sheet / 通知
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
            Page::Logs => pages::logs::render(self, cx),
            Page::Settings => pages::settings::render(self, cx),
        };

        // 长数据页内部是虚拟化组件，自己滚动（带可见滚动条），不再包滚动容器。
        let content_area: AnyElement = if matches!(
            page,
            Page::Connections | Page::Logs | Page::Proxies | Page::Rules
        ) {
            div()
                .id("page-content")
                .flex_1()
                .min_h_0()
                .p_4()
                .child(content)
                .into_any_element()
        } else {
            v_flex()
                .id("page-content")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p_6()
                .child(content)
                .into_any_element()
        };
        let error = self.state.last_error.clone();

        v_flex()
            .id("verge-root")
            .key_context("Verge")
            .track_focus(&self.focus_handle)
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
            .text_color(cx.theme().foreground)
            .child(self.title_bar(cx))
            .child(
                h_flex().flex_1().min_h_0().child(self.sidebar(cx)).child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .when_some(error, |this, error| {
                            this.child(
                                div().px_4().pt_4().child(
                                    Alert::error("global-operation-error", error.message.clone())
                                        .title(format!("操作未完成 · {:?}", error.code))
                                        .on_close(cx.listener(|this, _, _, cx| {
                                            this.state.clear_error();
                                            cx.notify();
                                        })),
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
