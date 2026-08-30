use std::str::FromStr;

use global_hotkey::{GlobalHotKeyManager, hotkey::HotKey};
use verge_domain::{AppError, ErrorCode, GlobalHotkeySpec};

/// 全局快捷键注册后端抽象，便于用 fake 测注册/回滚逻辑。
pub trait HotkeyBackend {
    fn register(&mut self, normalized: &str) -> Result<(), AppError>;
    fn unregister(&mut self, normalized: &str) -> Result<(), AppError>;
}

/// 基于 `global-hotkey` crate 的真实后端。
/// macOS 上 `GlobalHotKeyManager` 必须在主线程创建和调用。
pub struct GlobalHotKeyBackend {
    manager: GlobalHotKeyManager,
}

impl GlobalHotKeyBackend {
    pub fn new() -> Result<Self, AppError> {
        let manager = GlobalHotKeyManager::new().map_err(hotkey_error)?;
        Ok(Self { manager })
    }
}

impl HotkeyBackend for GlobalHotKeyBackend {
    fn register(&mut self, normalized: &str) -> Result<(), AppError> {
        self.manager
            .register(parse_normalized(normalized)?)
            .map_err(hotkey_error)
    }

    fn unregister(&mut self, normalized: &str) -> Result<(), AppError> {
        self.manager
            .unregister(parse_normalized(normalized)?)
            .map_err(hotkey_error)
    }
}

/// 规范化字符串 → `HotKey`。规范化输出与 `HotKey::from_str` 的语法兼容
/// （修饰键与命名键都是它的子集），域校验通过的组合在这里不会解析失败。
fn parse_normalized(normalized: &str) -> Result<HotKey, AppError> {
    HotKey::from_str(normalized).map_err(|error| {
        AppError::new(
            ErrorCode::InvalidInput,
            format!("invalid global hotkey '{normalized}': {error}"),
        )
    })
}

fn hotkey_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::PlatformFailed, error.to_string())
}

/// 当前已注册的全局快捷键（规范化字符串，`None` 表示禁用），
/// 带“先注销旧的再注册新的、失败回滚到旧快捷键”的热更新语义。
pub struct HotkeyRegistration<B> {
    backend: B,
    current: Option<String>,
}

