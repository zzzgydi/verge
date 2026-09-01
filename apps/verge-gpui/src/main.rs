use std::{path::Path, time::Duration};

use futures::StreamExt;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Root, TitleBar, WindowExt as _,
    input::{Copy, Cut, Paste, Redo, SelectAll, Undo},
    notification::Notification,
};

use gpui_component_assets::Assets;

use i18n::{Lang, tr};
use verge_domain::{
    AppCommand, CommandActor, CommandApproval, CommandContext, CommandRisk, RuntimeCommand,
    SystemProxyCommand,
};
use verge_ipc::{ClientEvent, ConnectError, IpcClient};
use verge_platform::{
    MacNotifier, ProcessRunner, daemon_socket_path, redirect_stderr_to_log, spawn_daemon,
};
use verge_ui::{UiAction, UiResponse};
use view::{
    GoToConnections, GoToHome, GoToLogs, GoToProfiles, GoToProxies, GoToRules, GoToSettings,
    MainView, RefreshPage,
};

mod format;
mod i18n;
mod pages;
mod view;

#[cfg(test)]
mod dialog_mechanism_tests;

actions!(
    app_menu,
    [
        AboutVerge,
        CloseWindow,
        HideVerge,
        HideOtherApps,
        ShowAllApps,
        MinimizeWindow,
        ZoomWindow,
        ToggleFullScreen,
        OpenProjectPage,
        QuitVerge
    ]
);

