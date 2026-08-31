use objc2_service_management::{SMAppService, SMAppServiceStatus};
use verge_domain::{AppError, ErrorCode};

use crate::LoginItemService;

/// 基于 `SMAppService.mainApp` 的登录项实现（macOS 13+）。
/// 只对打包成 `.app` 的进程有意义；裸二进制（如 `cargo run`）注册会失败，
/// 错误里会带上这一前提说明，调用方原样透出即可。
///
/// 双进程语义：注册的是 `.app` 主 bundle。登录时 LaunchServices 直接拉起
/// Verge.app（等价于双击启动）→ `main()` 默认 GUI 模式 →
/// `connect_or_start_daemon` 经 IPC 发现无守护进程后拉起 `--daemon` 分叉
/// （托盘、全局热键都在守护侧）。登录后主窗口随之打开，与
/// “launch at login 打开应用”的用户预期一致。
/// 结构要求只有标准 `.app` bundle（`Contents/MacOS/<CFBundleExecutable>` +
/// Info.plist 带 CFBundleIdentifier，见 apps/verge-gpui/macos/Info.plist），
/// 不需要 SMAppService.LoginItem 那类嵌套 helper 布局；`LSMinimumSystemVersion`
/// 13.0 满足 SMAppService 的可用版本。
pub struct SmAppLoginItem {
    _private: (),
}

impl SmAppLoginItem {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for SmAppLoginItem {
    fn default() -> Self {
        Self::new()
    }
}

impl LoginItemService for SmAppLoginItem {
    fn status(&mut self) -> Result<bool, AppError> {
        // SAFETY: mainAppService/status 无前置条件；对象生命周期由 Retained 管理。
        let status = unsafe { SMAppService::mainAppService().status() };
        match status {
            SMAppServiceStatus::Enabled | SMAppServiceStatus::RequiresApproval => Ok(true),
            SMAppServiceStatus::NotRegistered => Ok(false),
            _ => Err(login_item_error(
                "read status",
                "the main app service was not found (not running from a bundled .app?)",
            )),
        }
    }

    fn set_enabled(&mut self, enabled: bool) -> Result<(), AppError> {
        // SAFETY: register/unregisterAndReturnError 无前置条件，错误通过 Result 返回。
        let service = unsafe { SMAppService::mainAppService() };
        let result = unsafe {
            if enabled {
                service.registerAndReturnError()
            } else {
                service.unregisterAndReturnError()
            }
        };
        result.map_err(|error| {
            login_item_error(
                if enabled { "enable" } else { "disable" },
                &format!(
                    "{} (launch at login requires running from a bundled .app)",
                    error.localizedDescription()
                ),
            )
        })
    }
}

fn login_item_error(operation: &str, detail: &str) -> AppError {
    AppError::new(
        ErrorCode::PlatformFailed,
        format!("launch at login {operation} failed: {detail}"),
    )
}
