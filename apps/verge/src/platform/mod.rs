mod pac;
pub use pac::PacServer;
use std::{
    fs,
    fs::File,
    io,
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::Command,
};

use fs2::FileExt;

use crate::domain::{
    AppError, AutoProxyState, ErrorCode, HelperStatus, ProxyEndpoint, ProxyProtocolState,
    SystemProxyServiceState, SystemProxyState, SystemProxyTarget,
};
pub use verge_helper_protocol::TunConfig;
use verge_helper_protocol::{HelperRequest, HelperResponse, MAX_REQUEST_BYTES, PROTOCOL_VERSION};

mod app_bundle;
pub use app_bundle::{bundle_short_version, current_app_bundle, directory_writable};

mod helper_install;
pub use helper_install::{
    HELPER_LABEL, HELPER_SOCKET_PATH, HelperBundle, HelperInstallLayout, MacHelperInstaller,
    bundled_resources_directory, current_uid, discover_bundled_helper, run_privileged_script,
    shell_quote,
};

#[cfg(target_os = "macos")]
mod tray;
#[cfg(target_os = "macos")]
pub use tray::{TrayCommand, TrayDirectory, TrayMenuState, TrayService, TraySnapshot};

#[cfg(target_os = "macos")]
mod hotkey;
#[cfg(target_os = "macos")]
pub use hotkey::{GlobalHotKeyBackend, HotkeyBackend, HotkeyRegistration, spawn_hotkey_listener};

mod daemon;
pub use daemon::{
    daemon_socket_path, redirect_stderr, redirect_stderr_to_log, restore_gui_windows,
    run_accessory_appkit_loop, spawn_daemon,
};

#[cfg(target_os = "macos")]
mod login_item;
#[cfg(target_os = "macos")]
pub use login_item::AutoLaunchLoginItem;

/// 登录启动（launch at login）能力抽象，实现可注入、可 fake 测试。
pub trait LoginItemService {
    /// 当前是否已启用登录项（等待系统批准不视为启用）。
    fn status(&mut self) -> Result<bool, AppError>;
    /// 注册或注销登录项。
    fn set_enabled(&mut self, enabled: bool) -> Result<(), AppError>;
}

/// 当前平台的默认登录启动实现。
pub fn default_login_item_service() -> Box<dyn LoginItemService + Send> {
    #[cfg(target_os = "macos")]
    {
        Box::new(AutoLaunchLoginItem::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(UnsupportedLoginItem)
    }
}

#[cfg(not(target_os = "macos"))]
struct UnsupportedLoginItem;

#[cfg(not(target_os = "macos"))]
impl LoginItemService for UnsupportedLoginItem {
    fn status(&mut self) -> Result<bool, AppError> {
        Err(platform_error(
            "launch at login is not supported on this platform",
        ))
    }

    fn set_enabled(&mut self, _enabled: bool) -> Result<(), AppError> {
        Err(platform_error(
            "launch at login is not supported on this platform",
        ))
    }
}

#[cfg(test)]
const NETWORK_SETUP: &str = "/usr/sbin/networksetup";
pub use sysproxy::macos::ProcessRunner as SystemProxyRunner;
use sysproxy::macos::{Networksetup, ProxyType};
const SECURITY: &str = "/usr/bin/security";
const OSASCRIPT: &str = "/usr/bin/osascript";

pub struct MacHelperClient {
    socket: PathBuf,
}

impl MacHelperClient {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    pub fn status(&self) -> HelperStatus {
        match self.exchange(&HelperRequest::Ping {
            protocol_version: PROTOCOL_VERSION,
        }) {
            Ok(HelperResponse::Pong { protocol_version }) => {
                HelperStatus::Ready { protocol_version }
            }
            Ok(HelperResponse::Error { message, .. }) => HelperStatus::Incompatible { message },
            Ok(response) => HelperStatus::Incompatible {
                message: format!("unexpected helper response: {response:?}"),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => HelperStatus::NotInstalled,
            Err(error) => HelperStatus::Incompatible {
                message: error.to_string(),
            },
        }
    }

    /// helper 自述的能力集（协议版本 + TUN 生命周期支持）。
    pub fn capabilities(&self) -> Result<HelperCapabilities, AppError> {
        match self.exchange(&HelperRequest::GetCapabilities) {
            Ok(HelperResponse::Capabilities {
                protocol_version,
                tun_lifecycle,
            }) => Ok(HelperCapabilities {
                protocol_version,
                tun_lifecycle,
            }),
            Ok(HelperResponse::Error { code, message }) => Err(helper_call_error(&code, &message)),
            Ok(response) => Err(platform_error(format!(
                "unexpected helper response: {response:?}"
            ))),
            Err(error) => Err(platform_io_error(error)),
        }
    }

    /// 请 helper 创建并配置 TUN 设备，返回实际设备名。
    pub fn enable_tun(&self, config: &TunConfig) -> Result<String, AppError> {
        match self.exchange(&HelperRequest::EnableTun {
            config: config.clone(),
        }) {
            Ok(HelperResponse::TunEnabled { device }) => Ok(device),
            Ok(HelperResponse::Error { code, message }) => Err(helper_call_error(&code, &message)),
            Ok(response) => Err(platform_error(format!(
                "unexpected helper response: {response:?}"
            ))),
            Err(error) => Err(platform_io_error(error)),
        }
    }

    /// 拆除 helper 管理的 TUN 设备；None 表示全部拆除。
    pub fn disable_tun(&self, device: Option<&str>) -> Result<(), AppError> {
        match self.exchange(&HelperRequest::DisableTun {
            device: device.map(str::to_owned),
        }) {
            Ok(HelperResponse::TunDisabled) => Ok(()),
            Ok(HelperResponse::Error { code, message }) => Err(helper_call_error(&code, &message)),
            Ok(response) => Err(platform_error(format!(
                "unexpected helper response: {response:?}"
            ))),
            Err(error) => Err(platform_io_error(error)),
        }
    }

    fn exchange(&self, request: &HelperRequest) -> io::Result<HelperResponse> {
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
        serde_json::to_writer(&mut stream, request)?;
        stream.write_all(b"\n")?;
        let mut response = String::new();
        BufReader::new(stream)
            .take(MAX_REQUEST_BYTES)
            .read_line(&mut response)?;
        serde_json::from_str(&response).map_err(io::Error::other)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HelperCapabilities {
    pub protocol_version: u32,
    pub tun_lifecycle: bool,
}

/// 应用层面对的 helper 抽象：状态、能力、TUN 生命周期。
/// 真实实现是 MacHelperClient；测试注入 fake。
pub trait HelperControl {
    fn status(&self) -> HelperStatus;
    fn tun_supported(&self) -> Result<bool, AppError>;
    fn enable_tun(&self, config: &TunConfig) -> Result<String, AppError>;
    fn disable_tun(&self, device: Option<&str>) -> Result<(), AppError>;
}

impl HelperControl for MacHelperClient {
    fn status(&self) -> HelperStatus {
        MacHelperClient::status(self)
    }

    fn tun_supported(&self) -> Result<bool, AppError> {
        Ok(self.capabilities()?.tun_lifecycle)
    }

    fn enable_tun(&self, config: &TunConfig) -> Result<String, AppError> {
        MacHelperClient::enable_tun(self, config)
    }

    fn disable_tun(&self, device: Option<&str>) -> Result<(), AppError> {
        MacHelperClient::disable_tun(self, device)
    }
}

fn helper_call_error(code: &str, message: &str) -> AppError {
    let code = match code {
        "unauthorized_peer" => ErrorCode::PermissionDenied,
        _ => ErrorCode::PlatformFailed,
    };
    AppError::new(code, format!("helper: {message}"))
}

#[derive(Debug)]
pub struct SingleInstance {
    _lock: File,
}

impl SingleInstance {
    pub fn acquire(data_directory: impl AsRef<Path>) -> Result<Self, AppError> {
        fs::create_dir_all(data_directory.as_ref()).map_err(platform_io_error)?;
        let path = data_directory.as_ref().join("verge.lock");
        let lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(platform_io_error)?;
        lock.try_lock_exclusive().map_err(|error| {
            AppError::new(
                ErrorCode::Conflict,
                format!("another Verge instance is already running: {error}"),
            )
        })?;
        Ok(Self { _lock: lock })
    }
}

pub struct MacKeychain<R> {
    runner: R,
    service: String,
}

impl<R: CommandRunner> MacKeychain<R> {
    pub fn new(runner: R, service: impl Into<String>) -> Result<Self, AppError> {
        let service = service.into();
        validate_argument("keychain service", &service)?;
        Ok(Self { runner, service })
    }

    pub fn set(&mut self, account: &str, secret: &str) -> Result<(), AppError> {
        validate_argument("keychain account", account)?;
        validate_secret(secret)?;
        self.runner.run(
            SECURITY,
            &strings(&[
                "add-generic-password",
                "-U",
                "-a",
                account,
                "-s",
                &self.service,
                "-w",
                secret,
            ]),
        )?;
        Ok(())
    }

    pub fn get(&mut self, account: &str) -> Result<String, AppError> {
        validate_argument("keychain account", account)?;
        self.runner
            .run(
                SECURITY,
                &strings(&[
                    "find-generic-password",
                    "-a",
                    account,
                    "-s",
                    &self.service,
                    "-w",
                ]),
            )
            .map(|value| value.trim_end_matches(['\r', '\n']).to_owned())
    }

    pub fn delete(&mut self, account: &str) -> Result<(), AppError> {
        validate_argument("keychain account", account)?;
        self.runner.run(
            SECURITY,
            &strings(&[
                "delete-generic-password",
                "-a",
                account,
                "-s",
                &self.service,
            ]),
        )?;
        Ok(())
    }
}

pub struct MacNotifier<R> {
    runner: R,
}

impl<R: CommandRunner> MacNotifier<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn notify(&mut self, title: &str, body: &str) -> Result<(), AppError> {
        validate_argument("notification title", title)?;
        validate_argument("notification body", body)?;
        const SCRIPT: &str = "on run argv\n display notification (item 2 of argv) with title (item 1 of argv)\nend run";
        self.runner
            .run(OSASCRIPT, &strings(&["-e", SCRIPT, "--", title, body]))?;
        Ok(())
    }
}

fn validate_argument(name: &str, value: &str) -> Result<(), AppError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("{name} must be non-empty and contain no control characters"),
        ));
    }
    Ok(())
}