fn set_app_menus(lang: Lang, cx: &mut App) {
    cx.set_menus([
        Menu::new("Verge").items([
            MenuItem::action(tr(lang, "menu.about"), AboutVerge),
            MenuItem::separator(),
            MenuItem::os_submenu(tr(lang, "menu.services"), SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action(tr(lang, "menu.hide"), HideVerge),
            MenuItem::action(tr(lang, "menu.hide_others"), HideOtherApps),
            MenuItem::action(tr(lang, "menu.show_all"), ShowAllApps),
            MenuItem::separator(),
            MenuItem::action(tr(lang, "menu.quit"), QuitVerge),
        ]),
        Menu::new(tr(lang, "menu.file"))
            .items([MenuItem::action(tr(lang, "menu.close_window"), CloseWindow)]),
        Menu::new(tr(lang, "menu.edit")).items([
            MenuItem::os_action(tr(lang, "menu.undo"), Undo, OsAction::Undo),
            MenuItem::os_action(tr(lang, "menu.redo"), Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action(tr(lang, "menu.cut"), Cut, OsAction::Cut),
            MenuItem::os_action(tr(lang, "menu.copy"), Copy, OsAction::Copy),
            MenuItem::os_action(tr(lang, "menu.paste"), Paste, OsAction::Paste),
            MenuItem::separator(),
            MenuItem::os_action(tr(lang, "menu.select_all"), SelectAll, OsAction::SelectAll),
        ]),
        Menu::new(tr(lang, "menu.window")).items([
            MenuItem::action(tr(lang, "menu.minimize"), MinimizeWindow),
            MenuItem::action(tr(lang, "menu.zoom"), ZoomWindow),
            MenuItem::separator(),
            MenuItem::action(tr(lang, "menu.full_screen"), ToggleFullScreen),
        ]),
        Menu::new(tr(lang, "menu.help")).items([MenuItem::action(
            tr(lang, "menu.project_page"),
            OpenProjectPage,
        )]),
    ]);
}

fn quit_request() -> verge_ui::UiRequestEnvelope {
    verge_ui::UiRequestEnvelope {
        request_id: 0,
        operation_id: 0,
        operation_index: 0,
        operation_len: 1,
        context: CommandContext {
            actor: CommandActor::UserInterface,
            approval: Some(CommandApproval {
                max_risk: CommandRisk::Destructive,
            }),
        },
        request: verge_ui::UiRequest::Profile(AppCommand::QuitApplication),
    }
}

fn main() {
    // 守护进程分叉：同一可执行文件以 --daemon 参数启动，由 GUI 进程拉起或登录项调用。
    if std::env::args().any(|argument| argument == "--daemon") {
        return verge_runtime::run_daemon();
    }
    let data_directory = match verge_runtime::data_directory() {
        Ok(directory) => directory,
        Err(error) => {
            eprintln!("failed to resolve Verge data directory: {}", error.message);
            return;
        }
    };
    // 双击启动没有终端，GUI 进程 stderr 也落盘到数据目录日志。
    if let Err(error) = redirect_stderr_to_log(&data_directory) {
        eprintln!("failed to redirect GUI stderr to log: {error}");
    }
    // 守护优先：先连守护进程，不在则拉起并等待就绪。
    let client = match connect_or_start_daemon(&daemon_socket_path(&data_directory)) {
        Ok(client) => client,
        Err(message) => {
            eprintln!("{message}");
            return;
        }
    };
    let request_tx = client.request_sender();
    let mut events = client.into_events();

    gpui_platform::application()
        // GUI 进程关掉最后一个窗口即退出；守护进程与托盘不受影响。
        .with_quit_mode(QuitMode::LastWindowClosed)
        .with_assets(Assets)
        .run(|cx| {
            gpui_component::init(cx);
            set_app_menus(Lang::En, cx);
            cx.bind_keys([
                KeyBinding::new("cmd-q", QuitVerge, None),
                KeyBinding::new("cmd-w", CloseWindow, None),
                KeyBinding::new("cmd-h", HideVerge, None),
                KeyBinding::new("cmd-alt-h", HideOtherApps, None),
                KeyBinding::new("cmd-m", MinimizeWindow, None),
                KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
            ]);
            cx.on_action(|_: &HideVerge, cx| cx.hide());
            cx.on_action(|_: &HideOtherApps, cx| cx.hide_other_apps());
            cx.on_action(|_: &ShowAllApps, cx| cx.unhide_other_apps());
            cx.on_action(|_: &OpenProjectPage, cx| {
                cx.open_url("https://github.com/zzzgydi/verge")
            });
            cx.on_action(|_: &AboutVerge, cx| {
                if let Some(handle) = cx.active_window() {
                    let _ = handle.update(cx, |_, window, cx| {
                        let answer = window.prompt(
                            PromptLevel::Info,
                            "Verge",
                            Some(concat!("Version ", env!("CARGO_PKG_VERSION"))),
                            &[PromptButton::ok("OK")],
                            cx,
                        );
                        cx.spawn(async move |_| {
                            let _ = answer.await;
                        })
                        .detach();
                    });
                }
            });
            cx.on_action(|_: &CloseWindow, cx| {
                if let Some(handle) = cx.active_window() {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            });
            cx.on_action(|_: &MinimizeWindow, cx| {
                if let Some(handle) = cx.active_window() {
                    let _ = handle.update(cx, |_, window, _| window.minimize_window());
                }
            });
            cx.on_action(|_: &ZoomWindow, cx| {
                if let Some(handle) = cx.active_window() {
                    let _ = handle.update(cx, |_, window, _| window.zoom_window());
                }
            });
            cx.on_action(|_: &ToggleFullScreen, cx| {
                if let Some(handle) = cx.active_window() {
                    let _ = handle.update(cx, |_, window, _| window.toggle_fullscreen());
                }
            });
            let quit_requests = request_tx.clone();
            cx.on_action(move |_: &QuitVerge, cx| {
                if quit_requests.send(quit_request()).is_err() {
                    cx.quit();
                }
            });
            // 窗口级快捷键：cmd+1…7 切页面、cmd+r 刷新当前页（“Verge” key context）。
            let modifier = if cfg!(target_os = "macos") {
                "cmd"
            } else {
                "ctrl"
            };
            cx.bind_keys([
                KeyBinding::new(&format!("{modifier}-1"), GoToHome, Some("Verge")),
                KeyBinding::new(&format!("{modifier}-2"), GoToProxies, Some("Verge")),
                KeyBinding::new(&format!("{modifier}-3"), GoToRules, Some("Verge")),
                KeyBinding::new(&format!("{modifier}-4"), GoToConnections, Some("Verge")),
                KeyBinding::new(&format!("{modifier}-5"), GoToProfiles, Some("Verge")),
                KeyBinding::new(&format!("{modifier}-6"), GoToLogs, Some("Verge")),
                KeyBinding::new(&format!("{modifier}-7"), GoToSettings, Some("Verge")),
                KeyBinding::new(&format!("{modifier}-r"), RefreshPage, Some("Verge")),
            ]);
            let mut notifier = MacNotifier::new(ProcessRunner);
            let window_options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1100.), px(720.)), cx)),
                window_min_size: Some(size(px(960.), px(640.))),
                ..TitleBar::window_options()
            };
            cx.open_window(window_options, |window, cx| {
                    window.set_window_title("Verge");
                    let view = cx.new(|cx| MainView::new(request_tx, window, cx));
                    view.update(cx, |view, cx| {
                        // 启动时先按系统外观设置一次主题。
                        view.sync_theme(window, cx);
                    });
                    let weak_view = view.downgrade();
                    // IPC 事件驱动消费：Welcome / Response / RealtimeBatch / 窗口控制。
                    // 窗口句柄通过 weak_view.update_in 获取，协程不持有 WindowHandle。
                    cx.spawn(async move |cx| {
                        while let Some(event) = events.next().await {
                            match event {
                                ClientEvent::Welcome {
                                    protocol_version,
                                    initial,
                                } => {
                                    let _ = weak_view.update_in(cx, |view, window, cx| {
                                        // 初始快照直接填充领域态，首帧即有内容。
                                        view.state.profiles = initial.profiles;
                                        view.state.selected_profile = initial.selected_profile;
                                        view.state.application_settings =
                                            Some(initial.application_settings);
                                        view.state.runtime_settings = initial.runtime_settings;
                                        set_app_menus(view.lang(), cx);
                                        view.sync_theme(window, cx);
                                        view.sync_form_inputs(window, cx);
                                        // 增量补齐：RefreshHome 同时建立实时订阅，
                                        // 后续变更全部走 Response / RealtimeBatch。
                                        view.dispatch(UiAction::RefreshHome, cx);
                                        view.dispatch(UiAction::RefreshProfiles, cx);
                                        view.dispatch(UiAction::RefreshSettings, cx);
                                    });
                                    let _ = protocol_version;
                                }
                                ClientEvent::Response(envelope) => {
                                    let (yaml_load_error, merge_load_error, merged_load_error) =
                                        match &envelope.response {
                                            UiResponse::Profile {
                                                request: AppCommand::GetProfileYaml { .. },
                                                result: Err(error),
                                            } => (Some(error.clone()), None, None),
                                            UiResponse::Profile {
                                                request: AppCommand::GetMergeConfig,
                                                result: Err(error),
                                            } => (None, Some(error.clone()), None),
                                            UiResponse::Profile {
                                                request: AppCommand::GetMergedProfileYaml { .. },
                                                result: Err(error),
                                            } => (None, None, Some(error.clone())),
                                            _ => (None, None, None),
                                        };
                                    let import_preview_failed = matches!(
                                        &envelope.response,
                                        UiResponse::Profile {
                                            request:
                                                AppCommand::PreviewApplicationSettingsImport {
                                                    ..
                                                },
                                            result: Err(_),
                                        }
                                    );
                                    // toast / OS 通知文案按当前设置语言生成，语言从视图状态取，
                                    // 因此移进 update_in 闭包内计算。
                                    let response = envelope.response.clone();
                                    let mut os_notification = None;
                                    {
                                        let os_notification_slot = &mut os_notification;
                                        let _ = weak_view.update_in(cx, move |view, window, cx| {
                                            let lang = view.lang();
                                            *os_notification_slot = notification_for(lang, &response);
                                            let toast = toast_for(lang, &response);
                                            view.state.apply_response_envelope(envelope);
                                            set_app_menus(view.lang(), cx);
                                            if let Some(error) = &yaml_load_error {
                                                view.fail_yaml_sheet(error, window, cx);
                                            }
                                            if let Some(error) = &merge_load_error {
                                                view.fail_merge_sheet(error, window, cx);
                                            }
                                            if let Some(error) = &merged_load_error {
                                                view.fail_merged_sheet(error, window, cx);
                                            }
                                            if import_preview_failed {
                                                view.fail_import_preview();
                                            }
                                            // 设置响应可能改了主题偏好，顺势同步一次。
                                            view.sync_theme(window, cx);
                                            // 设置首次到达后同步一次表单初值。
                                            view.sync_form_inputs(window, cx);
                                            // “查看 YAML”在加载完成后打开 Sheet。
                                            view.maybe_open_yaml_sheet(window, cx);
                                            // Merge 配置与合并结果 Sheet 同样在加载完成后填充。
                                            view.maybe_open_merge_sheet(window, cx);
                                            view.maybe_open_merged_sheet(window, cx);
                                            // 设置导入预览到达后打开差异确认弹窗。
                                            view.maybe_open_import_preview_dialog(window, cx);
                                            if let Some(toast) = toast {
                                                window.push_notification(toast, cx);
                                            }
                                            cx.notify();
                                        });
                                    }
                                    if let Some((title, body)) = os_notification {
                                        let _ = notifier.notify(title, body);
                                    }
                                }
                                ClientEvent::RealtimeBatch(events) => {
                                    let _ = weak_view.update_in(cx, |view, _, cx| {
                                        for event in events {
                                            view.state.apply_realtime(event);
                                        }
                                        cx.notify();
                                    });
                                }
                                ClientEvent::ActivateWindow => {
                                    let _ = weak_view.update_in(
                                        cx,
                                        |_, window, _| window.activate_window(),
                                    );
                                }
                                ClientEvent::HideWindow => cx.update(|cx| cx.hide()),
                                ClientEvent::Duplicate => {
                                    // 已有主 GUI 实例，本实例退出。
                                    cx.update(|cx| cx.quit());
                                    return;
                                }
                                ClientEvent::Closed { reason } => {
                                    eprintln!("Verge daemon closed the connection: {reason}");
                                    cx.update(|cx| cx.quit());
                                    return;
                                }
                            }
                        }
                        // 事件流结束（守护进程退出）：本实例也退出。
                        cx.update(|cx| cx.quit());
                    })
                    .detach();
                    cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
                })
                .expect("failed to open Verge window");
        });
}

