use auto_launch::{AutoLaunch, MacOSLaunchMode};
use smappservice_rs::{AppService, ServiceManagementError, ServiceStatus, ServiceType};

use crate::domain::{AppError, ErrorCode};

use super::LoginItemService;

/// Registers the current main `.app` through auto-launch's SMAppService backend.
/// Login starts the normal GUI entry point, which connects to or starts the daemon.
pub struct AutoLaunchLoginItem {
    backend: NativeRegistration,
}

impl AutoLaunchLoginItem {
    pub fn new() -> Self {
        Self {
            backend: NativeRegistration(AutoLaunch::new(
                "",
                "",
                MacOSLaunchMode::SMAppService,
                &[] as &[&str],
                &[] as &[&str],
                "",
            )),
        }
    }
}

impl Default for AutoLaunchLoginItem {
    fn default() -> Self {
        Self::new()
    }
}

impl LoginItemService for AutoLaunchLoginItem {
    fn status(&mut self) -> Result<bool, AppError> {
        enabled_from_status(self.backend.status())
    }

    fn set_enabled(&mut self, enabled: bool) -> Result<(), AppError> {
        set_registration(
            &mut self.backend,
            enabled,
            super::current_app_bundle().is_some(),
        )
    }
}

trait Registration {
    fn status(&self) -> ServiceStatus;
    fn set_enabled(&mut self, enabled: bool) -> auto_launch::Result<()>;
}

struct NativeRegistration(AutoLaunch);

impl Registration for NativeRegistration {
    fn status(&self) -> ServiceStatus {
        // auto-launch 0.6 exposes only is_enabled(). Use its underlying crate to
        // distinguish pending approval from unregistered and missing services.
        AppService::new(ServiceType::MainApp).status()
    }

    fn set_enabled(&mut self, enabled: bool) -> auto_launch::Result<()> {
        if enabled {
            self.0.enable()
        } else {
            self.0.disable()
        }
    }
}

fn enabled_from_status(status: ServiceStatus) -> Result<bool, AppError> {
    match status {
        ServiceStatus::Enabled => Ok(true),
        ServiceStatus::RequiresApproval | ServiceStatus::NotRegistered => Ok(false),
        ServiceStatus::NotFound => Err(login_item_error(
            "read status",
            "the main app service was not found (not running from a bundled .app?)",
        )),
    }
}

fn set_registration(
    backend: &mut impl Registration,
    enabled: bool,
    bundled: bool,
) -> Result<(), AppError> {
    if !bundled {
        return Err(login_item_error(
            "change",
            "launch at login requires running from a bundled .app",
        ));
    }
    let status = backend.status();
    enabled_from_status(status)?;
    if (enabled && status == ServiceStatus::Enabled)
        || (!enabled && status == ServiceStatus::NotRegistered)
    {
        return Ok(());
    }
    // A pending registration already exists. Registering it again cannot grant
    // consent; disabling it must still unregister, despite is_enabled() == false.
    if enabled && status == ServiceStatus::RequiresApproval {
        return Err(approval_required());
    }
    let operation = if enabled { "enable" } else { "disable" };
    backend
        .set_enabled(enabled)
        .map_err(|error| registration_error(operation, error))?;
    match (enabled, backend.status()) {
        (true, ServiceStatus::Enabled) | (false, ServiceStatus::NotRegistered) => Ok(()),
        (true, ServiceStatus::RequiresApproval) => Err(approval_required()),
        _ => Err(login_item_error(
            operation,
            "the system did not apply the requested login item state",
        )),
    }
}

fn approval_required() -> AppError {
    login_item_error(
        "enable",
        "allow Verge in System Settings > General > Login Items",
    )
}

fn registration_error(operation: &str, error: auto_launch::Error) -> AppError {
    let detail = match error {
        auto_launch::Error::SMAppServiceRegistrationFailed(code)
        | auto_launch::Error::SMAppServiceUnregistrationFailed(code) => {
            let cause = ServiceManagementError::try_from(code)
                .unwrap_or(ServiceManagementError::Unknown(code));
            format!("{cause} (ServiceManagement code {code})")
        }
        error => error.to_string(),
    };
    login_item_error(operation, &detail)
}