fn validate_secret(secret: &str) -> Result<(), AppError> {
    if secret.is_empty() || secret.contains('\0') {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "keychain secret must be non-empty and contain no NUL byte",
        ));
    }
    Ok(())
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub trait CommandRunner {
    fn run(&mut self, program: &str, args: &[String]) -> Result<String, AppError>;
}

/// Application-facing system proxy port. Platform implementations keep their
/// native command and recovery details behind this interface.
pub trait SystemProxyPlatform {
    fn configure(
        &mut self,
        services: &[String],
        target: &SystemProxyTarget,
    ) -> Result<SystemProxyState, AppError>;
    fn restore_snapshot(&mut self, state: &SystemProxyState) -> Result<(), AppError>;
    fn state(&mut self, services: &[String]) -> Result<SystemProxyState, AppError>;
    fn recovery_pending(&self) -> bool;
    fn list_network_services(&mut self) -> Result<Vec<String>, AppError>;
    fn enable(
        &mut self,
        services: &[String],
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError>;
    fn disable(&mut self, services: &[String]) -> Result<SystemProxyState, AppError>;
    fn recover_pending(&mut self) -> Result<SystemProxyState, AppError>;
    fn set_socks(
        &mut self,
        services: &[String],
        enabled: bool,
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError>;
    fn set_auto_proxy(
        &mut self,
        services: &[String],
        url: Option<&str>,
    ) -> Result<SystemProxyState, AppError>;
    fn set_bypass(
        &mut self,
        services: &[String],
        domains: &[String],
    ) -> Result<SystemProxyState, AppError>;
}

#[derive(Default)]
pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run(&mut self, program: &str, args: &[String]) -> Result<String, AppError> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(platform_io_error)?;
        if !output.status.success() {
            return Err(platform_error(
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
        String::from_utf8(output.stdout).map_err(platform_error)
    }
}

pub struct MacSystemProxy<R> {
    runner: R,
    recovery_path: PathBuf,
}

// Retain the same read used by sysproxy so legacy recovery can inspect a value
// rejected by its strict endpoint parser, without issuing a second OS query.
struct ProxyReadCapture<'a, R> {
    runner: &'a mut R,
    stdout: Option<String>,
}

impl<R: sysproxy::macos::CommandRunner> sysproxy::macos::CommandRunner for ProxyReadCapture<'_, R> {
    fn run(&mut self, args: &[&str]) -> io::Result<std::process::Output> {
        let output = self.runner.run(args)?;
        self.stdout = output
            .status
            .success()
            .then(|| String::from_utf8(output.stdout.clone()).ok())
            .flatten();
        Ok(output)
    }
}

impl<R: sysproxy::macos::CommandRunner> MacSystemProxy<R> {
    pub fn new(runner: R, recovery_path: impl Into<PathBuf>) -> Self {
        Self {
            runner,
            recovery_path: recovery_path.into(),
        }
    }

    pub fn state(&mut self, services: &[String]) -> Result<SystemProxyState, AppError> {
        validate_services(services)?;
        let mut states = Vec::with_capacity(services.len());
        for service in services {
            states.push(self.snapshot(service)?);
        }
        Ok(SystemProxyState {
            services: states,
            recovery_pending: self.recovery_path.is_file(),
        })
    }

    pub fn recovery_pending(&self) -> bool {
        self.recovery_path.is_file()
    }

    pub fn list_network_services(&mut self) -> Result<Vec<String>, AppError> {
        Networksetup::new(&mut self.runner)
            .list_network_services()
            .map_err(platform_error)
    }