/// 连接守护进程；未运行时拉起并等待就绪。
fn connect_or_start_daemon(socket: &Path) -> Result<IpcClient, String> {
    const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
    // 先试连：守护可能在跑。只有连接失败说明"无守护进程/正在启动"（Io）时才拉起；
    // Duplicate / 版本不匹配 / 被拒都是明确结论，直接返回，绝不能动 socket。
    match IpcClient::connect(socket, APP_VERSION) {
        Ok(client) => return Ok(client),
        Err(ConnectError::Duplicate) => {
            return Err("已有 Verge 主窗口在运行，本实例退出。".into());
        }
        Err(ConnectError::VersionMismatch { server, client }) => {
            return Err(format!(
                "Verge 版本不匹配：守护进程协议 {server}，本构建协议 {client}，请重启应用。"
            ));
        }
        Err(ConnectError::Closed { reason }) => {
            return Err(format!("Verge 守护进程拒绝连接：{reason}"));
        }
        Err(ConnectError::Io(_) | ConnectError::Protocol(_)) => {}
    }
    // 无守护进程：清理可能的 stale socket 后拉起。
    let _ = std::fs::remove_file(socket);
    if let Err(error) = spawn_daemon() {
        return Err(format!("failed to start Verge daemon: {error}"));
    }
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(100));
        match IpcClient::connect(socket, APP_VERSION) {
            Ok(client) => return Ok(client),
            Err(ConnectError::VersionMismatch { server, client }) => {
                return Err(format!(
                    "Verge version mismatch: daemon speaks protocol {server}, this build speaks {client}"
                ));
            }
            Err(ConnectError::Closed { reason }) => {
                return Err(format!("Verge daemon refused the connection: {reason}"));
            }
            Err(ConnectError::Duplicate) => {
                // 已有主 GUI 实例，守护已通知旧实例激活窗口；本实例直接退出。
                return Err("Verge 主窗口已在运行，本实例退出。".into());
            }
            Err(ConnectError::Io(_) | ConnectError::Protocol(_)) => continue,
        }
    }
    Err("Verge daemon did not become ready in time".into())
}