fn login_item_error(operation: &str, detail: &str) -> AppError {
    AppError::new(
        ErrorCode::PlatformFailed,
        format!("launch at login {operation} failed: {detail}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeRegistration {
        status: ServiceStatus,
        after_change: ServiceStatus,
        failure: Option<auto_launch::Error>,
        calls: Vec<bool>,
    }

    impl FakeRegistration {
        fn new(status: ServiceStatus, after_change: ServiceStatus) -> Self {
            Self {
                status,
                after_change,
                failure: None,
                calls: Vec::new(),
            }
        }
    }

    impl Registration for FakeRegistration {
        fn status(&self) -> ServiceStatus {
            self.status
        }

        fn set_enabled(&mut self, enabled: bool) -> auto_launch::Result<()> {
            self.calls.push(enabled);
            if let Some(error) = self.failure.take() {
                return Err(error);
            }
            self.status = self.after_change;
            Ok(())
        }
    }

    #[test]
    fn registration_checks_bundle_and_missing_service_before_writes() {
        let mut backend =
            FakeRegistration::new(ServiceStatus::NotRegistered, ServiceStatus::Enabled);
        assert!(
            set_registration(&mut backend, true, false)
                .unwrap_err()
                .message
                .contains("bundled .app")
        );
        backend.status = ServiceStatus::NotFound;
        assert!(set_registration(&mut backend, true, true).is_err());
        assert!(backend.calls.is_empty());
    }

    #[test]
    fn registration_is_idempotent_and_verifies_both_changes() {
        let mut backend =
            FakeRegistration::new(ServiceStatus::NotRegistered, ServiceStatus::Enabled);
        set_registration(&mut backend, false, true).unwrap();
        set_registration(&mut backend, true, true).unwrap();
        set_registration(&mut backend, true, true).unwrap();
        backend.after_change = ServiceStatus::NotRegistered;
        set_registration(&mut backend, false, true).unwrap();
        assert_eq!(backend.calls, [true, false]);
        backend.after_change = ServiceStatus::NotRegistered;
        assert!(set_registration(&mut backend, true, true).is_err());
        backend.status = ServiceStatus::Enabled;
        backend.after_change = ServiceStatus::Enabled;
        assert!(set_registration(&mut backend, false, true).is_err());
    }

    #[test]
    fn pending_registration_requires_approval_but_can_be_removed() {
        let mut backend = FakeRegistration::new(
            ServiceStatus::NotRegistered,
            ServiceStatus::RequiresApproval,
        );
        assert!(
            set_registration(&mut backend, true, true)
                .unwrap_err()
                .message
                .contains("System Settings")
        );
        assert!(!enabled_from_status(backend.status).unwrap());
        assert!(
            set_registration(&mut backend, true, true)
                .unwrap_err()
                .message
                .contains("System Settings")
        );
        assert_eq!(backend.calls, [true]);
        backend.after_change = ServiceStatus::NotRegistered;
        set_registration(&mut backend, false, true).unwrap();
        assert_eq!(backend.calls, [true, false]);
    }

    #[test]
    fn registration_errors_keep_native_reason_and_code() {
        let mut backend =
            FakeRegistration::new(ServiceStatus::NotRegistered, ServiceStatus::Enabled);
        let code = ServiceManagementError::InvalidSignature.code();
        backend.failure = Some(auto_launch::Error::SMAppServiceRegistrationFailed(code));
        let error = set_registration(&mut backend, true, true).unwrap_err();
        assert_eq!(error.code, ErrorCode::PlatformFailed);
        assert!(error.message.contains("signature"));
        assert!(error.message.contains(&code.to_string()));
        assert_eq!(backend.status, ServiceStatus::NotRegistered);
    }
}