    pub fn enable(
        &mut self,
        services: &[String],
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError> {
        ProxyEndpoint::new(&endpoint.host, endpoint.port)?;
        self.apply_change(services, |proxy, service| {
            proxy.apply_endpoint(service, endpoint)
        })
    }

    /// Retarget only enabled listeners that still point to Verge's previous endpoints.
    /// apply_change retains the original recovery record and rolls back partial writes.
    pub fn remap_endpoints(
        &mut self,
        services: &[String],
        old_http: &ProxyEndpoint,
        old_socks: Option<&ProxyEndpoint>,
        new: &ProxyEndpoint,
    ) -> Result<(), AppError> {
        if !self.recovery_path.exists() {
            return Ok(());
        }
        let state = self.state(services)?;
        self.apply_change(services, |proxy, service| {
            let current = state
                .services
                .iter()
                .find(|s| s.service == service)
                .expect("queried service");
            for (protocol, value, old) in [
                (ProxyType::Http, &current.web, Some(old_http)),
                (ProxyType::Https, &current.secure_web, Some(old_http)),
                (ProxyType::Socks, &current.socks, old_socks),
            ] {
                if value.enabled && old == Some(&value.endpoint) && value.endpoint != *new {
                    proxy.set_protocol(
                        protocol,
                        service,
                        &ProxyProtocolState {
                            enabled: true,
                            endpoint: new.clone(),
                        },
                    )?;
                }
            }
            Ok(())
        })?;
        Ok(())
    }

    pub fn disable(&mut self, services: &[String]) -> Result<SystemProxyState, AppError> {
        if self.recovery_pending() {
            return self.recover_pending();
        }
        // A proxy may have been enabled outside Verge. Turning it off must also work
        // without a Verge recovery file. The master switch includes SOCKS and PAC.
        self.apply_change(services, |proxy, service| {
            for protocol in [ProxyType::Http, ProxyType::Https, ProxyType::Socks] {
                proxy.set_protocol_enabled(protocol, service, false)?;
            }
            proxy.set_auto_proxy_enabled(service, false)?;
            Ok(())
        })?;
        remove_recovery(&self.recovery_path)?;
        self.state(services)
    }

    pub fn recover_pending(&mut self) -> Result<SystemProxyState, AppError> {
        let previous = read_recovery(&self.recovery_path)?;
        let mut restored = self.restore_services(&previous.services)?;
        remove_recovery(&self.recovery_path)?;
        restored.recovery_pending = false;
        Ok(restored)
    }

    pub fn configure(
        &mut self,
        services: &[String],
        target: &SystemProxyTarget,
    ) -> Result<SystemProxyState, AppError> {
        if let SystemProxyTarget::Manual { endpoint, bypass } = target {
            ProxyEndpoint::new(&endpoint.host, endpoint.port)?;
            for domain in bypass {
                validate_argument("proxy bypass domain", domain)?;
            }
        }
        if let SystemProxyTarget::Pac { url } = target {
            validate_argument("PAC URL", url)?;
        }
        self.apply_change_checked(
            services,
            |proxy, service| match target {
                SystemProxyTarget::Manual { endpoint, bypass } => {
                    proxy.apply_endpoint(service, endpoint)?;
                    proxy.set_protocol(
                        ProxyType::Socks,
                        service,
                        &ProxyProtocolState {
                            enabled: true,
                            endpoint: endpoint.clone(),
                        },
                    )?;
                    proxy.apply_bypass(service, bypass)
                }
                SystemProxyTarget::Pac { url } => {
                    for protocol in [ProxyType::Http, ProxyType::Https, ProxyType::Socks] {
                        proxy.set_protocol_enabled(protocol, service, false)?;
                    }
                    proxy.apply_auto_proxy(
                        service,
                        &AutoProxyState {
                            enabled: true,
                            url: Some(url.clone()),
                        },
                    )
                }
            },
            |state| {
                if target.matches(state) {
                    Ok(())
                } else {
                    Err(platform_error(
                        "system proxy did not match the requested configuration after applying it",
                    ))
                }
            },
        )
    }

    pub fn set_socks(
        &mut self,
        services: &[String],
        enabled: bool,
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError> {
        ProxyEndpoint::new(&endpoint.host, endpoint.port)?;
        self.apply_change(services, |proxy, service| {
            proxy.set_protocol(
                ProxyType::Socks,
                service,
                &ProxyProtocolState {
                    enabled,
                    endpoint: endpoint.clone(),
                },
            )
        })
    }

    pub fn set_auto_proxy(
        &mut self,
        services: &[String],
        url: Option<&str>,
    ) -> Result<SystemProxyState, AppError> {
        if let Some(url) = url {
            validate_argument("auto proxy url", url)?;
        }
        let state = AutoProxyState {
            enabled: url.is_some(),
            url: url.map(str::to_owned),
        };
        self.apply_change(services, |proxy, service| {
            if state.enabled {
                proxy.apply_auto_proxy(service, &state)
            } else {
                proxy.set_auto_proxy_enabled(service, false)
            }
        })
    }

    pub fn set_bypass(
        &mut self,
        services: &[String],
        domains: &[String],
    ) -> Result<SystemProxyState, AppError> {
        for domain in domains {
            validate_argument("proxy bypass domain", domain)?;
        }
        self.apply_change(services, |proxy, service| {
            proxy.apply_bypass(service, domains)
        })
    }

    /// 快照当前状态，逐个网络服务应用变更，失败时按快照回滚。
    /// 恢复记录只在不存在时写入（记录永远是首个变更前的用户原始状态），
    /// 也只有本调用创建它时才会在回滚成功后移除。
    fn apply_change(
        &mut self,
        services: &[String],
        apply: impl FnMut(&mut Self, &str) -> Result<(), AppError>,
    ) -> Result<SystemProxyState, AppError> {
        self.apply_change_checked(services, apply, |_| Ok(()))
    }

    fn apply_change_checked(
        &mut self,
        services: &[String],
        mut apply: impl FnMut(&mut Self, &str) -> Result<(), AppError>,
        verify: impl FnOnce(&SystemProxyState) -> Result<(), AppError>,
    ) -> Result<SystemProxyState, AppError> {
        validate_services(services)?;
        let previous = self.state(services)?;
        let created_record = !self.recovery_path.exists();
        if created_record {
            write_recovery(&self.recovery_path, &previous)?;
        }
        let result = (|| {
            for service in services {
                apply(self, service)?;
            }
            let state = self.state(services)?;
            verify(&state)?;
            Ok(state)
        })();
        match result {
            Ok(state) => Ok(state),
            Err(cause) => match self.restore_services(&previous.services) {
                Ok(_) => {
                    if created_record {
                        remove_recovery(&self.recovery_path)?;
                    }
                    Err(cause)
                }
                Err(rollback) => Err(platform_error(format!(
                    "system proxy apply failed: {cause}; rollback failed: {rollback}"
                ))),
            },
        }
    }

    fn snapshot(&mut self, service: &str) -> Result<SystemProxyServiceState, AppError> {
        Ok(SystemProxyServiceState {
            service: service.to_owned(),
            web: self.get_protocol(ProxyType::Http, service)?,
            secure_web: self.get_protocol(ProxyType::Https, service)?,
            socks: self.get_protocol(ProxyType::Socks, service)?,
            auto_proxy: self.get_auto_proxy(service)?,
            bypass: self.get_bypass(service)?,
        })
    }

    fn get_protocol(
        &mut self,
        protocol: ProxyType,
        service: &str,
    ) -> Result<ProxyProtocolState, AppError> {
        self.read_protocol(protocol, service, None)
    }

    fn read_protocol(
        &mut self,
        protocol: ProxyType,
        service: &str,
        legacy: Option<&ProxyProtocolState>,
    ) -> Result<ProxyProtocolState, AppError> {
        let result = if let Some(expected) = legacy.filter(|value| legacy_disabled_endpoint(value))
        {
            let mut reader = ProxyReadCapture {
                runner: &mut self.runner,
                stdout: None,
            };
            let result = Networksetup::new(&mut reader).get_proxy(protocol, service);
            if result.is_err()
                && let Some(text) = reader.stdout.as_deref()
                && legacy_protocol_output_matches(text, expected)
            {
                return Ok(expected.clone());
            }
            result
        } else {
            Networksetup::new(&mut self.runner).get_proxy(protocol, service)
        };
        let proxy = result.map_err(platform_error)?;
        Ok(ProxyProtocolState {
            enabled: proxy.enable,
            endpoint: ProxyEndpoint {
                host: proxy.host,
                port: proxy.port,
            },
        })
    }

    fn get_auto_proxy(&mut self, service: &str) -> Result<AutoProxyState, AppError> {
        let proxy = Networksetup::new(&mut self.runner)
            .get_auto_proxy(service)
            .map_err(platform_error)?;
        Ok(AutoProxyState {
            enabled: proxy.enable,
            url: (!proxy.url.is_empty()).then_some(proxy.url),
        })
    }

    fn get_bypass(&mut self, service: &str) -> Result<Vec<String>, AppError> {
        let bypass = Networksetup::new(&mut self.runner)
            .get_bypass(service)
            .map_err(platform_error)?;
        Ok(bypass
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect())
    }

    fn set_protocol_enabled(
        &mut self,
        protocol: ProxyType,
        service: &str,
        enabled: bool,
    ) -> Result<(), AppError> {
        Networksetup::new(&mut self.runner)
            .set_proxy_enabled(protocol, service, enabled)
            .map_err(platform_error)
    }

    fn set_auto_proxy_enabled(&mut self, service: &str, enabled: bool) -> Result<(), AppError> {
        Networksetup::new(&mut self.runner)
            .set_auto_proxy_enabled(service, enabled)
            .map_err(platform_error)
    }

    fn apply_endpoint(&mut self, service: &str, endpoint: &ProxyEndpoint) -> Result<(), AppError> {
        let state = ProxyProtocolState {
            enabled: true,
            endpoint: endpoint.clone(),
        };
        // PAC takes precedence on macOS; preserve it in the recovery snapshot,
        // then disable it before enabling explicit HTTP/HTTPS proxy endpoints.
        self.set_auto_proxy_enabled(service, false)?;
        self.set_protocol(ProxyType::Http, service, &state)?;
        self.set_protocol(ProxyType::Https, service, &state)
    }

    fn restore_services(
        &mut self,
        states: &[SystemProxyServiceState],
    ) -> Result<SystemProxyState, AppError> {
        validate_services(
            &states
                .iter()
                .map(|state| state.service.clone())
                .collect::<Vec<_>>(),
        )?;
        let mut errors = Vec::new();
        for state in states {
            for (protocol, name, value) in [
                (ProxyType::Http, "web", &state.web),
                (ProxyType::Https, "secure web", &state.secure_web),
                (ProxyType::Socks, "socks", &state.socks),
            ] {
                let result = if legacy_disabled_endpoint(value) {
                    self.set_protocol_enabled(protocol, &state.service, false)
                } else {
                    self.set_protocol(protocol, &state.service, value)
                };
                if let Err(error) = result {
                    errors.push(format!("{} {name}: {error}", state.service));
                }
            }
            if let Err(error) = self.apply_auto_proxy(&state.service, &state.auto_proxy) {
                errors.push(format!("{} automatic proxy: {error}", state.service));
            }
            if let Err(error) = self.apply_bypass(&state.service, &state.bypass) {
                errors.push(format!("{} bypass domains: {error}", state.service));
            }
        }
        if !errors.is_empty() {
            return Err(platform_error(format!(
                "system proxy recovery was incomplete: {}",
                errors.join("; ")
            )));
        }
        let mut restored = SystemProxyState {
            services: Vec::with_capacity(states.len()),
            recovery_pending: self.recovery_pending(),
        };
        for expected in states {
            let service = &expected.service;
            let actual = SystemProxyServiceState {
                service: service.clone(),
                web: self.read_protocol(ProxyType::Http, service, Some(&expected.web))?,
                secure_web: self.read_protocol(
                    ProxyType::Https,
                    service,
                    Some(&expected.secure_web),
                )?,
                socks: self.read_protocol(ProxyType::Socks, service, Some(&expected.socks))?,
                auto_proxy: self.get_auto_proxy(service)?,
                bypass: self.get_bypass(service)?,
            };
            if !restored_protocol_matches(&expected.web, &actual.web)
                || !restored_protocol_matches(&expected.secure_web, &actual.secure_web)
                || !restored_protocol_matches(&expected.socks, &actual.socks)
                || expected.auto_proxy != actual.auto_proxy
                || expected.bypass != actual.bypass
            {
                return Err(platform_error(format!(
                    "system proxy did not match the recovery snapshot for {}",
                    expected.service
                )));
            }
            restored.services.push(actual);
        }
        Ok(restored)
    }

    fn set_protocol(
        &mut self,
        protocol: ProxyType,
        service: &str,
        state: &ProxyProtocolState,
    ) -> Result<(), AppError> {
        if state.enabled {
            ProxyEndpoint::new(&state.endpoint.host, state.endpoint.port)?;
        }
        let proxy = sysproxy::Sysproxy {
            enable: state.enabled,
            host: state.endpoint.host.clone(),
            port: state.endpoint.port,
            bypass: String::new(),
        };
        Networksetup::new(&mut self.runner)
            .set_proxy(protocol, service, &proxy)
            .map_err(platform_error)
    }

    fn apply_auto_proxy(&mut self, service: &str, state: &AutoProxyState) -> Result<(), AppError> {
        let proxy = sysproxy::Autoproxy {
            enable: state.enabled,
            url: state.url.clone().unwrap_or_default(),
        };
        Networksetup::new(&mut self.runner)
            .set_auto_proxy(service, &proxy)
            .map_err(platform_error)
    }

    fn apply_bypass(&mut self, service: &str, domains: &[String]) -> Result<(), AppError> {
        for domain in domains {
            validate_argument("proxy bypass domain", domain)?;
        }
        Networksetup::new(&mut self.runner)
            .set_bypass(service, &domains.join(","))
            .map_err(platform_error)
    }
}

impl<R: sysproxy::macos::CommandRunner> SystemProxyPlatform for MacSystemProxy<R> {
    fn configure(
        &mut self,
        services: &[String],
        target: &SystemProxyTarget,
    ) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::configure(self, services, target)
    }
    fn restore_snapshot(&mut self, state: &SystemProxyState) -> Result<(), AppError> {
        self.restore_services(&state.services).map(|_| ())
    }
    fn state(&mut self, services: &[String]) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::state(self, services)
    }