fn notification_for(lang: Lang, response: &UiResponse) -> Option<(&'static str, &'static str)> {
    let UiResponse::Profile {
        request,
        result: Ok(_),
    } = response
    else {
        return None;
    };
    match request {
        AppCommand::ImportProfile { .. }
        | AppCommand::ImportRemoteProfile { .. }
        | AppCommand::SelectProfile { .. }
        | AppCommand::UpdateProfileYaml { .. }
        | AppCommand::UpdateMergeConfig { .. }
        | AppCommand::UpdateRemoteProfile { .. }
        | AppCommand::SetProfileUpdatePolicy { .. }
        | AppCommand::DeleteProfile { .. } => Some(("Verge", tr(lang, "notify.profile_done"))),
        AppCommand::ExportDiagnostics { .. } => {
            Some(("Verge", tr(lang, "notify.diagnostics_exported")))
        }
        _ => None,
    }
}

/// 错误 toast 附带的恢复指引，按错误码给出下一步。
fn recovery_hint(lang: Lang, error: &verge_domain::AppError) -> Option<&'static str> {
    use verge_domain::ErrorCode;
    let key = match error.code {
        ErrorCode::InvalidInput | ErrorCode::ValidationFailed => "hint.invalid_input",
        ErrorCode::NotFound => "hint.not_found",
        ErrorCode::Conflict => "hint.conflict",
        ErrorCode::PermissionDenied => "hint.permission_denied",
        ErrorCode::CoreUnavailable => "hint.core_unavailable",
        ErrorCode::CoreRejectedConfig => "hint.core_rejected",
        ErrorCode::StorageFailed | ErrorCode::PlatformFailed => return None,
    };
    Some(tr(lang, key))
}