impl<B: HotkeyBackend> HotkeyRegistration<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            current: None,
        }
    }

    /// 同步到新的快捷键设置（`None` = 禁用）。规范化后相同则不动；
    /// 注册失败时尽力回滚到旧快捷键，并返回可读错误。
    pub fn sync(&mut self, desired: Option<&str>) -> Result<(), AppError> {
        let desired = desired
            .map(|value| GlobalHotkeySpec::parse(value).map(|spec| spec.normalized()))
            .transpose()?;
        if desired == self.current {
            return Ok(());
        }
        let previous = self.current.clone();
        if let Some(old) = &previous {
            // 注销失败时保持现状不动，避免系统侧与本地状态错位。
            self.backend.unregister(old)?;
        }
        self.current = None;
        let Some(new) = desired else {
            return Ok(());
        };
        if let Err(error) = self.backend.register(&new) {
            self.current = match &previous {
                Some(old) if self.backend.register(old).is_ok() => previous,
                _ => None,
            };
            let restored = match &self.current {
                Some(old) => format!("已恢复为 {old}"),
                None => "旧快捷键恢复失败，当前无生效快捷键".to_owned(),
            };
            return Err(AppError::new(
                error.code,
                format!(
                    "全局快捷键 {new} 注册失败（组合键无效或已被占用：{}），{restored}",
                    error.message
                ),
            ));
        }
        self.current = Some(new);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verge_domain::GLOBAL_HOTKEY_NAMED_KEYS;

    #[derive(Default)]
    struct FakeBackend {
        registered: Option<String>,
        calls: Vec<String>,
        /// 接下来 N 次 register 调用失败。
        register_failures: usize,
        fail_unregister: bool,
    }

    impl HotkeyBackend for FakeBackend {
        fn register(&mut self, normalized: &str) -> Result<(), AppError> {
            self.calls.push(format!("register:{normalized}"));
            if self.register_failures > 0 {
                self.register_failures -= 1;
                return Err(AppError::new(ErrorCode::PlatformFailed, "hotkey is in use"));
            }
            self.registered = Some(normalized.to_owned());
            Ok(())
        }

        fn unregister(&mut self, normalized: &str) -> Result<(), AppError> {
            self.calls.push(format!("unregister:{normalized}"));
            if self.fail_unregister {
                return Err(AppError::new(ErrorCode::PlatformFailed, "not registered"));
            }
            self.registered = None;
            Ok(())
        }
    }

    #[test]
    fn sync_registers_unregisters_in_order_and_disables() {
        let mut registration = HotkeyRegistration::new(FakeBackend::default());
        registration.sync(Some("cmd+shift+v")).unwrap();
        assert_eq!(registration.current.as_deref(), Some("CmdOrCtrl+Shift+V"));
        registration.sync(Some("Alt+F5")).unwrap();
        assert_eq!(
            registration.backend.calls,
            [
                "register:CmdOrCtrl+Shift+V",
                "unregister:CmdOrCtrl+Shift+V",
                "register:Alt+F5"
            ]
        );
        registration.sync(None).unwrap();
        assert_eq!(registration.current.as_deref(), None);
        assert_eq!(
            registration.backend.calls.last().map(String::as_str),
            Some("unregister:Alt+F5")
        );
    }

    #[test]
    fn sync_is_idempotent_for_normalized_equal_values() {
        let mut registration = HotkeyRegistration::new(FakeBackend::default());
        registration.sync(Some("CmdOrCtrl+Shift+V")).unwrap();
        registration.sync(Some("shift+command+v")).unwrap();
        assert_eq!(registration.backend.calls.len(), 1);
    }

    #[test]
    fn failed_registration_rolls_back_to_previous_hotkey() {
        let mut registration = HotkeyRegistration::new(FakeBackend::default());
        registration.sync(Some("CmdOrCtrl+Shift+V")).unwrap();
        registration.backend.register_failures = 1;
        let error = registration.sync(Some("Alt+F5")).unwrap_err();
        assert_eq!(error.code, ErrorCode::PlatformFailed);
        assert!(error.message.contains("Alt+F5"));
        assert!(error.message.contains("已恢复为 CmdOrCtrl+Shift+V"));
        assert_eq!(registration.current.as_deref(), Some("CmdOrCtrl+Shift+V"));
        assert_eq!(
            registration.backend.registered.as_deref(),
            Some("CmdOrCtrl+Shift+V")
        );
    }

    #[test]
    fn failed_rollback_reports_no_active_hotkey() {
        let mut registration = HotkeyRegistration::new(FakeBackend::default());
        registration.sync(Some("CmdOrCtrl+Shift+V")).unwrap();
        // 新快捷键注册失败，回滚旧快捷键也失败。
        registration.backend.register_failures = 2;
        let error = registration.sync(Some("Alt+F5")).unwrap_err();
        assert!(error.message.contains("旧快捷键恢复失败"));
        assert_eq!(registration.current.as_deref(), None);
    }

    #[test]
    fn failed_registration_without_previous_reports_no_active_hotkey() {
        let mut registration = HotkeyRegistration::new(FakeBackend::default());
        registration.backend.register_failures = 1;
        let error = registration.sync(Some("Alt+F5")).unwrap_err();
        assert!(error.message.contains("当前无生效快捷键"));
        assert_eq!(registration.current.as_deref(), None);
    }

    #[test]
    fn failed_unregister_keeps_current_hotkey() {
        let mut registration = HotkeyRegistration::new(FakeBackend::default());
        registration.sync(Some("CmdOrCtrl+Shift+V")).unwrap();
        registration.backend.fail_unregister = true;
        assert_eq!(
            registration.sync(Some("Alt+F5")).unwrap_err().code,
            ErrorCode::PlatformFailed
        );
        assert_eq!(registration.current.as_deref(), Some("CmdOrCtrl+Shift+V"));
    }

    #[test]
    fn invalid_hotkey_is_rejected_before_touching_backend() {
        let mut registration = HotkeyRegistration::new(FakeBackend::default());
        assert_eq!(
            registration.sync(Some("not-a-hotkey")).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        assert!(registration.backend.calls.is_empty());
    }

    #[test]
    fn every_domain_accepted_key_parses_as_global_hotkey() {
        for key in GLOBAL_HOTKEY_NAMED_KEYS
            .iter()
            .map(|name| (*name).to_owned())
            .chain(('A'..='Z').map(|letter| letter.to_string()))
            .chain(('0'..='9').map(|digit| digit.to_string()))
            .chain((1..=12).map(|number| format!("F{number}")))
        {
            let normalized = GlobalHotkeySpec::parse(&format!("CmdOrCtrl+{key}"))
                .unwrap()
                .normalized();
            parse_normalized(&normalized).unwrap_or_else(|error| {
                panic!("domain-accepted hotkey {normalized} must parse: {error}")
            });
        }
    }

    #[test]
    fn normalized_string_maps_to_expected_modifiers_and_code() {
        use global_hotkey::hotkey::{Code, Modifiers};
        let hotkey = parse_normalized("CmdOrCtrl+Shift+V").unwrap();
        #[cfg(target_os = "macos")]
        assert!(hotkey.matches(Modifiers::SUPER | Modifiers::SHIFT, Code::KeyV));
        #[cfg(not(target_os = "macos"))]
        assert!(hotkey.matches(Modifiers::CONTROL | Modifiers::SHIFT, Code::KeyV));
    }
}