    fn recovery_pending(&self) -> bool {
        MacSystemProxy::recovery_pending(self)
    }

    fn list_network_services(&mut self) -> Result<Vec<String>, AppError> {
        MacSystemProxy::list_network_services(self)
    }

    fn enable(
        &mut self,
        services: &[String],
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::enable(self, services, endpoint)
    }

    fn disable(&mut self, services: &[String]) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::disable(self, services)
    }

    fn recover_pending(&mut self) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::recover_pending(self)
    }

    fn set_socks(
        &mut self,
        services: &[String],
        enabled: bool,
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::set_socks(self, services, enabled, endpoint)
    }

    fn set_auto_proxy(
        &mut self,
        services: &[String],
        url: Option<&str>,
    ) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::set_auto_proxy(self, services, url)
    }

    fn set_bypass(
        &mut self,
        services: &[String],
        domains: &[String],
    ) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::set_bypass(self, services, domains)
    }
}

fn validate_services(services: &[String]) -> Result<(), AppError> {
    if services.is_empty()
        || services
            .iter()
            .any(|service| service.trim().is_empty() || service.chars().any(char::is_control))
    {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "at least one valid network service is required",
        ));
    }
    Ok(())
}

fn write_recovery(path: &Path, state: &SystemProxyState) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| platform_error("recovery path has no parent"))?;
    fs::create_dir_all(parent).map_err(storage_error)?;
    let temporary = path.with_extension("tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(state).map_err(storage_error)?,
    )
    .map_err(storage_error)?;
    fs::rename(temporary, path).map_err(storage_error)
}

fn read_recovery(path: &Path) -> Result<SystemProxyState, AppError> {
    if !path.is_file() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "no pending system proxy recovery record",
        ));
    }
    let mut state: SystemProxyState =
        serde_json::from_slice(&fs::read(path).map_err(storage_error)?).map_err(storage_error)?;
    // The old parser retained networksetup's surrounding quotes. Match the new
    // getter's representation before writing the URL and comparing the result.
    for service in &mut state.services {
        service.auto_proxy.url = service.auto_proxy.url.as_deref().and_then(|url| {
            let url = url
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or(url);
            (!url.is_empty() && url != "(null)").then(|| url.to_owned())
        });
    }
    Ok(state)
}

fn restored_protocol_matches(expected: &ProxyProtocolState, actual: &ProxyProtocolState) -> bool {
    // An empty or legacy disabled endpoint only restores the switch. networksetup retains
    // the last saved address, so exact endpoint equality would reject a valid restore.
    expected.enabled == actual.enabled
        && ((!expected.enabled
            && (expected.endpoint.host.is_empty() || expected.endpoint.port == 0))
            || expected.endpoint == actual.endpoint)
}

fn legacy_disabled_endpoint(value: &ProxyProtocolState) -> bool {
    !value.enabled && (value.endpoint.host.is_empty() != (value.endpoint.port == 0))
}

fn legacy_protocol_output_matches(text: &str, expected: &ProxyProtocolState) -> bool {
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .map(str::trim)
    };
    field("Enabled:") == Some("No")
        && field("Server:") == Some(expected.endpoint.host.as_str())
        && field("Port:").and_then(|port| port.parse::<u16>().ok()) == Some(expected.endpoint.port)
        && !expected.endpoint.host.chars().any(char::is_control)
}

fn remove_recovery(path: &Path) -> Result<(), AppError> {
    fs::remove_file(path).map_err(storage_error)
}

fn platform_io_error(error: io::Error) -> AppError {
    platform_error(error)
}

fn platform_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::PlatformFailed, error.to_string())
}