/// 应用内 toast：结果不可见的写操作成功给成功提示（自动隐藏），失败给错误提示并附恢复指引（不自动隐藏）。
/// 结果在界面上直接可见的操作（切换模式、切换节点、开关网络设置等）不再弹成功 toast。
fn toast_for(lang: Lang, response: &UiResponse) -> Option<Notification> {
    let ok = |message: &'static str| Notification::success(message);
    let fail = |error: &verge_domain::AppError| {
        let message = i18n::fmt_toast_error(lang, &error.message, recovery_hint(lang, error));
        Notification::error(message).autohide(false)
    };
    match response {
        UiResponse::Profile { request, result } => {
            let success = match request {
                AppCommand::ImportProfile { .. } | AppCommand::ImportRemoteProfile { .. } => {
                    Some(tr(lang, "toast.profile_imported"))
                }
                AppCommand::SelectProfile { .. } => Some(tr(lang, "toast.profile_selected")),
                AppCommand::UpdateProfileYaml { .. } => Some(tr(lang, "toast.yaml_saved")),
                AppCommand::UpdateMergeConfig { .. } => Some(tr(lang, "toast.merge_saved")),
                AppCommand::UpdateRemoteProfile { .. } => Some(tr(lang, "toast.remote_updated")),
                AppCommand::SetProfileUpdatePolicy { .. } => Some(tr(lang, "toast.policy_saved")),
                AppCommand::DeleteProfile { .. } => Some(tr(lang, "toast.profile_deleted")),
                AppCommand::ExportDiagnostics { .. } => {
                    Some(tr(lang, "toast.diagnostics_exported"))
                }
                AppCommand::ExportApplicationSettings { .. } => {
                    Some(tr(lang, "toast.settings_exported"))
                }
                AppCommand::ImportApplicationSettings { .. } => {
                    Some(tr(lang, "toast.settings_imported"))
                }
                AppCommand::ResetApplicationSettingsScope { .. } => {
                    Some(tr(lang, "toast.settings_reset"))
                }
                AppCommand::ExportEncryptedBackup { .. } => Some(tr(lang, "toast.backup_exported")),
                AppCommand::RestoreEncryptedBackup { .. } => {
                    Some(tr(lang, "toast.backup_restored"))
                }
                AppCommand::UpdateMihomo => Some(tr(lang, "toast.mihomo_updated")),
                AppCommand::UpdateApplication => Some(tr(lang, "toast.app_updated")),
                AppCommand::RestartApplication => Some(tr(lang, "toast.app_restarting")),
                // UpdateApplicationSettings / CheckAppUpdate：结果在界面上直接可见，不再弹 toast。
                _ => None,
            };
            match result {
                Ok(_) => success.map(ok),
                // 设置保存/更新检查的成功 toast 免了，但失败必须提示。
                Err(error)
                    if success.is_some()
                        || matches!(
                            request,
                            AppCommand::UpdateApplicationSettings { .. }
                                | AppCommand::PreviewApplicationSettingsImport { .. }
                                | AppCommand::CheckAppUpdate
                        ) =>
                {
                    Some(fail(error))
                }
                Err(_) => None,
            }
        }
        UiResponse::Runtime { request, result } => {
            let success = match request {
                RuntimeCommand::UpdateProvider { .. } => Some(tr(lang, "toast.provider_updated")),
                // SetMode / SelectProxy / SetNetworkSettings / CloseConnection 的结果
                // 在界面上直接可见，不再弹成功 toast。
                _ => None,
            };
            let report_error = success.is_some()
                || matches!(
                    request,
                    RuntimeCommand::SetMode { .. }
                        | RuntimeCommand::SetNetworkSettings { .. }
                        | RuntimeCommand::SelectProxy { .. }
                        | RuntimeCommand::CloseConnection { .. }
                        | RuntimeCommand::TestProxyDelay { .. }
                );
            match result {
                Ok(_) => success.map(ok),
                Err(error) if report_error => Some(fail(error)),
                Err(_) => None,
            }
        }
        UiResponse::SystemProxy { request, result } => match result {
            Ok(_) => None,
            Err(error) if !matches!(request, SystemProxyCommand::GetState) => Some(fail(error)),
            Err(_) => None,
        },
        UiResponse::Realtime(_) => None,
    }
}
