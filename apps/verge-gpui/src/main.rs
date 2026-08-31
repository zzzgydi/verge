use std::{
    path::Path,
    sync::mpsc,
    time::Duration,
};

use futures::StreamExt;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Root, TitleBar, WindowExt as _, notification::Notification,
};
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use verge_domain::{AppCommand, RuntimeCommand, SystemProxyCommand};
use verge_ipc::{ClientEvent, ConnectError, IpcClient};
use verge_platform::{
    MacNotifier, ProcessRunner, daemon_socket_path, spawn_daemon,
};
use verge_ui::{UiAction, UiResponse};
use view::{
    GoToConnections, GoToHome, GoToLogs, GoToProfiles, GoToProxies, GoToRules, GoToSettings,
    MainView, RefreshPage,
};

mod format;
mod hotkey;
mod pages;
mod view;

#[cfg(test)]
mod dialog_mechanism_tests;

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
        .run(|cx| {
            gpui_component::init(cx);
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
            // 全局快捷键：macOS 要求 GlobalHotKeyManager 在主线程创建和调用；
            // 事件经 futures channel 由 GPUI 协程事件驱动消费（无轮询）。
            let (hotkey_event_tx, mut hotkey_event_rx) =
                futures::channel::mpsc::unbounded::<()>();
            let mut hotkeys = match hotkey::GlobalHotKeyBackend::new() {
                Ok(backend) => {
                    std::thread::spawn(move || {
                        let receiver = GlobalHotKeyEvent::receiver();
                        while let Ok(event) = receiver.recv() {
                            if event.state == HotKeyState::Pressed
                                && hotkey_event_tx.unbounded_send(()).is_err()
                            {
                                return;
                            }
                        }
                    });
                    Some(hotkey::HotkeyRegistration::new(backend))
                }
                Err(error) => {
                    eprintln!("global hotkey is unavailable: {}", error.message);
                    None
                }
            };
            let mut notifier = MacNotifier::new(ProcessRunner);
            let (view_tx, view_rx) = mpsc::channel();
            let window_options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1100.), px(720.)), cx)),
                window_min_size: Some(size(px(960.), px(640.))),
                ..TitleBar::window_options()
            };
            let window = cx
                .open_window(window_options, |window, cx| {
                    window.set_window_title("Verge");
                    let view = cx.new(|cx| MainView::new(request_tx, window, cx));
                    view.update(cx, |view, cx| {
                        // 启动时先按系统外观设置一次主题。
                        view.sync_theme(window, cx);
                    });
                    let weak_view = view.downgrade();
                    let _ = view_tx.send(weak_view.clone());
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
                                    if let Some((title, body)) =
                                        notification_for(&envelope.response)
                                    {
                                        let _ = notifier.notify(title, body);
                                    }
                                    let toast = toast_for(&envelope.response);
                                    let _ = weak_view.update_in(cx, |view, window, cx| {
                                        view.state.apply_response_envelope(envelope);
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
                                    // 设置快照（首次到达或变更）后同步全局快捷键注册。
                                    if let Some(service) = &mut hotkeys {
                                        let _ = weak_view.update_in(cx, |view, window, cx| {
                                            let desired = view
                                                .state
                                                .application_settings
                                                .as_ref()
                                                .and_then(|snapshot| {
                                                    snapshot.settings.global_hotkey.clone()
                                                });
                                            if let Err(error) = service.sync(desired.as_deref()) {
                                                window.push_notification(
                                                    Notification::error(error.message)
                                                        .autohide(false),
                                                    cx,
                                                );
                                            }
                                        });
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
            let weak_view = view_rx
                .recv()
                .expect("main view should be created with the window");
            // 全局快捷键“显示/隐藏主窗口”：事件驱动消费（open_window 之后才有 WindowHandle）。
            cx.spawn(async move |cx| {
                while hotkey_event_rx.next().await.is_some() {
                    cx.update(|cx| {
                        let active = window
                            .update(cx, |_, window, _| window.is_window_active())
                            .unwrap_or(false);
                        if active {
                            cx.hide();
                        } else {
                            show_main_window(&window, cx);
                        }
                    });
                }
            })
            .detach();
            let _ = weak_view;
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

/// 托盘“显示 Verge”与全局快捷键共用的窗口激活逻辑。
fn show_main_window(window: &WindowHandle<Root>, cx: &mut App) {
    cx.activate(true);
    let _ = window.update(cx, |_, window, _| window.activate_window());
}

fn notification_for(response: &UiResponse) -> Option<(&'static str, &'static str)> {
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
        | AppCommand::DeleteProfile { .. } => Some(("Verge", "配置操作已完成")),
        AppCommand::ExportDiagnostics { .. } => Some(("Verge", "脱敏诊断已导出")),
        _ => None,
    }
}

/// 错误 toast 附带的恢复指引，按错误码给出下一步。
fn recovery_hint(error: &verge_domain::AppError) -> Option<&'static str> {
    use verge_domain::ErrorCode;
    match error.code {
        ErrorCode::InvalidInput | ErrorCode::ValidationFailed => Some("请检查输入后重试"),
        ErrorCode::NotFound => Some("目标可能已被移除，请刷新后重试"),
        ErrorCode::Conflict => Some("请刷新确认当前状态后重试"),
        ErrorCode::PermissionDenied => Some("该操作需要明确确认后才能执行"),
        ErrorCode::CoreUnavailable => Some("请先在“设置”页安装或更新 Mihomo 内核"),
        ErrorCode::CoreRejectedConfig => Some("请检查配置 YAML 后重试"),
        ErrorCode::StorageFailed | ErrorCode::PlatformFailed => None,
    }
}

/// 应用内 toast：结果不可见的写操作成功给成功提示（自动隐藏），失败给错误提示并附恢复指引（不自动隐藏）。
/// 结果在界面上直接可见的操作（切换模式、切换节点、开关网络设置等）不再弹成功 toast。
fn toast_for(response: &UiResponse) -> Option<Notification> {
    let ok = |message: &'static str| Notification::success(message);
    let fail = |error: &verge_domain::AppError| {
        let message = match recovery_hint(error) {
            Some(hint) => format!("{}。{hint}。", error.message),
            None => error.message.clone(),
        };
        Notification::error(message).autohide(false)
    };
    match response {
        UiResponse::Profile { request, result } => {
            let success = match request {
                AppCommand::ImportProfile { .. } | AppCommand::ImportRemoteProfile { .. } => {
                    Some("配置已导入")
                }
                AppCommand::SelectProfile { .. } => Some("已启用配置"),
                AppCommand::UpdateProfileYaml { .. } => Some("配置 YAML 已保存"),
                AppCommand::UpdateMergeConfig { .. } => Some("Merge 配置已保存"),
                AppCommand::UpdateRemoteProfile { .. } => Some("远程配置已更新"),
                AppCommand::SetProfileUpdatePolicy { .. } => Some("更新策略已保存"),
                AppCommand::DeleteProfile { .. } => Some("配置已删除"),
                AppCommand::ExportDiagnostics { .. } => Some("诊断已导出"),
                AppCommand::ExportApplicationSettings { .. } => Some("设置已导出"),
                AppCommand::ImportApplicationSettings { .. } => Some("设置已导入"),
                AppCommand::ResetApplicationSettingsScope { .. } => Some("已恢复默认设置"),
                AppCommand::ExportEncryptedBackup { .. } => Some("加密备份已导出"),
                AppCommand::RestoreEncryptedBackup { .. } => Some("备份已恢复"),
                AppCommand::UpdateMihomo => Some("Mihomo 内核已更新"),
                // UpdateApplicationSettings：设置页的选中态即结果，不再弹 toast。
                _ => None,
            };
            match result {
                Ok(_) => success.map(ok),
                // 设置保存的成功 toast 免了，但失败必须提示。
                Err(error)
                    if success.is_some()
                        || matches!(
                            request,
                            AppCommand::UpdateApplicationSettings { .. }
                                | AppCommand::PreviewApplicationSettingsImport { .. }
                        ) =>
                {
                    Some(fail(error))
                }
                Err(_) => None,
            }
        }
        UiResponse::Runtime { request, result } => {
            let success = match request {
                RuntimeCommand::UpdateProvider { .. } => Some("Provider 已更新"),
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