fn storage_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::StorageFailed, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[derive(Default)]
    struct FakeRunner {
        outputs: VecDeque<Result<String, AppError>>,
        calls: Vec<Vec<String>>,
    }

    impl CommandRunner for FakeRunner {
        fn run(&mut self, program: &str, args: &[String]) -> Result<String, AppError> {
            let mut call = vec![program.to_owned()];
            call.extend_from_slice(args);
            self.calls.push(call);
            self.outputs
                .pop_front()
                .unwrap_or_else(|| Ok(String::new()))
        }
    }

    impl sysproxy::macos::CommandRunner for FakeRunner {
        fn run(&mut self, args: &[&str]) -> std::io::Result<std::process::Output> {
            use std::os::unix::process::ExitStatusExt as _;
            let result = CommandRunner::run(
                self,
                NETWORK_SETUP,
                &args.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
            );
            let (status, stdout, stderr) = match result {
                Ok(text) => (0, text.into_bytes(), vec![]),
                Err(error) => (1 << 8, vec![], error.message.into_bytes()),
            };
            Ok(std::process::Output {
                status: std::process::ExitStatus::from_raw(status),
                stdout,
                stderr,
            })
        }
    }

    struct TestDir(PathBuf);

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "verge-platform-{}-{nonce}-{}",
                std::process::id(),
                TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn single_instance_lock_is_exclusive_and_released_on_drop() {
        let directory = TestDir::new();
        let first = SingleInstance::acquire(&directory.0).unwrap();
        let error = SingleInstance::acquire(&directory.0).unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        drop(first);
        SingleInstance::acquire(&directory.0).unwrap();
    }

    #[test]
    fn helper_client_distinguishes_ready_and_not_installed() {
        use std::os::unix::net::UnixListener;

        let directory = TestDir::new();
        let socket = directory.0.join("helper.sock");
        assert_eq!(
            MacHelperClient::new(&socket).status(),
            HelperStatus::NotInstalled
        );
        let listener = UnixListener::bind(&socket).unwrap();
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let tun = verge_helper::shared_tun_backend(verge_helper::MacTun::default());
            verge_helper::serve_connection(stream, uid, &tun).unwrap();
        });
        assert_eq!(
            MacHelperClient::new(&socket).status(),
            HelperStatus::Ready {
                protocol_version: PROTOCOL_VERSION,
            }
        );
        server.join().unwrap();
    }

    #[test]
    fn helper_client_maps_tun_responses_and_errors() {
        use std::os::unix::net::UnixListener;

        struct FakeTun;

        impl verge_helper::TunBackend for FakeTun {
            fn tun_lifecycle_supported(&self) -> bool {
                true
            }

            fn enable_tun(
                &mut self,
                _config: &verge_helper::ValidatedTun,
            ) -> Result<String, verge_helper::HelperFailure> {
                Ok("utun4".into())
            }

            fn disable_tun(
                &mut self,
                _device: Option<&str>,
            ) -> Result<(), verge_helper::HelperFailure> {
                Err(verge_helper::HelperFailure {
                    code: "tun_not_managed",
                    message: "device is not managed by this helper".into(),
                })
            }
        }

        let directory = TestDir::new();
        let socket = directory.0.join("helper.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        let server = std::thread::spawn(move || {
            let tun = verge_helper::shared_tun_backend(FakeTun);
            for stream in listener.incoming().take(3) {
                verge_helper::serve_connection(stream.unwrap(), uid, &tun).unwrap();
            }
        });
        let client = MacHelperClient::new(&socket);

        let capabilities = client.capabilities().unwrap();
        assert_eq!(capabilities.protocol_version, PROTOCOL_VERSION);
        assert!(capabilities.tun_lifecycle);
        assert_eq!(
            client.enable_tun(&TunConfig::verge_default()).unwrap(),
            "utun4"
        );
        let error = client.disable_tun(Some("utun9")).unwrap_err();
        assert_eq!(error.code, ErrorCode::PlatformFailed);
        assert!(error.message.contains("not managed"));
        server.join().unwrap();
    }

    #[test]
    fn keychain_uses_argumentized_security_commands() {
        let mut runner = FakeRunner::default();
        runner.outputs.push_back(Ok("stored-secret\n".into()));
        runner.outputs.push_back(Ok(String::new()));
        runner.outputs.push_back(Ok(String::new()));
        let mut keychain = MacKeychain::new(runner, "com.zzzgydi.verge").unwrap();
        assert_eq!(keychain.get("openai").unwrap(), "stored-secret");
        keychain.set("openai", "new-secret").unwrap();
        keychain.delete("openai").unwrap();
        assert_eq!(keychain.runner.calls.len(), 3);
        assert!(
            keychain
                .runner
                .calls
                .iter()
                .all(|call| call.first().is_some_and(|program| program == SECURITY))
        );
        assert_eq!(
            MacKeychain::new(FakeRunner::default(), "bad\nservice")
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn notifications_pass_dynamic_text_as_argv() {
        let mut runner = FakeRunner::default();
        runner.outputs.push_back(Ok(String::new()));
        let mut notifier = MacNotifier::new(runner);
        notifier.notify("Verge", "Diagnostics exported").unwrap();
        let call = &notifier.runner.calls[0];
        assert_eq!(call[0], OSASCRIPT);
        assert_eq!(&call[4..], ["Verge", "Diagnostics exported"]);
        assert_eq!(
            notifier.notify("Verge", "bad\nbody").unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[derive(Default)]
    struct FakeLoginItem {
        enabled: bool,
        calls: Vec<&'static str>,
        failure: Option<AppError>,
    }

    impl LoginItemService for FakeLoginItem {
        fn status(&mut self) -> Result<bool, AppError> {
            self.calls.push("status");
            match &self.failure {
                Some(error) => Err(error.clone()),
                None => Ok(self.enabled),
            }
        }

        fn set_enabled(&mut self, enabled: bool) -> Result<(), AppError> {
            self.calls.push(if enabled { "enable" } else { "disable" });
            match self.failure.take() {
                Some(error) => Err(error),
                None => {
                    self.enabled = enabled;
                    Ok(())
                }
            }
        }
    }

    #[test]
    fn login_item_service_fake_records_sequence_and_maps_errors() {
        let mut item = FakeLoginItem::default();
        assert!(!item.status().unwrap());
        item.set_enabled(true).unwrap();
        assert!(item.status().unwrap());
        item.set_enabled(false).unwrap();
        assert_eq!(item.calls, ["status", "enable", "status", "disable"]);

        item.failure = Some(AppError::new(
            ErrorCode::PlatformFailed,
            "not a bundled .app",
        ));
        let error = item.set_enabled(true).unwrap_err();
        assert_eq!(error.code, ErrorCode::PlatformFailed);
        assert!(error.message.contains("not a bundled .app"));
        assert!(!item.status().unwrap());
    }

    #[test]
    fn default_login_item_service_is_constructible() {
        let mut service = default_login_item_service();
        // 只验证对象可构造；真实系统交互留待人工冒烟。
        let _ = &mut service;
    }

    fn output(enabled: bool, host: &str, port: u16) -> String {
        format!(
            "Enabled: {}\nServer: {host}\nPort: {port}\nAuthenticated Proxy Enabled: 0\n",
            if enabled { "Yes" } else { "No" }
        )
    }

    fn snapshot_outputs(runner: &mut FakeRunner, services: usize) {
        for index in 0..services {
            runner
                .outputs
                .push_back(Ok(output(false, "old.local", 8080)));
            runner
                .outputs
                .push_back(Ok(output(true, "secure.local", 8443)));
            runner
                .outputs
                .push_back(Ok(output(false, "socks.local", 1080)));
            runner
                .outputs
                .push_back(Ok("URL: (null)\nEnabled: No\n".into()));
            runner.outputs.push_back(Ok(format!(
                "There aren't any bypass domains set on {}.\n",
                if index == 0 { "Wi-Fi" } else { "Ethernet" }
            )));
        }
    }

    #[test]
    fn disabled_unconfigured_proxy_is_readable_and_restores_without_endpoint() {
        let directory = TestDir::new();
        let mut runner = FakeRunner::default();
        runner
            .outputs
            .push_back(Ok("Enabled: No\nServer: \nPort: \n".into()));
        runner.outputs.push_back(Ok(output(true, "", 0)));
        let mut proxy = MacSystemProxy::new(runner, directory.0.join("recovery.json"));
        let state = proxy.get_protocol(ProxyType::Http, "Wi-Fi").unwrap();
        assert!(!state.enabled);
        assert_eq!(state.endpoint.port, 0);
        assert!(state.endpoint.host.is_empty());
        assert!(proxy.get_protocol(ProxyType::Http, "Wi-Fi").is_err());
        proxy.runner.calls.clear();
        proxy
            .set_protocol(ProxyType::Http, "Wi-Fi", &state)
            .unwrap();
        assert_eq!(
            proxy.runner.calls,
            vec![vec![NETWORK_SETUP, "-setwebproxystate", "Wi-Fi", "off"]]
        );
    }

    #[test]
    fn invalid_proxy_snapshot_prevents_mutation_and_recovery_record() {
        for (index, invalid) in [
            (0, "Enabled: No\nServer: host\nPort: 0\n"),
            (1, "Enabled: No\nServer: \nPort: 8080\n"),
            (2, "Enabled: No\nServer: bad\thost\nPort: 1080\n"),
            (3, "URL: http://bad\turl/proxy.pac\nEnabled: No\n"),
            (4, "localhost\nEmpty\n"),
        ] {
            let directory = TestDir::new();
            let recovery = directory.0.join("recovery.json");
            let mut runner = FakeRunner::default();
            snapshot_outputs(&mut runner, 1);
            runner.outputs[index] = Ok(invalid.into());
            let mut proxy = MacSystemProxy::new(runner, &recovery);
            let error = proxy
                .enable(
                    &["Wi-Fi".into()],
                    &ProxyEndpoint::new("127.0.0.1", 7897).unwrap(),
                )
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::PlatformFailed);
            assert!(!recovery.exists());
            assert!(
                proxy
                    .runner
                    .calls
                    .iter()
                    .all(|call| call[1].starts_with("-get"))
            );
        }
    }

    #[test]
    fn reads_multiple_services_without_shell_commands() {
        let directory = TestDir::new();
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 2);
        let mut proxy = MacSystemProxy::new(runner, directory.0.join("recovery.json"));
        let state = proxy.state(&["Wi-Fi".into(), "Ethernet".into()]).unwrap();
        assert_eq!(state.services.len(), 2);
        assert_eq!(proxy.runner.calls[0][0], NETWORK_SETUP);
        assert_eq!(proxy.runner.calls[0][2], "Wi-Fi");
        assert!(!state.recovery_pending);
    }

    #[test]
    fn discovers_enabled_network_services_and_skips_disabled_entries() {
        let directory = TestDir::new();
        let mut runner = FakeRunner::default();
        runner.outputs.push_back(Ok(
            "An asterisk (*) denotes that a network service is disabled.\nWi-Fi\n*Ethernet\nUSB 10/100/1000 LAN\n"
                .into(),
        ));
        let mut proxy = MacSystemProxy::new(runner, directory.0.join("recovery.json"));
        assert_eq!(
            proxy.list_network_services().unwrap(),
            ["Wi-Fi", "USB 10/100/1000 LAN"]
        );
        assert_eq!(
            proxy.runner.calls[0],
            [NETWORK_SETUP, "-listallnetworkservices"]
        );
    }

    #[test]
    fn network_port_remap_preserves_unrelated_protocols_and_recovery_record() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        fs::write(&recovery, "original-recovery-record").unwrap();
        let mut runner = FakeRunner::default();
        for _ in 0..2 {
            runner.outputs.extend([
                Ok(output(true, "127.0.0.1", 7890)),
                Ok(output(true, "unrelated.local", 8443)),
                Ok(output(false, "127.0.0.1", 7890)),
                Ok("URL: (null)\nEnabled: No\n".into()),
                Ok("There aren't any bypass domains set on Wi-Fi.\n".into()),
            ]);
        }
        runner
            .outputs
            .extend([Ok(String::new()), Ok(String::new())]);
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let old = ProxyEndpoint::new("127.0.0.1", 7890).unwrap();
        let new = ProxyEndpoint::new("127.0.0.1", 9000).unwrap();
        proxy
            .remap_endpoints(&["Wi-Fi".into()], &old, Some(&old), &new)
            .unwrap();
        let writes = proxy
            .runner
            .calls
            .iter()
            .filter(|call| call.get(1).is_some_and(|arg| arg.starts_with("-set")))
            .collect::<Vec<_>>();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0][1], "-setwebproxy");
        assert_eq!(writes[0].last().unwrap(), "9000");
        assert_eq!(
            fs::read_to_string(recovery).unwrap(),
            "original-recovery-record"
        );
    }

    #[test]
    fn enable_persists_snapshot_before_mutation_and_recovers_it() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        for _ in 0..5 {
            runner.outputs.push_back(Ok(String::new()));
        }
        snapshot_outputs(&mut runner, 1);
        for _ in 0..9 {
            runner.outputs.push_back(Ok(String::new()));
        }
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let services = ["Wi-Fi".into()];
        let enabled = proxy
            .enable(&services, &ProxyEndpoint::new("127.0.0.1", 7890).unwrap())
            .unwrap();
        assert!(enabled.recovery_pending);
        assert!(recovery.is_file());
        assert_eq!(
            proxy.runner.calls[5],
            [NETWORK_SETUP, "-setautoproxystate", "Wi-Fi", "off"]
        );
        assert_eq!(proxy.runner.calls[6][1], "-setwebproxy");
        let restored = proxy.recover_pending().unwrap();
        assert!(!restored.recovery_pending);
        assert!(!recovery.exists());
        assert!(proxy.runner.calls.windows(2).any(|calls| {
            calls[0] == [NETWORK_SETUP, "-setautoproxyurl", "Wi-Fi", "\"\""]
                && calls[1] == [NETWORK_SETUP, "-setautoproxystate", "Wi-Fi", "off"]
        }));
        assert!(
            proxy
                .runner
                .calls
                .iter()
                .any(|call| call.get(1).is_some_and(|arg| arg == "-setwebproxy"))
        );
    }

    #[test]
    fn partial_failure_rolls_back_and_removes_recovery_record() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        runner.outputs.push_back(Ok(String::new()));
        runner.outputs.push_back(Ok(String::new()));
        runner
            .outputs
            .push_back(Err(platform_error("secure proxy failed")));
        runner.outputs.extend((0..9).map(|_| Ok(String::new())));
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let error = proxy
            .enable(
                &["Wi-Fi".into()],
                &ProxyEndpoint::new("127.0.0.1", 7890).unwrap(),
            )
            .unwrap_err();
        assert!(error.message.contains("secure proxy failed"));
        assert!(!recovery.exists());
        assert_eq!(proxy.runner.calls.len(), 22);
    }

    #[test]
    fn disable_external_proxy_without_recovery_clears_all_protocols() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        runner.outputs.extend((0..4).map(|_| Ok(String::new())));
        snapshot_outputs(&mut runner, 1);
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        proxy.disable(&["Wi-Fi".into()]).unwrap();
        assert!(!recovery.exists());
        let writes = proxy
            .runner
            .calls
            .iter()
            .filter(|call| call[1].starts_with("-set"))
            .collect::<Vec<_>>();
        assert_eq!(writes.len(), 4);
        assert_eq!(
            writes[0],
            &[NETWORK_SETUP, "-setwebproxystate", "Wi-Fi", "off"]
        );
        assert_eq!(
            writes[1],
            &[NETWORK_SETUP, "-setsecurewebproxystate", "Wi-Fi", "off"]
        );
    }

    #[test]
    fn failed_recovery_keeps_record_for_next_start() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let previous = SystemProxyState {
            services: vec![SystemProxyServiceState {
                service: "Wi-Fi".into(),
                web: ProxyProtocolState {
                    enabled: false,
                    endpoint: ProxyEndpoint::new("old.local", 8080).unwrap(),
                },
                secure_web: ProxyProtocolState {
                    enabled: false,
                    endpoint: ProxyEndpoint::new("old.local", 8080).unwrap(),
                },
                socks: ProxyProtocolState {
                    enabled: false,
                    endpoint: ProxyEndpoint::new("old.local", 1080).unwrap(),
                },
                auto_proxy: AutoProxyState {
                    enabled: false,
                    url: None,
                },
                bypass: Vec::new(),
            }],
            recovery_pending: false,
        };
        write_recovery(&recovery, &previous).unwrap();
        let mut runner = FakeRunner::default();
        runner
            .outputs
            .push_back(Err(platform_error("permission denied")));
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        assert!(proxy.recover_pending().is_err());
        assert!(recovery.is_file());
        assert_eq!(proxy.runner.calls.len(), 8);
    }

    fn recovery_fixture() -> SystemProxyState {
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        let directory = TestDir::new();
        MacSystemProxy::new(runner, directory.0.join("recovery.json"))
            .state(&["Wi-Fi".into()])
            .unwrap()
    }

    #[test]
    fn legacy_disabled_endpoints_recover_by_switching_off() {
        for protocol in 0..3 {
            for (host, port) in [("old.local", 0), ("", 8080)] {
                let directory = TestDir::new();
                let recovery = directory.0.join("recovery.json");
                let mut saved = recovery_fixture();
                let service = &mut saved.services[0];
                let value = match protocol {
                    0 => &mut service.web,
                    1 => &mut service.secure_web,
                    _ => &mut service.socks,
                };
                value.enabled = false;
                value.endpoint = ProxyEndpoint {
                    host: host.into(),
                    port,
                };
                write_recovery(&recovery, &saved).unwrap();
                let mut runner = FakeRunner::default();
                // The incomplete endpoint is skipped, but its switch is restored.
                runner.outputs.extend((0..8).map(|_| Ok(String::new())));
                snapshot_outputs(&mut runner, 1);
                runner.outputs[8 + protocol] = Ok(output(false, "127.0.0.1", 7897));
                let mut proxy = MacSystemProxy::new(runner, &recovery);
                let restored = proxy.recover_pending().unwrap();
                assert!(!restored.recovery_pending);
                assert!(!recovery.exists());
                let command = [
                    "-setwebproxy",
                    "-setsecurewebproxy",
                    "-setsocksfirewallproxy",
                ][protocol];
                assert!(!proxy.runner.calls.iter().any(|call| call[1] == command));
                assert!(
                    proxy
                        .runner
                        .calls
                        .iter()
                        .any(|call| { call[1] == format!("{command}state") && call[3] == "off" })
                );
            }
        }
    }

    #[test]
    fn legacy_recovery_accepts_only_the_saved_disabled_endpoint() {
        for protocol in 0..3 {
            for (host, port) in [("old.local", 0), ("", 8080)] {
                for (valid, returned) in [
                    (true, Ok(output(false, host, port))),
                    (false, Ok(output(true, host, port))),
                    (false, Ok(output(false, "different.local", 0))),
                    (false, Ok("Enabled: No\nServer: \nPort: invalid\n".into())),
                    (false, Ok("Server: old.local\nPort: 0\n".into())),
                    (
                        false,
                        Ok("Enabled: Maybe\nServer: old.local\nPort: 0\n".into()),
                    ),
                    (false, Err(platform_error("read failed"))),
                ] {
                    let directory = TestDir::new();
                    let recovery = directory.0.join("recovery.json");
                    let mut saved = recovery_fixture();
                    let service = &mut saved.services[0];
                    let value = match protocol {
                        0 => &mut service.web,
                        1 => &mut service.secure_web,
                        _ => &mut service.socks,
                    };
                    *value = ProxyProtocolState {
                        enabled: false,
                        endpoint: ProxyEndpoint {
                            host: host.into(),
                            port,
                        },
                    };
                    write_recovery(&recovery, &saved).unwrap();
                    let original = fs::read(&recovery).unwrap();
                    let mut runner = FakeRunner::default();
                    runner.outputs.extend((0..8).map(|_| Ok(String::new())));
                    snapshot_outputs(&mut runner, 1);
                    runner.outputs[8 + protocol] = returned;
                    let mut proxy = MacSystemProxy::new(runner, &recovery);
                    let result = proxy.recover_pending();
                    if valid {
                        assert_eq!(result.unwrap(), saved);
                        assert!(!recovery.exists());
                        // Compatibility is limited to recovery: new snapshots still reject it.
                        let mut runner = FakeRunner::default();
                        snapshot_outputs(&mut runner, 1);
                        runner.outputs[protocol] = Ok(output(false, host, port));
                        assert!(
                            MacSystemProxy::new(runner, &recovery)
                                .state(&["Wi-Fi".into()])
                                .is_err()
                        );
                    } else {
                        assert!(result.is_err());
                        assert_eq!(fs::read(&recovery).unwrap(), original);
                    }
                }
            }
        }
    }

    #[test]
    fn legacy_pac_quotes_are_normalized_before_restoring() {
        for (url, normalized) in [
            ("\"\"", None),
            (
                "\"https://old.local/proxy.pac\"",
                Some("https://old.local/proxy.pac"),
            ),
            (
                "https://old.local/proxy.pac",
                Some("https://old.local/proxy.pac"),
            ),
        ] {
            for enabled in [false, true] {
                if enabled && normalized.is_none() {
                    continue;
                }
                let directory = TestDir::new();
                let recovery = directory.0.join("recovery.json");
                let mut saved = recovery_fixture();
                saved.services[0].auto_proxy = AutoProxyState {
                    enabled,
                    url: Some(url.into()),
                };
                write_recovery(&recovery, &saved).unwrap();
                let mut runner = FakeRunner::default();
                runner.outputs.extend((0..9).map(|_| Ok(String::new())));
                snapshot_outputs(&mut runner, 1);
                runner.outputs[12] = Ok(format!(
                    "URL: {}\nEnabled: {}\n",
                    normalized.unwrap_or("(null)"),
                    if enabled { "Yes" } else { "No" }
                ));
                let mut proxy = MacSystemProxy::new(runner, &recovery);
                let state = proxy.recover_pending().unwrap();
                assert_eq!(
                    state.services[0].auto_proxy,
                    AutoProxyState {
                        enabled,
                        url: normalized.map(str::to_owned)
                    }
                );
                assert_eq!(proxy.runner.calls[6][3], normalized.unwrap_or("\"\""));
                assert!(!recovery.exists());
            }
        }
    }

    #[test]
    fn recovery_read_failure_retains_record_and_can_retry() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        write_recovery(&recovery, &recovery_fixture()).unwrap();
        let original = fs::read(&recovery).unwrap();
        let mut runner = FakeRunner::default();
        runner.outputs.extend((0..9).map(|_| Ok(String::new())));
        runner.outputs.push_back(Err(platform_error("read failed")));
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        assert!(
            proxy
                .recover_pending()
                .unwrap_err()
                .message
                .contains("read failed")
        );
        assert_eq!(fs::read(&recovery).unwrap(), original);
        proxy
            .runner
            .outputs
            .extend((0..9).map(|_| Ok(String::new())));
        snapshot_outputs(&mut proxy.runner, 1);
        let restored = proxy.recover_pending().unwrap();
        assert_eq!(restored, recovery_fixture());
        assert!(!recovery.exists());
    }

    #[test]
    fn recovery_mismatch_retains_record_for_all_proxy_fields() {
        for (index, actual) in [
            (0, output(true, "old.local", 8080)),
            (0, output(false, "wrong.local", 8080)),
            (1, output(false, "secure.local", 8443)),
            (2, output(true, "socks.local", 1080)),
            (3, "URL: http://wrong.local/proxy.pac\nEnabled: No\n".into()),
            (
                3,
                "URL: http://wrong.local/proxy.pac\nEnabled: Yes\n".into(),
            ),
            (4, "wrong.local\n".into()),
        ] {
            let directory = TestDir::new();
            let recovery = directory.0.join("recovery.json");
            write_recovery(&recovery, &recovery_fixture()).unwrap();
            let original = fs::read(&recovery).unwrap();
            let mut runner = FakeRunner::default();
            runner.outputs.extend((0..9).map(|_| Ok(String::new())));
            snapshot_outputs(&mut runner, 1);
            runner.outputs[9 + index] = Ok(actual);
            let mut proxy = MacSystemProxy::new(runner, &recovery);
            let error = proxy.disable(&["Wi-Fi".into()]).unwrap_err();
            assert!(error.message.contains("did not match"));
            assert_eq!(fs::read(&recovery).unwrap(), original);
        }
    }

    #[test]
    fn recovery_of_empty_disabled_endpoint_allows_saved_values() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut saved = recovery_fixture();
        saved.services[0].web.endpoint = ProxyEndpoint {
            host: String::new(),
            port: 0,
        };
        write_recovery(&recovery, &saved).unwrap();
        let mut runner = FakeRunner::default();
        runner.outputs.extend((0..8).map(|_| Ok(String::new())));
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let restored = proxy.recover_pending().unwrap();
        assert_eq!(restored.services[0].web.endpoint.host, "old.local");
        assert!(!recovery.exists());
    }

    #[test]
    fn legacy_recovery_does_not_accept_invalid_enabled_endpoints() {
        for (host, port) in [("old.local", 0), ("", 8080)] {
            let directory = TestDir::new();
            let recovery = directory.0.join("recovery.json");
            let mut saved = recovery_fixture();
            saved.services[0].web = ProxyProtocolState {
                enabled: true,
                endpoint: ProxyEndpoint {
                    host: host.into(),
                    port,
                },
            };
            write_recovery(&recovery, &saved).unwrap();
            let mut proxy = MacSystemProxy::new(FakeRunner::default(), &recovery);
            assert!(proxy.recover_pending().is_err());
            assert!(recovery.exists());
            assert!(
                !proxy
                    .runner
                    .calls
                    .iter()
                    .any(|call| { call[1] == "-setwebproxy" || call[1] == "-setwebproxystate" })
            );
        }
    }

    #[test]
    fn rollback_mismatch_retains_record() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        runner
            .outputs
            .push_back(Err(platform_error("apply failed")));
        runner.outputs.extend((0..9).map(|_| Ok(String::new())));
        snapshot_outputs(&mut runner, 1);
        runner.outputs[15] = Ok(output(true, "127.0.0.1", 7897));
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let error = proxy
            .enable(
                &["Wi-Fi".into()],
                &ProxyEndpoint::new("127.0.0.1", 7897).unwrap(),
            )
            .unwrap_err();
        assert!(error.message.contains("rollback failed"));
        assert!(recovery.exists());
    }

    #[test]
    fn snapshot_reads_socks_pac_and_bypass() {
        let directory = TestDir::new();
        let mut runner = FakeRunner::default();
        runner
            .outputs
            .push_back(Ok(output(false, "old.local", 8080)));
        runner
            .outputs
            .push_back(Ok(output(true, "secure.local", 8443)));
        runner
            .outputs
            .push_back(Ok(output(true, "socks.local", 1080)));
        runner
            .outputs
            .push_back(Ok("URL: http://127.0.0.1/proxy.pac\nEnabled: Yes\n".into()));
        runner
            .outputs
            .push_back(Ok("*.local\n192.168.0.0/16\n".into()));
        let mut proxy = MacSystemProxy::new(runner, directory.0.join("recovery.json"));
        let state = proxy.state(&["Wi-Fi".into()]).unwrap();
        let service = &state.services[0];
        assert_eq!(
            service.socks.endpoint,
            ProxyEndpoint::new("socks.local", 1080).unwrap()
        );
        assert!(service.socks.enabled);
        assert_eq!(
            service.auto_proxy.url.as_deref(),
            Some("http://127.0.0.1/proxy.pac")
        );
        assert!(service.auto_proxy.enabled);
        assert_eq!(service.bypass, ["*.local", "192.168.0.0/16"]);
        let calls = &proxy.runner.calls;
        assert_eq!(calls[2][1], "-getsocksfirewallproxy");
        assert_eq!(calls[3][1], "-getautoproxyurl");
        assert_eq!(calls[4][1], "-getproxybypassdomains");
    }

    fn unified_outputs(runner: &mut FakeRunner, pac: bool) {
        for _ in 0..3 {
            runner
                .outputs
                .push_back(Ok(output(!pac, "127.0.0.1", 7897)));
        }
        runner.outputs.push_back(Ok(if pac {
            "URL: http://127.0.0.1:1234/proxy.pac\nEnabled: Yes\n"
        } else {
            "URL: (null)\nEnabled: No\n"
        }
        .into()));
        runner.outputs.push_back(Ok("localhost\n*.local\n".into()));
    }

    #[test]
    fn unified_proxy_switches_modes_and_preserves_original_recovery() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        runner.outputs.extend((0..8).map(|_| Ok(String::new())));
        unified_outputs(&mut runner, false);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let services = ["Wi-Fi".into()];
        let manual = SystemProxyTarget::Manual {
            endpoint: ProxyEndpoint::new("127.0.0.1", 7897).unwrap(),
            bypass: vec!["localhost".into(), "*.local".into()],
        };
        assert!(manual.matches(&proxy.configure(&services, &manual).unwrap()));
        let original = fs::read(&recovery).unwrap();
        let writes: Vec<_> = proxy
            .runner
            .calls
            .iter()
            .filter(|c| c[1].starts_with("-set"))
            .collect();
        assert_eq!(writes.len(), 8);
        assert_eq!(writes[0][1], "-setautoproxystate");
        for (index, command) in [
            (1, "-setwebproxy"),
            (3, "-setsecurewebproxy"),
            (5, "-setsocksfirewallproxy"),
        ] {
            assert_eq!(writes[index][1], command);
            assert_eq!(&writes[index][3..], &["127.0.0.1", "7897"]);
        }
        unified_outputs(&mut proxy.runner, false);
        proxy
            .runner
            .outputs
            .extend((0..5).map(|_| Ok(String::new())));
        unified_outputs(&mut proxy.runner, true);
        let pac = SystemProxyTarget::Pac {
            url: "http://127.0.0.1:1234/proxy.pac".into(),
        };
        assert!(pac.matches(&proxy.configure(&services, &pac).unwrap()));
        assert_eq!(fs::read(&recovery).unwrap(), original);
        let writes: Vec<_> = proxy
            .runner
            .calls
            .iter()
            .filter(|c| c[1].starts_with("-set"))
            .collect();
        assert_eq!(writes[8][1], "-setwebproxystate");
        assert_eq!(writes[10][1], "-setsocksfirewallproxystate");
        assert_eq!(writes[12][1], "-setautoproxystate");
    }

    #[test]
    fn unified_proxy_rolls_back_when_successful_commands_do_not_change_state() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        runner.outputs.extend((0..8).map(|_| Ok(String::new())));
        snapshot_outputs(&mut runner, 1);
        runner.outputs.extend((0..9).map(|_| Ok(String::new())));
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let error = proxy
            .configure(
                &["Wi-Fi".into()],
                &SystemProxyTarget::Manual {
                    endpoint: ProxyEndpoint::new("127.0.0.1", 7897).unwrap(),
                    bypass: vec![],
                },
            )
            .unwrap_err();
        assert!(error.message.contains("did not match"));
        assert!(!recovery.exists());
        assert!(
            proxy.runner.calls.len() > 18,
            "rollback must restore the snapshot"
        );
    }

    #[test]
    fn unified_failure_on_second_service_restores_every_protocol() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 2);
        runner.outputs.extend((0..8).map(|_| Ok(String::new())));
        runner
            .outputs
            .push_back(Err(platform_error("permission denied")));
        runner.outputs.extend((0..18).map(|_| Ok(String::new())));
        snapshot_outputs(&mut runner, 2);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let error = proxy
            .configure(
                &["Wi-Fi".into(), "Ethernet".into()],
                &SystemProxyTarget::Manual {
                    endpoint: ProxyEndpoint::new("127.0.0.1", 7897).unwrap(),
                    bypass: vec![],
                },
            )
            .unwrap_err();
        assert!(error.message.contains("permission denied"));
        assert!(!recovery.exists());
        for service in ["Wi-Fi", "Ethernet"] {
            for command in [
                "-setwebproxy",
                "-setsecurewebproxy",
                "-setsocksfirewallproxy",
                "-setautoproxystate",
                "-setproxybypassdomains",
            ] {
                assert!(
                    proxy
                        .runner
                        .calls
                        .iter()
                        .skip(19)
                        .any(|call| call[1] == command && call[2] == service)
                );
            }
        }
    }

    #[test]
    fn set_socks_uses_argumentized_commands_and_recovers() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        for _ in 0..2 {
            runner.outputs.push_back(Ok(String::new()));
        }
        snapshot_outputs(&mut runner, 1);
        for _ in 0..9 {
            runner.outputs.push_back(Ok(String::new()));
        }
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let services = ["Wi-Fi".into()];
        let state = proxy
            .set_socks(
                &services,
                true,
                &ProxyEndpoint::new("127.0.0.1", 7891).unwrap(),
            )
            .unwrap();
        assert!(state.recovery_pending);
        let calls = &proxy.runner.calls;
        assert_eq!(
            calls[5],
            [
                NETWORK_SETUP,
                "-setsocksfirewallproxy",
                "Wi-Fi",
                "127.0.0.1",
                "7891"
            ]
        );
        assert_eq!(
            calls[6],
            [NETWORK_SETUP, "-setsocksfirewallproxystate", "Wi-Fi", "on"]
        );
        proxy.recover_pending().unwrap();
        assert!(!recovery.exists());
        assert!(proxy.runner.calls.iter().any(|call| {
            call.get(1)
                .is_some_and(|arg| arg == "-setsocksfirewallproxy")
        }));
    }

    #[test]
    fn set_socks_partial_failure_rolls_back_and_removes_recovery_record() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        runner
            .outputs
            .push_back(Err(platform_error("socks proxy failed")));
        runner.outputs.extend((0..9).map(|_| Ok(String::new())));
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let error = proxy
            .set_socks(
                &["Wi-Fi".into()],
                true,
                &ProxyEndpoint::new("127.0.0.1", 7891).unwrap(),
            )
            .unwrap_err();
        assert!(error.message.contains("socks proxy failed"));
        assert!(!recovery.exists());
        // 快照 5 次读取 + 1 次失败写入 + 9 次回滚写入 + 5 次校验读取。
        assert_eq!(proxy.runner.calls.len(), 20);
    }

    #[test]
    fn set_auto_proxy_enables_with_url_and_disables_without() {
        let directory = TestDir::new();
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        for _ in 0..2 {
            runner.outputs.push_back(Ok(String::new()));
        }
        snapshot_outputs(&mut runner, 1);
        snapshot_outputs(&mut runner, 1);
        runner.outputs.push_back(Ok(String::new()));
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, directory.0.join("recovery.json"));
        let services = ["Wi-Fi".into()];
        proxy
            .set_auto_proxy(&services, Some("http://127.0.0.1/proxy.pac"))
            .unwrap();
        let calls = &proxy.runner.calls;
        assert_eq!(
            calls[5],
            [
                NETWORK_SETUP,
                "-setautoproxyurl",
                "Wi-Fi",
                "http://127.0.0.1/proxy.pac"
            ]
        );
        assert_eq!(
            calls[6],
            [NETWORK_SETUP, "-setautoproxystate", "Wi-Fi", "on"]
        );
        proxy.set_auto_proxy(&services, None).unwrap();
        let calls = &proxy.runner.calls;
        assert_eq!(
            calls[17],
            [NETWORK_SETUP, "-setautoproxystate", "Wi-Fi", "off"]
        );
        assert_eq!(
            proxy
                .set_auto_proxy(&services, Some("http://bad\nurl"))
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn set_bypass_passes_domains_and_uses_empty_marker() {
        let directory = TestDir::new();
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        runner.outputs.push_back(Ok(String::new()));
        snapshot_outputs(&mut runner, 1);
        snapshot_outputs(&mut runner, 1);
        runner.outputs.push_back(Ok(String::new()));
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, directory.0.join("recovery.json"));
        let services = ["Wi-Fi".into()];
        proxy
            .set_bypass(&services, &["*.local".into(), "192.168.0.0/16".into()])
            .unwrap();
        assert_eq!(
            proxy.runner.calls[5],
            [
                NETWORK_SETUP,
                "-setproxybypassdomains",
                "Wi-Fi",
                "*.local",
                "192.168.0.0/16"
            ]
        );
        proxy.set_bypass(&services, &[]).unwrap();
        assert_eq!(
            proxy.runner.calls[16],
            [NETWORK_SETUP, "-setproxybypassdomains", "Wi-Fi", "Empty"]
        );
        assert_eq!(
            proxy
                .set_bypass(&services, &["bad\ndomain".into()])
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn later_mutations_do_not_clobber_the_original_recovery_record() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        for _ in 0..5 {
            runner.outputs.push_back(Ok(String::new()));
        }
        snapshot_outputs(&mut runner, 1);
        snapshot_outputs(&mut runner, 1);
        runner.outputs.push_back(Ok(String::new()));
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let services = ["Wi-Fi".into()];
        proxy
            .enable(&services, &ProxyEndpoint::new("127.0.0.1", 7890).unwrap())
            .unwrap();
        let original = fs::read(&recovery).unwrap();
        proxy.set_bypass(&services, &["*.local".into()]).unwrap();
        assert_eq!(fs::read(&recovery).unwrap(), original);
    }
}
